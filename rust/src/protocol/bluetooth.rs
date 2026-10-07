//! 蓝牙(BLE)通道的协议常量与纯函数(不依赖任何蓝牙库)。
//!
//! 来源: HUB 的 FUN_00409b20 / FUN_0040a310 与 Inphic IN10 的真机只读验证(见 wiki):
//! 服务 0xFEE0 下 FEE3 = 写命令(原始载荷, 无 04 L+5 帧头与校验), FEE4 = 状态通知。
//! FEE4 通知 = HID 状态报告去掉开头的 report id 03, 如 93 40 01 42。

use crate::bail;
use crate::error::Result;
use crate::protocol::status::{parse_report, StatusReport, STATUS_REPORT_ID};

pub const SERVICE_UUID: u128 = 0x0000fee0_0000_1000_8000_00805f9b34fb;
pub const WRITE_UUID: u128 = 0x0000fee3_0000_1000_8000_00805f9b34fb;
pub const NOTIFY_UUID: u128 = 0x0000fee4_0000_1000_8000_00805f9b34fb;

/// `'78:87:11:75:9D:56'` / `'788711759D56'` -> 整数地址。
pub fn parse_address(text: &str) -> Result<u64> {
    let digits: String = text.replace([':', '-'], "").trim().to_string();
    if digits.chars().count() != 12 {
        bail!("蓝牙地址需要 12 位十六进制(如 78:87:11:75:9D:56), 实际 {text:?}");
    }
    match u64::from_str_radix(&digits, 16) {
        Ok(v) => Ok(v),
        Err(_) => bail!("蓝牙地址 {text:?} 不是十六进制"),
    }
}

/// FEE4 通知 -> StatusReport; 不是状态报告返回 None。
pub fn report_from_notification(data: &[u8]) -> Option<StatusReport> {
    let mut full = vec![STATUS_REPORT_ID];
    full.extend_from_slice(data);
    let mut report = parse_report(&full)?;
    report.raw = data.to_vec(); // raw 记收到的原样字节(不含补回去的 03)
    Some(report)
}
