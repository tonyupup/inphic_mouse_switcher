"""测试共用的夹具加载。夹具都来自真实 USB 抓包(见 tests/fixtures)。"""

from __future__ import annotations

import zlib
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def captured_frames() -> list[bytes]:
    """usb / profile_switch / key_remap 三份抓包里全部不重复的 OUT 帧。"""
    return [
        bytes.fromhex(line.strip())
        for line in (FIXTURES / "captured_frames.txt").read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    ]


def captured_macro_frames() -> dict[str, list[bytes]]:
    """宏抓包的 4 帧(开始包、按键表、宏分片 0、宏分片 1), 按标签分: A、B、M2、M3、M4。"""
    out: dict[str, list[bytes]] = {}
    for line in (FIXTURES / "captured_macro_frames.txt").read_text(encoding="utf-8").splitlines():
        if line.strip() and not line.startswith("#"):
            tag, hexs = line.split()
            out.setdefault(tag, []).append(bytes.fromhex(hexs))
    return out


def profile_switch_frames() -> dict[str, list[bytes]]:
    """profile_switch 抓包里 cf / default 两个方案各 6 个命令帧。"""
    raw = yaml.safe_load((FIXTURES / "profile_switch_frames.yaml").read_text(encoding="utf-8"))
    return {name: [bytes.fromhex(h) for h in frames] for name, frames in raw.items()}


def profiles_5_6() -> bytes:
    """HUB 存档 pro.data 里的方案 5("配置1")和方案 6("CF"), 压缩存放, 解压后是两个连续的方案结构体。"""
    return zlib.decompress((FIXTURES / "profiles_5_6.bin.z").read_bytes())
