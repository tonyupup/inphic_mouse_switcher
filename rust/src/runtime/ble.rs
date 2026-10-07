//! 蓝牙(BLE)通道: 鼠标走蓝牙时, 配置命令写进 GATT 服务 0xFEE0 的特征值 FEE3, 状态从 FEE4 通知来。
//!
//! 来源: HUB 的 FUN_00409b20(写) / FUN_0040a310(订阅), 以及对 Inphic IN10 的真机只读验证(见 wiki)。
//! - FEE3  写     值 = 原始命令载荷(不带 04 L+5 帧头, 不带校验), 宏消息 111 字节整条写
//! - FEE4  通知   内容 = 状态报告去掉开头的 report id 03, 如 93 40 01 42
//! - 蓝牙下不发回报率命令(0x06): HUB 也不发, 官方 UI 里蓝牙状态下回报率设置无效
//!
//! ⚠ 同一时间只能有一个程序打开 FEE0: HUB(INPHIC.exe)或 Web 驱动(Inphic.exe)在运行时会被拒绝访问。
//!
//! 直接用 Windows 自带的 WinRT(`windows` crate), 不走扫描: 按地址连接已连上的 LE 设备。
//! 所有 WinRT 异步调用都在调用线程里阻塞等待(带超时), 不需要事件循环线程。

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};

use crate::profile::ConfiguredProfile;
use crate::protocol::bluetooth::{parse_address, report_from_notification};
use crate::protocol::status::{StatusReport, KIND_COMMAND_ACK};
use crate::runtime::channel::Channel;

const LOG_TARGET: &str = "inphic.ble";

/// 一次到鼠标 FEE0 服务的 GATT 会话(已订阅 FEE4)。
pub trait BleSession: Send {
    /// 把载荷写进 FEE3(带响应)。
    fn write(&mut self, payload: &[u8]) -> Result<()>;
    fn connected(&self) -> bool;
    fn close(&mut self);
}

/// FEE4 的通知回调: 参数是收到的原始字节。
pub type NotifyFn = Arc<dyn Fn(Vec<u8>) + Send + Sync>;
/// 打开会话: (蓝牙地址, 通知回调) -> 会话。
pub type OpenSession = Box<dyn Fn(u64, NotifyFn) -> Result<Box<dyn BleSession>> + Send + Sync>;
/// 不打开 FEE0, 只看鼠标是否通过蓝牙连着。
pub type Probe = Box<dyn Fn(u64) -> bool + Send + Sync>;

pub type StatusListener = Arc<dyn Fn(StatusReport) + Send + Sync>;
pub type RawListener = Arc<dyn Fn(Vec<u8>) + Send + Sync>;
pub type DisconnectListener = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Acks {
    /// 收到的命令应答 (命令字, 结果; 0=成功), 真机验证: 93 50 00 <命令字>
    list: Mutex<Vec<(u8, u8)>>,
    cond: Condvar,
}

pub struct BleOptions {
    pub packet_delay: Duration,
    pub listener: Option<StatusListener>,
    pub on_raw: Option<RawListener>,
    pub on_disconnect: Option<DisconnectListener>,
    pub open_session: Option<OpenSession>,
    pub probe: Option<Probe>,
    pub ack_timeout: Duration,
    pub ack_retries: u32,
}

impl Default for BleOptions {
    fn default() -> Self {
        BleOptions {
            packet_delay: Duration::from_millis(200),
            listener: None,
            on_raw: None,
            on_disconnect: None,
            open_session: None,
            probe: None,
            ack_timeout: Duration::from_secs(2),
            ack_retries: 2,
        }
    }
}

/// 到鼠标蓝牙通道的连接。接口与 `HidChannel` 对齐。
///
/// 会话懒打开, 写失败就丢掉会话下次重开。`listener` 在通知线程里被调用;
/// `on_disconnect` 在会话丢失时调用(不能在里面回调本通道)。
pub struct BleChannel {
    address: u64,
    packet_delay: Duration,
    on_disconnect: Option<DisconnectListener>,
    open_session: OpenSession,
    probe: Probe,
    ack_timeout: Duration,
    ack_retries: u32,
    notify: NotifyFn,
    acks: Arc<Acks>,
    session: Mutex<Option<Box<dyn BleSession>>>,
}

