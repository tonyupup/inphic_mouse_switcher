//! 前台程序切换方案: 监听前台窗口变化, 在后台线程里把对应方案的命令帧发给鼠标。
//!
//! 不安装任何键盘/鼠标钩子, 也不常驻 HUB 的 Hook.dll。
//! 实测(用户确认): 用本程序后切回游戏时键盘不再短暂失灵; HUB 的 Hook.dll 装有全局低级键盘钩子,
//! 但它的 HID 写入跑在独立工作线程上, 具体机制未查清(见项目 Gitea wiki 的 Home 页)。
//!
//! 可靠性: 写失败就丢掉句柄、每 3 秒重试; 鼠标拔出再插回会重新应用当前方案。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Result};
use log::Level;

use crate::config::AppConfig;
use crate::profile::describe_status_change;
use crate::protocol::hex_spaced;
use crate::protocol::status::{battery_of, StatusReport};
use crate::runtime::channel::Channel;
#[cfg(windows)]
use crate::runtime::channel::{HidBackend, HidChannel};
use crate::runtime::monitor::{BatteryTracker, StatusMonitor};

const LOG_TARGET: &str = "inphic.switcher";

pub type Foreground = Box<dyn Fn() -> Option<String> + Send + Sync>;

/// 切换器的可变状态。
#[derive(Debug)]
pub struct State {
    /// 当前在用的连接方式: dongle / bluetooth / None
    pub kind: Option<&'static str>,
    /// 按前台程序应该用的方案
    pub desired: Option<String>,
    /// 已经成功写进鼠标的方案
    pub applied: Option<String>,
    pub applying: bool,
    /// 失败后最早什么时候重试
    pub retry_at: Option<Instant>,
    pub was_present: bool,
    pub next_presence_check: Option<Instant>,
}

/// 切换器和各通道的回调共享的部分: 回调(状态上报、断线)只需要它。
struct Shared {
    /// 当前生效的配置; 热重载时整体替换
    config: Mutex<Arc<AppConfig>>,
    state: Mutex<State>,
    battery: Mutex<BatteryTracker>,
}

struct Inner {
    shared: Arc<Shared>,
    foreground: Foreground,
    /// 检测顺序 = 优先级。蓝牙在前: 鼠标走蓝牙时 2.4G 接收器仍插在电脑上(它的 HID 设备一直在),
    /// 只有蓝牙连接状态才能说明鼠标此刻在用哪条链路; 鼠标一次只用一条。
    channels: Vec<(&'static str, Arc<dyn Channel>)>,
    monitor: Option<StatusMonitor>,
}

/// 交给通道/监听线程的回调工厂。
pub struct Callbacks {
    shared: Arc<Shared>,
}

impl Callbacks {
    pub fn on_status(&self) -> Arc<dyn Fn(StatusReport) + Send + Sync> {
        let shared = self.shared.clone();
        Arc::new(move |r| shared.on_status(r))
    }

    pub fn on_raw(&self) -> Arc<dyn Fn(Vec<u8>) + Send + Sync> {
        Arc::new(|d| log_raw(&d))
    }

    /// 设备断开/重连后调用: 下一个电量读数当作第一个读数。
    pub fn on_disconnect(&self) -> Arc<dyn Fn() + Send + Sync> {
        let shared = self.shared.clone();
        Arc::new(move || shared.battery.lock().unwrap().reset())
    }
}

/// 构造切换器所需的通道和状态监听线程。
pub struct Parts {
    /// 顺序就是检测优先级
    pub channels: Vec<(&'static str, Arc<dyn Channel>)>,
    pub monitor: Option<StatusMonitor>,
}

fn log_raw(data: &[u8]) {
    log::info!(target: LOG_TARGET, "收到无法解析为状态报告的数据: {}", hex_spaced(data));
}

impl Shared {
    fn config(&self) -> Arc<AppConfig> {
        self.config.lock().unwrap().clone()
    }

