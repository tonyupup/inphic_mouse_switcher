"""
读取鼠标状态(电量等)。

    uv run python status.py                 # 读一次电量
    uv run python status.py --watch         # 持续打印全部状态报告, 方便观察 DPI 键/回报率/灯光变化时的上报
    uv run python status.py config.hub.yaml

注意: 只读, 不会向鼠标写任何东西。可以和 switcher.py 同时运行(每个句柄各自收到输入报告)。
"""

import sys
import time
from pathlib import Path

import hid
import yaml

from device_status import KIND_BATTERY, battery_of, parse_report, read_battery


# 状态报告(Report ID 3)不在配置集合(0xFF00, Col04)上, 而在同一个接口 2 的另一个集合 Col03
# (usage_page 0x000A, usage 0)。HID 接口 2 被 Windows 拆成 4 个集合, 按 Report ID 分流。
STATUS_USAGE_PAGE = 0x000A
CONFIG_INTERFACE = 2


def open_device(dev_cfg: dict):
    page = dev_cfg.get("status_usage_page", STATUS_USAGE_PAGE)
    for info in hid.enumerate(dev_cfg["vendor_id"], dev_cfg["product_id"]):
        if info.get("interface_number") == CONFIG_INTERFACE and info.get("usage_page") == page:
            dev = hid.device()
            dev.open_path(info["path"])
            return dev
    raise SystemExit(
        f"找不到状态集合: VID={dev_cfg['vendor_id']:#06x} PID={dev_cfg['product_id']:#06x} "
        f"接口 {CONFIG_INTERFACE} usage_page={page:#06x}, 检查鼠标/接收器是否已连接"
    )


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    watch = "--watch" in sys.argv
    path = Path(args[0]) if args else Path(__file__).parent / "config.hub.yaml"
    dev_cfg = yaml.safe_load(path.read_text(encoding="utf-8"))["device"]
    dev = open_device(dev_cfg)
    try:
        if not watch:
            bat = read_battery(dev, timeout_s=6.0)
            if bat is None:
                raise SystemExit("6 秒内没有收到电量报告(鼠标可能在休眠, 动一下鼠标再试)")
            print(f"电量: {bat}")
            return
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
        pass
    finally:
        dev.close()


if __name__ == "__main__":
    main()
