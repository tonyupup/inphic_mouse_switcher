"""
蓝牙(BLE)通道的协议常量与纯函数(不依赖任何蓝牙库)。

来源: HUB 的 FUN_00409b20 / FUN_0040a310 与 Inphic IN10 的真机只读验证(见 wiki):
    服务 0xFEE0 下 FEE3 = 写命令(原始载荷, 无 04 L+5 帧头与校验), FEE4 = 状态通知。
    FEE4 通知 = HID 状态报告去掉开头的 report id 03, 如 93 40 01 42。
"""

from __future__ import annotations

from dataclasses import replace

from inphic.protocol.status import STATUS_REPORT_ID, StatusReport, parse_report

SERVICE_UUID = "0000fee0-0000-1000-8000-00805f9b34fb"
WRITE_UUID = "0000fee3-0000-1000-8000-00805f9b34fb"
NOTIFY_UUID = "0000fee4-0000-1000-8000-00805f9b34fb"


def parse_address(text: str) -> int:
    """'78:87:11:75:9D:56' / '788711759D56' -> 整数地址。"""
    digits = str(text).replace(":", "").replace("-", "").strip()
    if len(digits) != 12:
        raise ValueError(f"蓝牙地址需要 12 位十六进制(如 78:87:11:75:9D:56), 实际 {text!r}")
    try:
        return int(digits, 16)
    except ValueError:
        raise ValueError(f"蓝牙地址 {text!r} 不是十六进制") from None


def report_from_notification(data: bytes) -> StatusReport | None:
    """FEE4 通知 -> StatusReport; 不是状态报告返回 None。"""
    report = parse_report(bytes([STATUS_REPORT_ID]) + bytes(data))
    return None if report is None else replace(report, raw=bytes(data))      # raw 记收到的原样字节(不含补回去的 03)