    /// 鼠标主动上报的每一条状态都记录: 电量变化/低电量写 INFO/WARNING, 其他类型一律 INFO, 每条原始字节写 DEBUG。
    fn on_status(&self, report: StatusReport) {
        let raw = if report.raw.is_empty() { "?".to_string() } else { hex_spaced(&report.raw) };
        log::debug!(target: LOG_TARGET, "状态报告 原始 {raw}");
        let Some(bat) = battery_of(Some(&report)) else {
            let name = {
                let s = self.state.lock().unwrap();
                s.applied.clone().or_else(|| s.desired.clone())
            };
            let config = self.config();
            let current = name.and_then(|n| config.profiles.get(&n));
            match describe_status_change(&report, current.map(|c| &c.profile)) {
                Some(text) => log::info!(target: LOG_TARGET, "{text}（原始 {raw}）"),
                None => log::info!(
                    target: LOG_TARGET,
                    "未识别的状态报告: 类型 {:#04x} a={:#04x} b={:#04x}（原始 {raw}）",
                    report.kind,
                    report.a,
                    report.b
                ),
            }
            return;
        };
        for (level, text) in self.battery.lock().unwrap().update(bat) {
            log::log!(target: LOG_TARGET, if level == Level::Warn { Level::Warn } else { Level::Info }, "{text}");
        }
    }
}

#[derive(Clone)]
pub struct ProfileSwitcher {
    inner: Arc<Inner>,
}

impl ProfileSwitcher {
    /// 应用失败后多久重试
    pub const RETRY_DELAY: Duration = Duration::from_secs(3);
    /// 多久检查一次设备是否还在
    pub const PRESENCE_CHECK: Duration = Duration::from_secs(2);
    /// 配置文件最后一次变动通知后等多久再重新加载
    pub const RELOAD_SETTLE: Duration = Duration::from_millis(300);

    /// 按配置建真实通道(只支持 Windows)。
    #[cfg(windows)]
    pub fn new(config: Arc<AppConfig>) -> Result<Self> {
        use crate::config::Connection;
        use crate::runtime::ble::{BleChannel, BleOptions};
        use crate::runtime::channel::{open_collection, HidApiBackend, Match, CONFIG_INTERFACE, STATUS_USAGE_PAGE};

        let backend: Arc<dyn HidBackend> = Arc::new(HidApiBackend::new()?);
        let foreground = Box::new(crate::runtime::foreground::get_foreground_process_name);
        let cfg = config.clone();
        Self::build(config, foreground, move |cb| {
            let dev = &cfg.device;
            let mut channels: Vec<(&'static str, Arc<dyn Channel>)> = Vec::new();
            if let Some(bt) = dev.bluetooth.as_ref().filter(|_| dev.connection != Connection::Dongle) {
                let ble = BleChannel::new(
                    &bt.address,
                    BleOptions {
                        packet_delay: Duration::from_millis(bt.packet_delay_ms),
                        listener: Some(cb.on_status()),
                        on_raw: Some(cb.on_raw()),
                        on_disconnect: Some(cb.on_disconnect()),
                        ..Default::default()
                    },
                )?;
                channels.push(("bluetooth", Arc::new(ble)));
            }
            if dev.connection != Connection::Bluetooth {
                channels.push((
                    "dongle",
                    Arc::new(HidChannel::new(backend.clone(), dev.vendor_id, dev.product_id, dev.usage_page, dev.usage)),
                ));
            }
            let (vid, pid, open_backend) = (dev.vendor_id, dev.product_id, backend.clone());
            let monitor = StatusMonitor::new(
                Box::new(move || {
                    open_collection(
                        &*open_backend,
                        vid,
                        pid,
                        Match { usage_page: STATUS_USAGE_PAGE, usage: None, interface: Some(CONFIG_INTERFACE) },
                    )
                }),
                cb.on_status(),
                Some(cb.on_raw()),
                Some(cb.on_disconnect()),
            );
            Ok(Parts { channels, monitor: Some(monitor) })
        })
    }

    #[cfg(not(windows))]
    pub fn new(_config: Arc<AppConfig>) -> Result<Self> {
        Err(anyhow!("inphic 只支持 Windows(HID + WinRT 蓝牙)"))
    }

    /// 通用构造: `make_parts` 拿到回调工厂, 返回通道和监听线程(测试里可以塞假通道)。
    pub fn build(
        config: Arc<AppConfig>,
        foreground: Foreground,
        make_parts: impl FnOnce(&Callbacks) -> Result<Parts>,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            config: Mutex::new(config),
            state: Mutex::new(State {
                kind: None,
                desired: None,
                applied: None,
                applying: false,
                retry_at: None,
                was_present: true,
                next_presence_check: None,
            }),
            battery: Mutex::new(BatteryTracker::new()),
        });
        let parts = make_parts(&Callbacks { shared: shared.clone() })?;
        Ok(ProfileSwitcher {
            inner: Arc::new(Inner { shared, foreground, channels: parts.channels, monitor: parts.monitor }),
        })
    }

