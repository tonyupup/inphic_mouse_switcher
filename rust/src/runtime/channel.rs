//! 与鼠标的 HID 通信: 按条件找到 HID 集合、打开、串行写帧、检测设备是否还在。
//!
//! Windows 把鼠标的接口 2 拆成 4 个 HID 集合(Col01–Col04), 按 Report ID 分流:
//! - Col04 (usage_page 0xFF00)  发配置命令(Report ID 4, 设备的确认应答也是 4)
//! - Col03 (usage_page 0x000A)  设备主动上报的状态(电量等, Report ID 3)
//!
//! 读错集合会一直收不到数据。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use crate::profile::ConfiguredProfile;
use crate::protocol::status::ReportRead;

pub const CONFIG_INTERFACE: i32 = 2;
pub const STATUS_USAGE_PAGE: u16 = 0x000A;

/// HID 枚举到的一个集合。
#[derive(Debug, Clone)]
pub struct HidInfo {
    pub path: Vec<u8>,
    pub usage_page: u16,
    pub usage: u16,
    pub interface_number: i32,
}

/// 一个已打开的 HID 集合。
pub trait HidDevice: Send {
    /// 写一个输出报告(第一个字节是 report id), 返回写入的字节数。
    fn write(&mut self, data: &[u8]) -> Result<usize>;
    /// 最多等 `timeout_ms` 毫秒读一个输入报告; 没数据返回空。
    fn read(&mut self, timeout_ms: i32) -> Result<Vec<u8>>;
}

/// HID 后端: 枚举与打开。Windows 上是 hidapi, 测试里是假的。
pub trait HidBackend: Send + Sync {
    fn enumerate(&self, vendor_id: u16, product_id: u16) -> Result<Vec<HidInfo>>;
    fn open(&self, info: &HidInfo) -> Result<Box<dyn HidDevice>>;
}

impl ReportRead for Box<dyn HidDevice> {
    fn read(&mut self, timeout_ms: i32) -> Result<Vec<u8>> {
        HidDevice::read(self.as_mut(), timeout_ms)
    }
}

/// 找集合的条件。
#[derive(Debug, Clone, Copy, Default)]
pub struct Match {
    pub usage_page: u16,
    pub usage: Option<u16>,
    pub interface: Option<i32>,
}

/// 返回第一个匹配的 HID 集合, 没有则返回 None。
pub fn find_collection(backend: &dyn HidBackend, vendor_id: u16, product_id: u16, m: Match) -> Result<Option<HidInfo>> {
    Ok(backend.enumerate(vendor_id, product_id)?.into_iter().find(|info| {
        info.usage_page == m.usage_page
            && m.usage.is_none_or(|u| info.usage == u)
            && m.interface.is_none_or(|i| info.interface_number == i)
    }))
}

/// 打开匹配的集合; 找不到时报错, 附带检查建议。
pub fn open_collection(backend: &dyn HidBackend, vendor_id: u16, product_id: u16, m: Match) -> Result<Box<dyn HidDevice>> {
    let Some(info) = find_collection(backend, vendor_id, product_id, m)? else {
        bail!(
            "找不到 VID={vendor_id:#06x} PID={product_id:#06x} 的 HID 集合 {m:?}, 检查鼠标/接收器是否已连接"
        );
    };
    backend.open(&info)
}

/// 到鼠标的一条连接(2.4G 接收器或蓝牙)。
pub trait Channel: Send + Sync {
    /// "dongle" / "bluetooth"
    fn kind(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn supports_polling_rate(&self) -> bool;
    /// 鼠标是否经这条连接在线(不打开设备, 用于发现拔插/休眠重连)。
    fn is_present(&self) -> bool;
    /// 把方案发给鼠标。`delay` 是帧/包间隔, None = 用通道自己的默认值。
    fn send_profile(&self, profile: &ConfiguredProfile, delay: Option<Duration>) -> Result<()>;
    /// 放掉占用的资源(蓝牙: 让 HUB 能用 FEE0), 下次发送会自动重连。
    fn release(&self) {}
    fn close(&self);
}

/// 到鼠标配置集合的连接: 懒打开、串行发送帧、写失败就丢掉句柄下次重开。
pub struct HidChannel {
    backend: Arc<dyn HidBackend>,
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    dev: Mutex<Option<Box<dyn HidDevice>>>,
}

impl HidChannel {
    pub const DEFAULT_DELAY: Duration = Duration::from_millis(520);

