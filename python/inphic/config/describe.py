"""把整份配置用中文描述出来: 配置文件、鼠标、宏库、切换规则、各方案的实际生效设置。"""

from __future__ import annotations

from pathlib import Path

from inphic.config.loader import AppConfig
from inphic.profile import describe_profile

_WIDTH = 72


def describe_rules(config: AppConfig) -> list[str]:
    lines = []
    for n, rule in enumerate(config.rules, 1):
        who = "其他所有程序" if rule["process"] == "*" else f"前台是 {rule['process']}"
        lines.append(f"  {n}. {who} → 方案「{rule['profile']}」")
    return lines


def describe_connection(config: AppConfig) -> str:
    bt = config.bluetooth
    mode = config.connection
    if mode == "dongle" or not bt:
        return "2.4G 接收器" + ("（蓝牙未配置）" if mode == "auto" else "")
    addr = f"蓝牙 {bt['address']}"
    if mode == "bluetooth":
        return addr + "（只用蓝牙；蓝牙下回报率设置无效；需先退出 HUB）"
    return f"自动 —— 鼠标连着蓝牙时用 {addr}，否则用 2.4G 接收器（蓝牙下回报率设置无效；蓝牙时需先退出 HUB）"


def describe_config(config: AppConfig, config_path) -> str:
    dev = config.device
    lines = [
        " inphic 当前设置 ".center(_WIDTH, "="),
        f"配置文件  {Path(config_path).resolve()}",
        f"鼠标      VID 0x{dev['vendor_id']:04X} · PID 0x{dev['product_id']:04X} · 配置通道 usage_page 0x{dev['usage_page']:04X}",
    ]
    lines.append("连接方式  " + describe_connection(config))
    lib = config.macro_library
    if lib is not None:
        groups = " · ".join(f"{g} {len(names)} 个" for g, names in lib.groups().items())
        lines.append(f"宏库      共 {len(lib.names())} 个宏（{groups}）")
    lines.append("")
    lines.append("切换规则（从上往下，第一个匹配的生效）:")
    lines += describe_rules(config)
    for name, cp in config.profiles.items():
        lines.append("")
        lines.append(f"方案「{name}」")
        lines += ["  " + line for line in describe_profile(cp.profile)]
    lines.append("")
    lines.append("★ = 与出厂默认不同。未在配置里写的项，显示的是出厂默认值。")
    lines.append("=" * _WIDTH)
    return "\n".join(lines)