    pub fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.inner.shared.state.lock().unwrap())
    }

    pub fn set_desired(&self, name: Option<&str>) {
        self.with_state(|s| s.desired = name.map(String::from));
    }

    pub fn desired(&self) -> Option<String> {
        self.with_state(|s| s.desired.clone())
    }

    pub fn applied(&self) -> Option<String> {
        self.with_state(|s| s.applied.clone())
    }

    pub fn kind(&self) -> Option<&'static str> {
        self.with_state(|s| s.kind)
    }

    pub fn was_present(&self) -> bool {
        self.with_state(|s| s.was_present)
    }

    pub fn is_applying(&self) -> bool {
        self.with_state(|s| s.applying)
    }

    /// 当前鼠标在用的连接方式: 蓝牙连着就是蓝牙, 否则看接收器。
    fn detect(&self) -> Option<&'static str> {
        self.inner.channels.iter().find(|(_, ch)| ch.is_present()).map(|&(kind, _)| kind)
    }

    fn channel(&self, kind: &str) -> Option<&Arc<dyn Channel>> {
        self.inner.channels.iter().find(|(k, _)| *k == kind).map(|(_, ch)| ch)
    }

    pub fn close(&self) {
        if let Some(m) = &self.inner.monitor {
            m.stop();
        }
        for (_, ch) in &self.inner.channels {
            ch.close();
        }
    }

    fn start_apply(&self, name: &str) {
        if !self.inner.shared.config().profiles.contains_key(name) {
            log::warn!(target: LOG_TARGET, "配置里没有名为「{name}」的方案");
            self.with_state(|s| s.desired = None);
            return;
        }
        self.with_state(|s| s.applying = true);
        let this = self.clone();
        let name = name.to_string();
        std::thread::Builder::new()
            .name(format!("apply-{name}"))
            .spawn(move || {
                let result = this.apply_worker(&name);
                this.with_state(|s| {
                    if result.is_err() {
                        s.retry_at = Some(Instant::now() + Self::RETRY_DELAY);
                    }
                    s.applying = false;
                });
                if let Err(e) = result {
                    log::error!(
                        target: LOG_TARGET,
                        "应用方案「{name}」失败, {:.0} 秒后重试: {e:#}",
                        Self::RETRY_DELAY.as_secs_f64()
                    );
                }
            })
            .expect("启动应用线程失败");
    }

    fn apply_worker(&self, name: &str) -> Result<()> {
        let config = self.inner.shared.config(); // 整个应用过程用同一份配置, 中途热重载不影响这一次
        let profile = config.profiles.get(name).ok_or_else(|| anyhow!("配置里没有名为「{name}」的方案"))?;
        let kind = match self.with_state(|s| s.kind).or_else(|| self.detect()) {
            Some(k) => k,
            None => self.inner.channels.first().map(|&(k, _)| k).ok_or_else(|| anyhow!("没有可用的连接通道"))?,
        };
        let ch = self.channel(kind).ok_or_else(|| anyhow!("没有 {kind} 通道"))?;
        log::info!(target: LOG_TARGET, "切换到方案「{name}」（{}）", ch.label());
        if !ch.supports_polling_rate() {
            log::info!(target: LOG_TARGET, "{}下回报率设置无效, 已跳过回报率命令", ch.label());
        }
        let delay = (kind == "dongle").then(|| Duration::from_millis(config.packet_delay_ms));
        ch.send_profile(profile, delay)?;
        // 写的过程中配置被热重载了: 写进去的是旧内容, 不记为已应用, 下一轮按新配置重写
        if !Arc::ptr_eq(&config, &self.inner.shared.config()) {
            log::info!(target: LOG_TARGET, "方案「{name}」写入期间配置已重载, 稍后按新配置重新应用");
            return Ok(());
        }
        self.with_state(|s| s.applied = Some(name.to_string()));
        log::info!(target: LOG_TARGET, "方案「{name}」应用完成");
        Ok(())
    }

    pub fn maybe_apply(&self) {
        let desired = self.with_state(|s| {
            if s.applying || s.desired.is_none() || s.desired == s.applied {
                return None;
            }
            if s.retry_at.is_some_and(|t| Instant::now() < t) {
                return None;
            }
            s.desired.clone()
        });
        if let Some(name) = desired {
            self.start_apply(&name);
        }
    }

    /// 定期检查设备是否还在; 断开后再出现就清掉 applied, 触发重新应用。
    pub fn poll_presence(&self, now: Instant) {
        let due = self.with_state(|s| {
            if s.applying || s.next_presence_check.is_some_and(|t| now < t) {
                return false;
            }
            s.next_presence_check = Some(now + Self::PRESENCE_CHECK);
            true
        });
        if !due {
            return;
        }
        let kind = self.detect();
        let present = kind.is_some();
        let (was_present, old_kind) = self.with_state(|s| (s.was_present, s.kind));
        if present && !was_present {
            log::info!(target: LOG_TARGET, "检测到鼠标重新连接, 重新应用当前方案");
            self.with_state(|s| s.applied = None);
        } else if was_present && !present {
            log::warn!(target: LOG_TARGET, "鼠标已断开, 等待重新连接");
        }
        if let Some(kind) = kind.filter(|k| Some(*k) != old_kind) {
            let ch = self.channel(kind).expect("detect 返回的一定是已有通道");
            log::info!(target: LOG_TARGET, "连接方式: {}", ch.label());
            if old_kind.is_some() {
                self.with_state(|s| s.applied = None); // 换了连接方式(如接收器 → 蓝牙), 要重新应用
            }
            if old_kind == Some("bluetooth") {
                if let Some(ble) = self.channel("bluetooth") {
                    ble.release(); // 不再用蓝牙了, 放掉 FEE0
                }
            }
            self.with_state(|s| s.kind = Some(kind));
        }
        self.with_state(|s| s.was_present = present);
    }

    /// 监视 `watched` 涉及的目录(目录本身, 或文件所在目录), 并撤掉 `old` 里不再需要的; 返回现在监视的目录。
    fn watch_dirs(
        w: &mut impl notify::Watcher,
        watched: &[std::path::PathBuf],
        old: &[std::path::PathBuf],
    ) -> Vec<std::path::PathBuf> {
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        for p in watched {
            let dir = if p.is_dir() {
                p.clone()
            } else {
                p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf()
            };
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        for d in old.iter().filter(|d| !dirs.contains(d)) {
            let _ = w.unwatch(d);
        }
        dirs.retain(|d| {
            old.contains(d) || w.watch(d, notify::RecursiveMode::NonRecursive).map_err(|e| {
                log::warn!(target: LOG_TARGET, "无法监视 {}: {e}", d.display());
            }).is_ok()
        });
        dirs
    }

    fn try_reload(&self, path: &Path) {
        match crate::config::load_config(path) {
            Ok(new) => {
                log::info!(target: LOG_TARGET, "配置文件有改动, 已重新加载:\n{}", crate::config::describe_config(&new, path));
                self.reload(new);
            }
            Err(e) => {
                log::error!(target: LOG_TARGET, "重新加载配置失败, 继续使用旧配置: {e}");
            }
        }
    }

    pub fn on_status(&self, report: StatusReport) {
        self.inner.shared.on_status(report);
    }

    /// 收到但不是状态报告格式的数据(比如命令的确认应答): 原样记录。
    pub fn on_raw(&self, data: &[u8]) {
        log_raw(data);
    }

    /// 换上新配置并重新应用当前方案。设备/通道在启动时就建好了, 不随配置热更新(device 段变了要重启)。
    pub fn reload(&self, new: AppConfig) {
        let old = self.inner.shared.config();
        if format!("{:?}", old.device) != format!("{:?}", new.device) {
            log::warn!(target: LOG_TARGET, "device 段有改动, 需要重启程序才能生效");
        }
        *self.inner.shared.config.lock().unwrap() = Arc::new(new);
        self.with_state(|s| {
            s.applied = None;
            s.retry_at = None;
        });
    }

    /// 主循环: 监听前台窗口切换, 直到 `stop` 被置位。
    pub fn run(&self, stop: &AtomicBool) {
        self.run_with_reload(stop, None)
    }

    /// 同 `run`; 给了 `config_path` 就同时监视配置文件和宏文件, 改动后自动重新加载。
    /// 新配置有错就报错并继续用旧配置。
    pub fn run_with_reload(&self, stop: &AtomicBool, config_path: Option<&Path>) {
        // 用系统的目录变更通知(Windows 的 ReadDirectoryChangesW), 不轮询; 通知只用来"叫醒", 是否真改了再用指纹确认
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = None;
        let mut watched_dirs: Vec<std::path::PathBuf> = Vec::new();
        let mut last = Fingerprint::of(&self.inner.shared.config().watched);
        if config_path.is_some() {
            match notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
                let _ = tx.send(ev);
            }) {
                Ok(mut w) => {
                    watched_dirs = Self::watch_dirs(&mut w, &self.inner.shared.config().watched, &[]);
                    watcher = Some(w);
                }
                Err(e) => log::warn!(target: LOG_TARGET, "无法监视配置文件, 改配置后需要重启: {e}"),
            }
        }
        let mut dirty: Option<Instant> = None;
        let mut last_process: Option<Option<String>> = None;
        log::info!(target: LOG_TARGET, "开始监听前台窗口切换 (Ctrl+C 退出)");
        if self.inner.channels.iter().any(|(k, _)| *k == "dongle") {
            if let Some(m) = &self.inner.monitor {
                m.start();
            }
        }
        while !stop.load(Ordering::SeqCst) {
            while rx.try_recv().is_ok() {
                dirty = Some(Instant::now());
            }
            // 编辑器保存常分几步写: 最后一次通知后安静一会儿再读
            if let (Some(path), Some(w), Some(t)) = (config_path, watcher.as_mut(), dirty) {
                if t.elapsed() >= Self::RELOAD_SETTLE {
                    dirty = None;
                    if Fingerprint::of(&self.inner.shared.config().watched) != last {
                        self.try_reload(path);
                        let watched = self.inner.shared.config().watched.clone();
                        watched_dirs = Self::watch_dirs(w, &watched, &watched_dirs);
                        last = Fingerprint::of(&watched);
                        last_process = None; // 按新规则重新匹配前台进程
                    }
                }
            }
            let poll_interval = Duration::from_millis(self.inner.shared.config().poll_interval_ms);
            let process_name = (self.inner.foreground)();
            if last_process.as_ref() != Some(&process_name) {
                log::debug!(target: LOG_TARGET, "前台进程: {process_name:?}");
                let config = self.inner.shared.config();
                if let Some(profile) = config.resolve_profile_name(process_name.as_deref()) {
                    self.set_desired(Some(profile));
                }
                last_process = Some(process_name);
            }
            self.poll_presence(Instant::now());
            if self.was_present() {
                self.maybe_apply();
            }
            // 切成小段睡眠, Ctrl+C 能很快响应
            let deadline = Instant::now() + poll_interval;
            while Instant::now() < deadline && !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(20).min(poll_interval));
            }
        }
    }
}

/// 一组文件的修改时间和大小, 用来判断有没有改动。
#[derive(PartialEq)]
struct Fingerprint(Vec<Option<(SystemTime, u64)>>);

impl Fingerprint {
    fn of(paths: &[std::path::PathBuf]) -> Self {
        Fingerprint(
            paths
                .iter()
                .map(|p| std::fs::metadata(p).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len())))
                .collect(),
        )
    }
}
