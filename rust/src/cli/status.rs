//! status 子命令: 读鼠标主动上报的状态报告(电量等)。只读, 不向鼠标写任何东西。
//!
//! 2.4G 接收器: 读 HID 状态集合, 可以和 run 同时运行。默认(`--via auto`)先看蓝牙是否连着, 连着就读蓝牙。
//! 蓝牙: 订阅 FEE4 通知(只开通知开关, 不写配置); 同一时间只能有一个程序打开 FEE0, 所以不能和 HUB / run 同时用。

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::cli::{Fatal, Via};
use crate::config::{read_device_section, DeviceConfig};
use crate::protocol::hex_spaced;
use crate::protocol::status::{battery_of, parse_report, StatusReport, KIND_BATTERY};
use crate::runtime::ble::{BleChannel, BleOptions};
use crate::runtime::channel::{open_collection, Channel, HidBackend, Match, CONFIG_INTERFACE, STATUS_USAGE_PAGE};

enum Item {
    Report(StatusReport),
    /// 不是状态报告格式的数据, 原样打印
    Raw(Vec<u8>),
    /// 这一轮没有新数据
    Idle,
}

trait ReportStream {
    /// 最多阻塞约 500 ms。
    fn next(&mut self) -> Result<Item>;
}

struct HidStream {
    dev: Box<dyn crate::runtime::channel::HidDevice>,
}

impl ReportStream for HidStream {
    fn next(&mut self) -> Result<Item> {
        let data = self.dev.read(500)?;
        if data.is_empty() {
            return Ok(Item::Idle);
        }
        Ok(match parse_report(&data) {
            Some(r) => Item::Report(r),
            None => Item::Raw(data), // 解析不了的也交出去, 不丢
        })
    }
}

struct BleStream {
    rx: Receiver<Item>,
    channel: BleChannel,
}

impl ReportStream for BleStream {
    fn next(&mut self) -> Result<Item> {
        match self.rx.recv_timeout(Duration::from_millis(500)) {
            Ok(item) => Ok(item),
            Err(RecvTimeoutError::Timeout) => Ok(Item::Idle),
            Err(RecvTimeoutError::Disconnected) => Ok(Item::Idle),
        }
    }
}

impl Drop for BleStream {
    fn drop(&mut self) {
        self.channel.close();
    }
}

#[cfg(windows)]
fn hid_backend() -> Result<Arc<dyn HidBackend>> {
    Ok(Arc::new(crate::runtime::channel::HidApiBackend::new()?))
}

#[cfg(not(windows))]
fn hid_backend() -> Result<Arc<dyn HidBackend>> {
    Err(anyhow::anyhow!("inphic 只支持 Windows(HID + WinRT 蓝牙)"))
}

fn print_report(item: &Item, stamp: &str) {
    match item {
        Item::Raw(data) => println!("{stamp}  其他数据 {}", hex_spaced(data)),
        Item::Report(rep) => match battery_of(Some(rep)).filter(|_| rep.kind == KIND_BATTERY) {
            Some(bat) => println!("{stamp}  电量 {bat}  原始 {}", hex_spaced(&rep.raw)),
            None => println!(
                "{stamp}  {} a={:#04x} b={:#04x} (数值 {})  原始 {}",
                rep.name(),
                rep.a,
                rep.b,
                rep.value(),
                hex_spaced(&rep.raw)
            ),
        },
        Item::Idle => {}
    }
}

/// 2.4G 接收器的状态流。
fn reports_hid(dev: &DeviceConfig) -> Result<Box<dyn ReportStream>> {
    let backend = hid_backend()?;
    let m = Match { usage_page: STATUS_USAGE_PAGE, usage: None, interface: Some(CONFIG_INTERFACE) };
    let dev = open_collection(&*backend, dev.vendor_id, dev.product_id, m).map_err(|e| Fatal(e.to_string()))?;
    Ok(Box::new(HidStream { dev }))
}

/// 蓝牙 FEE4 通知的状态流; 没有新通知时产出 Idle。
fn reports_ble(dev: &DeviceConfig) -> Result<Box<dyn ReportStream>> {
    let Some(bt) = &dev.bluetooth else {
        return Err(Fatal("config.yaml 的 device 里没有 bluetooth.address".into()).into());
    };
    let (tx, rx) = mpsc::channel::<Item>();
    let tx_raw = tx.clone();
    let channel = BleChannel::new(
        &bt.address,
        BleOptions {
            listener: Some(Arc::new(move |r| {
                let _ = tx.send(Item::Report(r));
            })),
            on_raw: Some(Arc::new(move |d| {
                let _ = tx_raw.send(Item::Raw(d));
            })),
            ..Default::default()
        },
    )?;
    if let Err(e) = channel.connect() {
        channel.close();
        return Err(Fatal(format!("蓝牙连接失败: {e}")).into());
    }
    Ok(Box::new(BleStream { rx, channel }))
}

fn choose(dev: &DeviceConfig, via: Via) -> Result<Box<dyn ReportStream>> {
    match via {
        Via::Bluetooth => return reports_ble(dev),
        Via::Dongle => return reports_hid(dev),
        Via::Auto => {}
    }
    if let Some(bt) = &dev.bluetooth {
        // 接收器会一直插着, 只有蓝牙连接状态能说明鼠标此刻走哪条
        let probe = BleChannel::new(&bt.address, BleOptions::default())?;
        let on_bluetooth = probe.is_present();
        probe.close();
        if on_bluetooth {
            return reports_ble(dev);
        }
    }
    reports_hid(dev)
}

pub fn status(config_path: &str, watch: bool, via: Via) -> Result<u8> {
    let dev = read_device_section(config_path)?;
    let mut reports = choose(&dev, via)?;
    let t0 = Instant::now();
    if !watch {
        while t0.elapsed() < Duration::from_secs(8) {
            if let Item::Report(rep) = reports.next()? {
                if let Some(bat) = battery_of(Some(&rep)) {
                    println!("电量: {bat}");
                    return Ok(0);
                }
            }
        }
        eprintln!("8 秒内没有收到电量报告(鼠标可能在休眠, 动一下鼠标再试)");
        return Ok(1);
    }
    println!("监听状态报告 (Ctrl+C 退出); 试试按鼠标上的 DPI 键 ...");
    loop {
        let item = reports.next()?;
        print_report(&item, &format!("{:7.1}s", t0.elapsed().as_secs_f64()));
    }
}
