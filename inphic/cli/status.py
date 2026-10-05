"""status 子命令: 读鼠标主动上报的状态报告(电量等)。只读, 不向鼠标写任何东西。

2.4G 接收器: 读 HID 状态集合, 可以和 run 同时运行。默认(--via auto)先看蓝牙是否连着, 连着就读蓝牙。
蓝牙: 订阅 FEE4 通知(只开通知开关, 不写配置); 同一时间只能有一个程序打开 FEE0, 所以不能和 HUB / run 同时用。
"""

from __future__ import annotations

import queue
import sys
import time
from pathlib import Path

import yaml

from inphic.protocol.status import KIND_BATTERY, battery_of, parse_report


def _device_section(config_path: str) -> dict:
    path = Path(config_path)
    try:
        return yaml.safe_load(path.read_text(encoding="utf-8"))["device"]
    except (FileNotFoundError, KeyError, TypeError):
        raise ValueError(f"{path} 不存在, 或里面没有 device 段") from None


def _print_report(rep, stamp: str) -> None:
    if rep.kind == KIND_BATTERY and battery_of(rep):
        print(f"{stamp}  电量 {battery_of(rep)}")
    else:
        print(f"{stamp}  {rep.name} a={rep.a:#04x} b={rep.b:#04x} (数值 {rep.value})")


def _reports_hid(dev_cfg: dict):
    """生成器: 不断产出 StatusReport(2.4G 接收器)。"""
    from inphic.runtime.channel import CONFIG_INTERFACE, STATUS_USAGE_PAGE, open_collection

    try:
        dev = open_collection(
            dev_cfg["vendor_id"], dev_cfg["product_id"], usage_page=STATUS_USAGE_PAGE, interface=CONFIG_INTERFACE
        )
    except RuntimeError as e:
        raise SystemExit(str(e)) from None
    try:
        while True:
            data = dev.read(64, 500)
            rep = parse_report(data) if data else None
            yield rep
    finally:
        dev.close()


def _reports_ble(dev_cfg: dict):
    """生成器: 不断产出 StatusReport(蓝牙 FEE4 通知); 没有新通知时产出 None。"""
    bt = dev_cfg.get("bluetooth")
    if not bt or not bt.get("address"):
        raise SystemExit("config.yaml 的 device 里没有 bluetooth.address")
    from inphic.runtime.ble import BleChannel

    q: queue.Queue = queue.Queue()
    ch = BleChannel(str(bt["address"]), listener=q.put)
    try:
        ch.connect()
    except Exception as e:
        ch.close()
        raise SystemExit(f"蓝牙连接失败: {e}") from None
    try:
        while True:
            try:
                yield q.get(timeout=0.5)
            except queue.Empty:
                yield None
    finally:
        ch.close()


def _choose(dev_cfg: dict, via: str):
    if via == "bluetooth":
        return _reports_ble(dev_cfg)
    if via == "dongle":
        return _reports_hid(dev_cfg)
    bt = dev_cfg.get("bluetooth")
    if bt and bt.get("address"):                    # 接收器会一直插着, 只有蓝牙连接状态能说明鼠标此刻走哪条
        from inphic.runtime.ble import BleChannel

        probe = BleChannel(str(bt["address"]))
        try:
            on_bluetooth = probe.is_present()
        finally:
            probe.close()
        if on_bluetooth:
            return _reports_ble(dev_cfg)
    return _reports_hid(dev_cfg)


def status(config_path: str, watch: bool = False, via: str = "auto") -> int:
    reports = _choose(_device_section(config_path), via)
    t0 = time.monotonic()
    try:
        if not watch:
            while time.monotonic() - t0 < 8.0:
                rep = next(reports)
                if rep is not None and battery_of(rep):
                    print(f"电量: {battery_of(rep)}")
                    return 0
            print("8 秒内没有收到电量报告(鼠标可能在休眠, 动一下鼠标再试)", file=sys.stderr)
            return 1
        print("监听状态报告 (Ctrl+C 退出); 试试按鼠标上的 DPI 键 ...")
        for rep in reports:
            if rep is not None:
                _print_report(rep, f"{time.monotonic() - t0:7.1f}s")
    except KeyboardInterrupt:
        return 0
    finally:
        reports.close()
    return 0