impl BleChannel {
    /// 宏消息(0x09)的应答没在真机上见过(已见 0c/04/05/07/08), 所以它的应答是可选的: 没等到只警告, 不当成失败
    pub const ACK_OPTIONAL: [u8; 1] = [0x09];

    pub fn new(address: &str, opts: BleOptions) -> crate::error::Result<Self> {
        let address = parse_address(address)?;
        let acks = Arc::new(Acks::default());
        let notify: NotifyFn = {
            let acks = acks.clone();
            let listener = opts.listener.clone();
            let on_raw = opts.on_raw.clone();
            Arc::new(move |data: Vec<u8>| {
                let report = report_from_notification(&data);
                if let Some(r) = report.as_ref().filter(|r| r.kind == KIND_COMMAND_ACK) {
                    acks.list.lock().unwrap().push((r.b, r.a));
                    acks.cond.notify_all();
                }
                match report {
                    Some(r) => {
                        if let Some(l) = &listener {
                            l(r); // 无论能不能解析都交出去, 不丢
                        }
                    }
                    None => {
                        if let Some(l) = &on_raw {
                            l(data);
                        }
                    }
                }
            })
        };
        Ok(BleChannel {
            address,
            packet_delay: opts.packet_delay,
            on_disconnect: opts.on_disconnect,
            open_session: opts.open_session.unwrap_or_else(default_open_session),
            probe: opts.probe.unwrap_or_else(default_probe),
            ack_timeout: opts.ack_timeout,
            ack_retries: opts.ack_retries,
            notify,
            acks,
            session: Mutex::new(None),
        })
    }

