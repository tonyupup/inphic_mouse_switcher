"""
鼠标主动上报的状态报告(电量等)。

接口 2(端点 0x83)上, 鼠标约每 2 秒主动发一个 5 字节输入报告, 不需要请求。注意 Windows 把接口 2 拆成 4 个
HID 集合, 这个报告(Report ID 3)在 **Col03 (usage_page 0x000A, usage 0)** 上, 不在发配置命令用的
Col04 (0xFF00) 上——读错集合会一直超时。HUB 也不是"主动询问", 就是被动收这个报告:

    [0x03, 0x93, 类型, a, b]

来源: 抓包里反复出现的 `03 93 40 01 45`(用户 HUB 界面当时显示电量 69% = 0x45), 以及 INPHIC.exe 中
FUN_0045fbd0 / FUN_0045fe20 对这个报告的解析(类型取自 b2, 数值 = a | b << 8):

    类型 0x40  电量:  a = 状态, b = 电量百分比。抓包验证: state=1, percent=0x45=69
    类型 0x10  DPI 档位变化(按鼠标上的 DPI 键):  数值 1..8 = 第几档      -- 只来自 HUB 代码, 没有抓包
    类型 0x20  回报率变化:  数值 = 序号                                    -- 只来自 HUB 代码, 没有抓包
    类型 0x70  灯光模式变化:  模式 = (数值 >> 4) & 0xF                     -- 只来自 HUB 代码, 没有抓包

电量状态 a 的含义只确认了 1 = 正常放电(抓包时)。HUB 代码里 无线模式下 a == 3 会触发"充电"动画,
有线模式下 a == 2 则显示 100%, 但这些没有抓包, 所以这里只给原始值, 不下结论。

另外每个 OUT 帧之后设备会回两个确认: `04 06 c0 01 00 cb` 和 `04 06 c1 01 00 cc`。
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field

STATUS_REPORT_ID = 0x03
STATUS_MARKER = 0x93

KIND_DPI_STAGE = 0x10
KIND_POLLING_RATE = 0x20
KIND_BATTERY = 0x40
KIND_LIGHT_MODE = 0x70

KIND_NAMES = {
    KIND_DPI_STAGE: "dpi_stage",
    KIND_POLLING_RATE: "polling_rate",
    KIND_BATTERY: "battery",
    KIND_LIGHT_MODE: "light_mode",
}


@dataclass(frozen=True)
class StatusReport:
    kind: int
    a: int          # 第 3 字节(数值低字节)
    b: int          # 第 4 字节(数值高字节)
    raw: bytes = field(default=b"", compare=False, repr=False)      # 收到的原始字节(含第 5 字节之后的内容), 日志用

    @property
    def name(self) -> str:
        return KIND_NAMES.get(self.kind, f"unknown_{self.kind:#04x}")

    @property
    def value(self) -> int:
        return self.a | (self.b << 8)


@dataclass(frozen=True)
class Battery:
    percent: int
    state: int      # 原始状态值; 抓包里是 1

    def __str__(self) -> str:
        return f"{self.percent}% (state {self.state})"


def parse_report(data) -> StatusReport | None:
    """解析一个输入报告; 不是状态报告(比如 04 06 c0 .. 的确认)返回 None。"""
    data = bytes(data)
    if len(data) < 5 or data[0] != STATUS_REPORT_ID or data[1] != STATUS_MARKER:
        return None
    return StatusReport(kind=data[2], a=data[3], b=data[4], raw=data)


def battery_of(report: StatusReport | None) -> Battery | None:
    if report is None or report.kind != KIND_BATTERY or report.b > 100:
        return None
    return Battery(percent=report.b, state=report.a)


def read_reports(dev, timeout_s: float = 5.0, poll_ms: int = 200) -> list[StatusReport]:
    """读 timeout_s 秒, 返回收到的全部状态报告(其他报告被忽略)。dev 是 hid.device()。"""
    out: list[StatusReport] = []
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        data = dev.read(64, poll_ms)
        if not data:
            continue
        rep = parse_report(data)
        if rep is not None:
            out.append(rep)
    return out


def read_battery(dev, timeout_s: float = 5.0) -> Battery | None:
    """等到第一个电量报告就返回; 超时返回 None。鼠标约每 2 秒报一次, 默认等 5 秒。"""
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        data = dev.read(64, 200)
        if not data:
            continue
        bat = battery_of(parse_report(data))
        if bat is not None:
            return bat
    return None
