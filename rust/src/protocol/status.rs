//! 鼠标主动上报的状态报告(电量等)。
//!
//! 接口 2(端点 0x83)上, 鼠标约每 2 秒主动发一个 5 字节输入报告, 不需要请求。注意 Windows 把接口 2 拆成 4 个
//! HID 集合, 这个报告(Report ID 3)在 **Col03 (usage_page 0x000A, usage 0)** 上, 不在发配置命令用的
//! Col04 (0xFF00) 上——读错集合会一直超时。HUB 也不是"主动询问", 就是被动收这个报告:
//!
//! ```text
//! [0x03, 0x93, 类型, a, b]
//! ```
//!
//! 来源: 抓包里反复出现的 `03 93 40 01 45`(用户 HUB 界面当时显示电量 69% = 0x45), 以及 INPHIC.exe 中
//! FUN_0045fbd0 / FUN_0045fe20 对这个报告的解析(类型取自 b2, 数值 = a | b << 8):
//!
//! - 类型 0x40  电量:  a = 状态, b = 电量百分比。抓包验证: state=1, percent=0x45=69
//! - 类型 0x10  DPI 档位变化(按鼠标上的 DPI 键):  数值 1..8 = 第几档      -- 只来自 HUB 代码, 没有抓包
//! - 类型 0x20  回报率变化:  数值 = 序号                                    -- 只来自 HUB 代码, 没有抓包
//! - 类型 0x70  灯光模式变化:  模式 = (数值 >> 4) & 0xF                     -- 只来自 HUB 代码, 没有抓包
//!
//! 电量状态 a 的含义只确认了 1 = 正常放电(抓包时)。HUB 代码里 无线模式下 a == 3 会触发"充电"动画,
//! 有线模式下 a == 2 则显示 100%, 但这些没有抓包, 所以这里只给原始值, 不下结论。
//!
//! 另外每个 OUT 帧之后设备会回两个确认: `04 06 c0 01 00 cb` 和 `04 06 c1 01 00 cc`。

use std::fmt;

pub const STATUS_REPORT_ID: u8 = 0x03;
pub const STATUS_MARKER: u8 = 0x93;

pub const KIND_DPI_STAGE: u8 = 0x10;
pub const KIND_POLLING_RATE: u8 = 0x20;
pub const KIND_BATTERY: u8 = 0x40;
/// 命令应答? HUB: this+0x158 = (数值低字节 == 0), 发送函数据此决定要不要重发
pub const KIND_COMMAND_ACK: u8 = 0x50;
/// HUB: 数值 1..10 存进 this+0xfc(含义未确认)
pub const KIND_LEVEL: u8 = 0x60;
pub const KIND_LIGHT_MODE: u8 = 0x70;
/// HUB: 数值 1..5 只触发存档, 不赋值(含义未确认)
pub const KIND_PROFILE_SLOT: u8 = 0x80;

#[derive(Debug, Clone, Default)]
pub struct StatusReport {
    pub kind: u8,
    /// 第 3 字节(数值低字节)
    pub a: u8,
    /// 第 4 字节(数值高字节)
    pub b: u8,
    /// 收到的原始字节(含第 5 字节之后的内容), 日志用; 不参与比较
    pub raw: Vec<u8>,
}

impl PartialEq for StatusReport {
    fn eq(&self, other: &Self) -> bool {
        (self.kind, self.a, self.b) == (other.kind, other.a, other.b)
    }
}

impl StatusReport {
    pub fn new(kind: u8, a: u8, b: u8) -> Self {
        StatusReport { kind, a, b, raw: Vec::new() }
    }

    pub fn name(&self) -> String {
        match self.kind {
            KIND_DPI_STAGE => "dpi_stage".into(),
            KIND_POLLING_RATE => "polling_rate".into(),
            KIND_BATTERY => "battery".into(),
            KIND_LIGHT_MODE => "light_mode".into(),
            KIND_COMMAND_ACK => "command_ack".into(),
            KIND_LEVEL => "level".into(),
            KIND_PROFILE_SLOT => "profile_slot".into(),
            k => format!("unknown_{k:#04x}"),
        }
    }

    pub fn value(&self) -> u32 {
        self.a as u32 | ((self.b as u32) << 8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    /// 原始状态值; 抓包里是 1
    pub state: u8,
}

impl fmt::Display for Battery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}% (state {})", self.percent, self.state)
    }
}

/// 解析一个输入报告; 不是状态报告(比如 04 06 c0 .. 的确认)返回 None。
pub fn parse_report(data: &[u8]) -> Option<StatusReport> {
    if data.len() < 5 || data[0] != STATUS_REPORT_ID || data[1] != STATUS_MARKER {
        return None;
    }
    Some(StatusReport { kind: data[2], a: data[3], b: data[4], raw: data.to_vec() })
}

pub fn battery_of(report: Option<&StatusReport>) -> Option<Battery> {
    let r = report?;
    if r.kind != KIND_BATTERY || r.b > 100 {
        return None;
    }
    Some(Battery { percent: r.b, state: r.a })
}

/// 能读状态报告的东西(HID 状态集合的句柄, 或测试里的替身)。
pub trait ReportRead {
    /// 最多等 `timeout_ms` 毫秒; 没数据返回空。
    fn read(&mut self, timeout_ms: i32) -> anyhow::Result<Vec<u8>>;
}

/// 读 `timeout_s` 秒, 返回收到的全部状态报告(其他报告被忽略)。
pub fn read_reports(dev: &mut dyn ReportRead, timeout_s: f64, poll_ms: i32) -> anyhow::Result<Vec<StatusReport>> {
    let mut out = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(timeout_s);
    while std::time::Instant::now() < deadline {
        let data = dev.read(poll_ms)?;
        if data.is_empty() {
            continue;
        }
        if let Some(rep) = parse_report(&data) {
            out.push(rep);
        }
    }
    Ok(out)
}

/// 等到第一个电量报告就返回; 超时返回 None。鼠标约每 2 秒报一次, 默认等 5 秒。
pub fn read_battery(dev: &mut dyn ReportRead, timeout_s: f64) -> anyhow::Result<Option<Battery>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(timeout_s);
    while std::time::Instant::now() < deadline {
        let data = dev.read(200)?;
        if data.is_empty() {
            continue;
        }
        if let Some(bat) = battery_of(parse_report(&data).as_ref()) {
            return Ok(Some(bat));
        }
    }
    Ok(None)
}
