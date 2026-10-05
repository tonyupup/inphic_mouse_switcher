"""status 子命令: 读鼠标主动上报的状态报告(电量等)。只读, 不向鼠标写任何东西。

可以和 run 同时运行(每个 HID 句柄各自收到输入报告)。
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

import yaml

from inphic.protocol.status import KIND_BATTERY, battery_of, parse_report, read_battery
from inphic.runtime.channel import CONFIG_INTERFACE, STATUS_USAGE_PAGE, open_collection


def _open_status_device(config_path: str):
    path = Path(config_path)
    try:
        dev_cfg = yaml.safe_load(path.read_text(encoding="utf-8"))["device"]
    except (FileNotFoundError, KeyError, TypeError):
        raise ValueError(f"{path} 不存在, 或里面没有 device 段") from None
    try:
        return open_collection(
            dev_cfg["vendor_id"], dev_cfg["product_id"], usage_page=STATUS_USAGE_PAGE, interface=CONFIG_INTERFACE
        )
    except RuntimeError as e:
        raise SystemExit(str(e)) from None


def status(config_path: str, watch: bool = False) -> int:
    dev = _open_status_device(config_path)
    try:
        if not watch:
            bat = read_battery(dev, timeout_s=6.0)
            if bat is None:
                print("6 秒内没有收到电量报告(鼠标可能在休眠, 动一下鼠标再试)", file=sys.stderr)
                return 1
            print(f"电量: {bat}")
            return 0
        print("监听状态报告 (Ctrl+C 退出); 试试按鼠标上的 DPI 键 ...")
        t0 = time.monotonic()
        while True:
            data = dev.read(64, 500)
            if not data:
                continue
            rep = parse_report(data)
            stamp = f"{time.monotonic() - t0:7.1f}s"
            if rep is None:
                print(f"{stamp}  其他报告 {bytes(data).hex(' ')}")
            elif rep.kind == KIND_BATTERY and battery_of(rep):
                print(f"{stamp}  电量 {battery_of(rep)}")
            else:
                print(f"{stamp}  {rep.name} a={rep.a:#04x} b={rep.b:#04x} (数值 {rep.value})")
    except KeyboardInterrupt:
        return 0
    finally:
        dev.close()
