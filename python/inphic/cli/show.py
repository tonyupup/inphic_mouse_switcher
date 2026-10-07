"""show 子命令: 用中文打印当前生效的设置。只读, 不连接鼠标, 在没有 hid 的环境里也能用。

--payloads: 另外打印每个方案"将要写给鼠标的字节"(干跑, 不发送)——蓝牙下是写进 FEE3 的原始载荷, 2.4G 下是 64 字节帧。
"""

from __future__ import annotations

from inphic.config import describe_config, load_config

_CMD_NAMES = {0x0C: "开始", 0x04: "DPI", 0x05: "灯光", 0x06: "回报率", 0x07: "杂项", 0x08: "按键表", 0x09: "宏"}


def _describe_payloads(config) -> list[str]:
    lines = []
    for name, cp in config.profiles.items():
        lines += ["", f"方案「{name}」将要发送的字节（干跑，没有发送）:"]
        lines.append("  蓝牙（逐条写进 FEE3，不含回报率命令）:")
        for i, payload in enumerate(cp.build_payloads(include_polling=False), 1):
            lines.append(f"    {i:>2}. {_CMD_NAMES.get(payload[0], '?'):<4} {len(payload):>3} 字节  {payload.hex(' ')}")
        frames = cp.build_packets()
        lines.append(f"  2.4G 接收器（{len(frames)} 个 64 字节帧，下面只列前 16 字节）:")
        for i, frame in enumerate(frames, 1):
            lines.append(f"    {i:>2}. {frame[:16].hex(' ')} ...")
    return lines


def show(config_path: str, payloads: bool = False) -> int:
    config = load_config(config_path)
    print(describe_config(config, config_path))
    if payloads:
        print("\n".join(_describe_payloads(config)))
    return 0