    // ---- 会话 ----
    fn ensure_session<'a>(&self, slot: &'a mut Option<Box<dyn BleSession>>) -> Result<&'a mut Box<dyn BleSession>> {
        if slot.as_ref().is_some_and(|s| !s.connected()) {
            self.drop_session(slot);
        }
        if slot.is_none() {
            *slot = Some((self.open_session)(self.address, self.notify.clone())?);
            log::debug!(target: LOG_TARGET, "蓝牙会话已建立");
        }
        Ok(slot.as_mut().unwrap())
    }

    fn drop_session(&self, slot: &mut Option<Box<dyn BleSession>>) {
        if let Some(mut s) = slot.take() {
            s.close();
            if let Some(cb) = &self.on_disconnect {
                cb();
            }
        }
    }

    /// 建立会话(订阅状态通知); 失败返回错误。
    pub fn connect(&self) -> Result<()> {
        let mut slot = self.session.lock().unwrap();
        self.ensure_session(&mut slot)?;
        Ok(())
    }

    // ---- 发送 ----
    /// 等命令 `cmd` 的应答, 返回结果字节(0=成功); 超时返回 None。其他命令的应答忽略。
    fn wait_ack(&self, cmd: u8) -> Option<u8> {
        let deadline = Instant::now() + self.ack_timeout;
        let mut list = self.acks.list.lock().unwrap();
        loop {
            if let Some(i) = list.iter().position(|&(c, _)| c == cmd) {
                return Some(list.remove(i).1);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            list = self.acks.cond.wait_timeout(list, left).unwrap().0;
        }
    }

    fn write_acked(&self, session: &mut dyn BleSession, payload: &[u8]) -> Result<()> {
        let cmd = payload[0];
        for attempt in 0..=self.ack_retries {
            self.acks.list.lock().unwrap().retain(|a| a.0 != cmd); // 丢掉上一轮残留的同命令应答
            session.write(payload)?;
            match self.wait_ack(cmd) {
                Some(0) => return Ok(()),
                Some(result) => bail!("鼠标拒绝了命令 {cmd:#04x}（应答结果 {result:#04x}）"),
                None => {}
            }
            if Self::ACK_OPTIONAL.contains(&cmd) {
                log::warn!(target: LOG_TARGET, "命令 {cmd:#04x} 没有收到应答（该命令的应答没在真机上见过, 当作已发送）");
                return Ok(());
            }
            log::warn!(
                target: LOG_TARGET,
                "命令 {cmd:#04x} 没有收到应答, 重发（第 {}/{} 次）",
                attempt + 1,
                self.ack_retries
            );
        }
        bail!("命令 {cmd:#04x} 重发 {} 次仍没有收到应答", self.ack_retries)
    }

    /// 依次把载荷写进 FEE3, 间隔 `delay`(默认 200 ms, 与 HUB 一致)。失败丢掉会话并重新抛出。
    pub fn send_payloads(&self, payloads: &[Vec<u8>], delay: Option<Duration>) -> Result<()> {
        let delay = delay.unwrap_or(self.packet_delay);
        let mut slot = self.session.lock().unwrap();
        let result = (|| -> Result<()> {
            let session = self.ensure_session(&mut slot)?;
            for (i, payload) in payloads.iter().enumerate() {
                self.write_acked(session.as_mut(), payload)?;
                if i != payloads.len() - 1 {
                    std::thread::sleep(delay);
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.drop_session(&mut slot);
        }
        result
    }
}

impl Channel for BleChannel {
    fn kind(&self) -> &'static str {
        "bluetooth"
    }

    fn label(&self) -> &'static str {
        "蓝牙"
    }

    fn supports_polling_rate(&self) -> bool {
        false
    }

    /// 鼠标是否通过蓝牙连着(不打开 FEE0, 所以不会和 HUB 抢)。
    fn is_present(&self) -> bool {
        match self.session.try_lock() {
            Ok(slot) => match slot.as_ref() {
                Some(s) => s.connected(),
                None => (self.probe)(self.address),
            },
            Err(_) => true, // 正在发送, 说明连着
        }
    }

    fn send_profile(&self, profile: &ConfiguredProfile, delay: Option<Duration>) -> Result<()> {
        let payloads = profile.build_payloads(false).map_err(|e| anyhow!(e))?;
        self.send_payloads(&payloads, delay)
    }

    /// 放掉 FEE0 会话(让 HUB 能用), 下次发送会自动重连。
    fn release(&self) {
        let mut slot = self.session.lock().unwrap();
        self.drop_session(&mut slot);
    }

    fn close(&self) {
        self.release();
    }
}

#[cfg(windows)]
fn default_open_session() -> OpenSession {
    Box::new(win::WinrtSession::open)
}

#[cfg(windows)]
fn default_probe() -> Probe {
    Box::new(win::probe)
}

#[cfg(not(windows))]
fn default_open_session() -> OpenSession {
    Box::new(|_, _| bail!("蓝牙只支持 Windows"))
}

#[cfg(not(windows))]
fn default_probe() -> Probe {
    Box::new(|_| false)
}

// ---------------- WinRT 实现(仅 Windows) ----------------

#[cfg(windows)]
mod win {
    use std::time::{Duration, Instant};

    use anyhow::{anyhow, bail, Context, Result};
    use windows::core::{RuntimeType, GUID};
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattClientCharacteristicConfigurationDescriptorValue, GattCharacteristic, GattCommunicationStatus,
        GattValueChangedEventArgs, GattWriteOption,
    };
    use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothLEDevice};
    use windows::Foundation::{AsyncStatus, EventRegistrationToken, IAsyncOperation, TypedEventHandler};
    use windows::Storage::Streams::{DataReader, DataWriter};
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    use super::{BleSession, NotifyFn};
    use crate::protocol::bluetooth::{NOTIFY_UUID, SERVICE_UUID, WRITE_UUID};

    const TIMEOUT: Duration = Duration::from_secs(15);

    /// 每个调用 WinRT 的线程都要先初始化一次(重复初始化、模式不同的错误都可以忽略)。
    fn init_winrt() {
        thread_local! { static INIT: () = unsafe { let _ = RoInitialize(RO_INIT_MULTITHREADED); }; }
        INIT.with(|_| ());
    }

    /// 阻塞等一个 WinRT 异步操作, 带超时(超时就取消)。
    fn wait<T: RuntimeType + 'static>(op: IAsyncOperation<T>, what: &str) -> Result<T> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match op.Status()? {
                AsyncStatus::Started => {
                    if Instant::now() >= deadline {
                        let _ = op.Cancel();
                        bail!("{what} 超时({}s)", TIMEOUT.as_secs());
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                AsyncStatus::Canceled => bail!("{what} 被取消"),
                _ => return Ok(op.GetResults()?), // Completed / Error(GetResults 会返回错误)
            }
        }
    }

    pub struct WinrtSession {
        device: BluetoothLEDevice,
        write_char: GattCharacteristic,
        notify_char: GattCharacteristic,
        token: EventRegistrationToken,
    }

    impl WinrtSession {
        pub fn open(address: u64, on_notify: NotifyFn) -> Result<Box<dyn BleSession>> {
            init_winrt();
            let device = wait(BluetoothLEDevice::FromBluetoothAddressAsync(address)?, "查找蓝牙设备")
                .map_err(|_| anyhow!("系统里没有这个蓝牙设备, 先在 Windows 设置里配对"))?;
            if device.ConnectionStatus()? != BluetoothConnectionStatus::Connected {
                bail!("鼠标没有通过蓝牙连接(休眠了? 动一下鼠标)");
            }
            let svc_res = wait(device.GetGattServicesForUuidAsync(GUID::from_u128(SERVICE_UUID))?, "读取 FEE0 服务")?;
            let status = svc_res.Status()?;
            if status == GattCommunicationStatus::AccessDenied {
                bail!("FEE0 服务被拒绝访问: 请先退出 INPHIC HUB(INPHIC.exe)和 Inphic.exe");
            }
            let services = svc_res.Services()?;
            if status != GattCommunicationStatus::Success || services.Size()? == 0 {
                bail!("读不到 FEE0 服务(状态 {})", status.0);
            }
            let service = services.GetAt(0)?;
            let mut chars = Vec::new();
            for uuid in [WRITE_UUID, NOTIFY_UUID] {
                let res = wait(
                    service.GetCharacteristicsForUuidAsync(GUID::from_u128(uuid))?,
                    "读取特征值",
                )?;
                let list = res.Characteristics()?;
                if res.Status()? != GattCommunicationStatus::Success || list.Size()? == 0 {
                    bail!("FEE0 下没有 {:04X} 特征值(状态 {})", (uuid >> 96) as u16, res.Status()?.0);
                }
                chars.push(list.GetAt(0)?);
            }
            let notify_char = chars.pop().unwrap();
            let write_char = chars.pop().unwrap();

            let handler = TypedEventHandler::<GattCharacteristic, GattValueChangedEventArgs>::new(move |_, args| {
                if let Some(args) = args {
                    let buf = args.CharacteristicValue()?;
                    let mut data = vec![0u8; buf.Length()? as usize];
                    DataReader::FromBuffer(&buf)?.ReadBytes(&mut data)?;
                    on_notify(data);
                }
                Ok(())
            });
            let token = notify_char.ValueChanged(&handler)?;
            let status = wait(
                notify_char.WriteClientCharacteristicConfigurationDescriptorAsync(
                    GattClientCharacteristicConfigurationDescriptorValue::Notify,
                )?,
                "订阅 FEE4",
            )?;
            if status != GattCommunicationStatus::Success {
                let _ = notify_char.RemoveValueChanged(token);
                bail!("订阅 FEE4 失败(状态 {})", status.0);
            }
            Ok(Box::new(WinrtSession { device, write_char, notify_char, token }))
        }
    }

    impl BleSession for WinrtSession {
        fn connected(&self) -> bool {
            self.device.ConnectionStatus().is_ok_and(|s| s == BluetoothConnectionStatus::Connected)
        }

        fn write(&mut self, payload: &[u8]) -> Result<()> {
            init_winrt();
            let writer = DataWriter::new()?;
            writer.WriteBytes(payload)?;
            let res = wait(
                self.write_char.WriteValueWithResultAndOptionAsync(&writer.DetachBuffer()?, GattWriteOption::WriteWithResponse)?,
                "写 FEE3",
            )?;
            if res.Status()? != GattCommunicationStatus::Success {
                let proto = res.ProtocolError().ok().and_then(|r| r.Value().ok());
                bail!("写 FEE3 失败(状态 {}, 协议错误码 {proto:?})", res.Status()?.0);
            }
            Ok(())
        }

        fn close(&mut self) {
            let _ = self.notify_char.RemoveValueChanged(self.token);
            let _ = self.device.Close();
        }
    }

    pub fn probe(address: u64) -> bool {
        init_winrt();
        let Ok(op) = BluetoothLEDevice::FromBluetoothAddressAsync(address) else { return false };
        let Ok(device) = wait(op, "查找蓝牙设备").context("probe") else { return false };
        let connected = device.ConnectionStatus().is_ok_and(|s| s == BluetoothConnectionStatus::Connected);
        let _ = device.Close();
        connected
    }
}