    pub fn new(backend: Arc<dyn HidBackend>, vendor_id: u16, product_id: u16, usage_page: u16, usage: u16) -> Self {
        HidChannel { backend, vendor_id, product_id, usage_page, usage, dev: Mutex::new(None) }
    }

    fn config_match(&self) -> Match {
        Match { usage_page: self.usage_page, usage: Some(self.usage), interface: None }
    }

    /// 依次写入 frames, 帧间隔 `delay`。任何一步失败都丢弃句柄(可能已失效)并重新抛出。
    pub fn send_frames(&self, frames: &[Vec<u8>], delay: Duration) -> Result<()> {
        let mut guard = self.dev.lock().unwrap();
        let result = (|| -> Result<()> {
            if guard.is_none() {
                *guard = Some(open_collection(&*self.backend, self.vendor_id, self.product_id, self.config_match())?);
            }
            let dev = guard.as_mut().unwrap();
            for (i, data) in frames.iter().enumerate() {
                // HUB 的 hid_write(h, buf, 0x41): 64 字节帧后多带 1 个 0, 共 65 字节, 这里对齐
                let mut buf = data.clone();
                buf.push(0);
                dev.write(&buf).map_err(|e| anyhow!("第 {}/{} 个包写入失败: {e}", i + 1, frames.len()))?;
                if i != frames.len() - 1 {
                    std::thread::sleep(delay);
                }
            }
            Ok(())
        })();
        if result.is_err() {
            *guard = None; // 句柄可能已失效, 下次重开
        }
        result
    }
}

impl Channel for HidChannel {
    fn kind(&self) -> &'static str {
        "dongle"
    }

    fn label(&self) -> &'static str {
        "2.4G 接收器"
    }

    fn supports_polling_rate(&self) -> bool {
        true
    }

    fn is_present(&self) -> bool {
        matches!(find_collection(&*self.backend, self.vendor_id, self.product_id, self.config_match()), Ok(Some(_)))
    }

    /// 把方案按 2.4G 的帧格式(64 字节帧)发出去。
    fn send_profile(&self, profile: &ConfiguredProfile, delay: Option<Duration>) -> Result<()> {
        self.send_frames(&profile.build_packets()?, delay.unwrap_or(Self::DEFAULT_DELAY))
    }

    fn close(&self) {
        *self.dev.lock().unwrap() = None;
    }
}

// ---------------- hidapi 后端(仅 Windows) ----------------

#[cfg(windows)]
pub use win::HidApiBackend;

#[cfg(windows)]
mod win {
    use std::ffi::CString;

    use anyhow::{Context, Result};

    use super::{HidBackend, HidDevice, HidInfo};

    pub struct HidApiBackend {
        api: std::sync::Mutex<hidapi::HidApi>,
    }

    impl HidApiBackend {
        pub fn new() -> Result<Self> {
            Ok(HidApiBackend { api: std::sync::Mutex::new(hidapi::HidApi::new().context("初始化 HID 失败")?) })
        }
    }

    struct Dev(hidapi::HidDevice);

    impl HidDevice for Dev {
        fn write(&mut self, data: &[u8]) -> Result<usize> {
            Ok(self.0.write(data)?)
        }

        fn read(&mut self, timeout_ms: i32) -> Result<Vec<u8>> {
            let mut buf = [0u8; 65];
            let n = self.0.read_timeout(&mut buf, timeout_ms)?;
            Ok(buf[..n].to_vec())
        }
    }

    impl HidBackend for HidApiBackend {
        fn enumerate(&self, vendor_id: u16, product_id: u16) -> Result<Vec<HidInfo>> {
            let mut api = self.api.lock().unwrap();
            api.refresh_devices()?;
            Ok(api
                .device_list()
                .filter(|d| d.vendor_id() == vendor_id && d.product_id() == product_id)
                .map(|d| HidInfo {
                    path: d.path().to_bytes().to_vec(),
                    usage_page: d.usage_page(),
                    usage: d.usage(),
                    interface_number: d.interface_number(),
                })
                .collect())
        }

        fn open(&self, info: &HidInfo) -> Result<Box<dyn HidDevice>> {
            let path = CString::new(info.path.clone())?;
            let dev = self.api.lock().unwrap().open_path(&path)?;
            Ok(Box::new(Dev(dev)))
        }
    }
}
