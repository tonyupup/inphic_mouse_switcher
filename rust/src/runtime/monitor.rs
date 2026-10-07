//! 状态监听: 后台线程读鼠标主动上报的状态报告(约每 2 秒一个), 电量变化时写日志。
//!
//! - 单独打开状态集合(Col03, 只读), 和写配置的句柄(Col04)互不干扰。
//! - 设备拔出/休眠后自动重连, 重连后的第一个读数会重新写一条日志。
//! - 日志只在值变化时写, 不会每 2 秒一行: 启动后的第一个读数、电量百分比变化、状态码变化、
//!   降到 20% / 10% 以下时的低电量警告(充电回升后再降下来会再次警告)。

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Result;
use log::Level;

use crate::protocol::status::{parse_report, Battery, StatusReport};
use crate::runtime::channel::HidDevice;

const LOG_TARGET: &str = "inphic.monitor";

pub const LOW_BATTERY_LEVELS: [u8; 2] = [20, 10];

/// 判断每个电量读数要写哪些日志。纯逻辑, 返回 `[(日志级别, 文字), ...]`。
#[derive(Default)]
pub struct BatteryTracker {
    last: Option<Battery>,
    warned: BTreeSet<u8>,
}

impl BatteryTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 设备重连后调用: 下一个读数当作"第一个读数"处理。
    pub fn reset(&mut self) {
        self.last = None;
    }

    pub fn update(&mut self, bat: Battery) -> Vec<(Level, String)> {
        let mut out = Vec::new();
        match self.last {
            None => out.push((Level::Info, format!("电量 {}%（状态码 {}）", bat.percent, bat.state))),
            Some(last) => {
                if bat.percent != last.percent {
                    out.push((Level::Info, format!("电量 {}% → {}%", last.percent, bat.percent)));
                }
                if bat.state != last.state {
                    out.push((
                        Level::Info,
                        format!("电量状态码 {} → {}（含义未确认，可能是充电状态变化）", last.state, bat.state),
                    ));
                }
            }
        }
        for level in LOW_BATTERY_LEVELS {
            if bat.percent > level {
                self.warned.remove(&level); // 充上去了, 下次再降下来要再警告
            } else if self.warned.insert(level) {
                out.push((Level::Warn, format!("电量低: {}%（已不高于 {level}%），请及时充电", bat.percent)));
            }
        }
        self.last = Some(bat);
        out
    }
}

/// 可以被打断的等待(对应 Python 的 `threading.Event.wait`)。
#[derive(Default)]
pub struct StopFlag {
    stopped: Mutex<bool>,
    cond: Condvar,
}

impl StopFlag {
    pub fn set(&self) {
        *self.stopped.lock().unwrap() = true;
        self.cond.notify_all();
    }

    pub fn is_set(&self) -> bool {
        *self.stopped.lock().unwrap()
    }

    /// 等 `d` 或被 set; 返回是否已 set。
    pub fn wait(&self, d: Duration) -> bool {
        let g = self.stopped.lock().unwrap();
        let (g, _) = self.cond.wait_timeout_while(g, d, |s| !*s).unwrap();
        *g
    }
}

pub type OpenDevice = Box<dyn Fn() -> Result<Box<dyn HidDevice>> + Send + Sync>;
pub type ReportHandler = Arc<dyn Fn(StatusReport) + Send + Sync>;
pub type RawHandler = Arc<dyn Fn(Vec<u8>) + Send + Sync>;
pub type Callback = Arc<dyn Fn() + Send + Sync>;

/// 后台线程: 打开状态集合, 把解析出的 StatusReport 交给 `on_report`。
pub struct StatusMonitor {
    open: Arc<OpenDevice>,
    on_report: ReportHandler,
    on_raw: Option<RawHandler>,
    on_disconnect: Option<Callback>,
    retry: Duration,
    poll_ms: i32,
    stop: Arc<StopFlag>,
    running: AtomicBool,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl StatusMonitor {
    pub fn new(
        open_device: OpenDevice,
        on_report: ReportHandler,
        on_raw: Option<RawHandler>,
        on_disconnect: Option<Callback>,
    ) -> Self {
        StatusMonitor {
            open: Arc::new(open_device),
            on_report,
            on_raw,
            on_disconnect,
            retry: Duration::from_secs(3),
            poll_ms: 500,
            stop: Arc::new(StopFlag::default()),
            running: AtomicBool::new(false),
            thread: Mutex::new(None),
        }
    }

    pub fn with_timing(mut self, retry: Duration, poll_ms: i32) -> Self {
        self.retry = retry;
        self.poll_ms = poll_ms;
        self
    }

    pub fn start(&self) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let ctx = Worker {
            open: self.open.clone(),
            on_report: self.on_report.clone(),
            on_raw: self.on_raw.clone(),
            on_disconnect: self.on_disconnect.clone(),
            retry: self.retry,
            poll_ms: self.poll_ms,
            stop: self.stop.clone(),
        };
        let handle = std::thread::Builder::new().name("status-monitor".into()).spawn(move || ctx.run()).unwrap();
        *self.thread.lock().unwrap() = Some(handle);
    }

    pub fn stop(&self) {
        self.stop.set();
        if let Some(h) = self.thread.lock().unwrap().take() {
            let _ = h.join();
        }
    }
}

struct Worker {
    open: Arc<OpenDevice>,
    on_report: ReportHandler,
    on_raw: Option<RawHandler>,
    on_disconnect: Option<Callback>,
    retry: Duration,
    poll_ms: i32,
    stop: Arc<StopFlag>,
}

impl Worker {
    fn run(&self) {
        let mut waiting_logged = false;
        while !self.stop.is_set() {
            let mut dev = match (self.open)() {
                Ok(d) => d,
                Err(_) => {
                    if !waiting_logged {
                        log::warn!(target: LOG_TARGET, "状态监听: 找不到鼠标的状态通道, 每 {:.0} 秒重试", self.retry.as_secs_f64());
                        waiting_logged = true;
                    }
                    self.stop.wait(self.retry);
                    continue;
                }
            };
            waiting_logged = false;
            log::debug!(target: LOG_TARGET, "状态监听: 已连接");
            let result = (|| -> Result<()> {
                while !self.stop.is_set() {
                    let data = dev.read(self.poll_ms)?;
                    if data.is_empty() {
                        continue;
                    }
                    match parse_report(&data) {
                        Some(rep) => (self.on_report)(rep),
                        None => {
                            if let Some(raw) = &self.on_raw {
                                raw(data); // 不是状态报告的数据也不丢: 交给调用方记录
                            }
                        }
                    }
                }
                Ok(())
            })();
            if result.is_err() {
                log::warn!(target: LOG_TARGET, "状态监听: 读取失败(鼠标断开或休眠?), 等待重连");
            }
            drop(dev);
            if let Some(cb) = &self.on_disconnect {
                cb();
            }
            self.stop.wait(self.retry);
        }
    }
}
