"""
从 USBPcap 抓包文件里提取鼠标配置写入包，存成 templates.yaml 里的一个模板。

这一步产生的是"原始字节模板"，给 mouse_protocol.Profile 当底板用，
不是给人直接编辑的——加好新模板之后，去 config.yaml 里用
`template: <这个名字>` 引用它，然后用 buttons 字段覆盖按键绑定即可。

用法:
    python extract_profile.py <capture.pcapng> <模板名> [templates.yaml路径] [起始秒数] [结束秒数]
"""

import subprocess
import sys
from pathlib import Path

import yaml

TSHARK_CANDIDATES = [
    r"D:\Program Files\Wireshark\tshark.exe",
    r"C:\Program Files\Wireshark\tshark.exe",
]


def find_tshark() -> str:
    for p in TSHARK_CANDIDATES:
        if Path(p).exists():
            return p
    raise RuntimeError("找不到 tshark.exe，检查 Wireshark 安装路径，需要的话把路径加进 TSHARK_CANDIDATES")


def extract_packets(pcap_path, tmin=None, tmax=None) -> list[str]:
    tshark = find_tshark()
    display_filter = "usb.endpoint_address==0x03 && usb.transfer_type==0x01 && usbhid.data"
    if tmin is not None:
        display_filter += f" && frame.time_relative>={tmin}"
    if tmax is not None:
        display_filter += f" && frame.time_relative<={tmax}"

    cmd = [tshark, "-r", str(pcap_path), "-Y", display_filter, "-T", "fields", "-e", "usbhid.data"]
    out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
    lines = [l.strip() for l in out.splitlines() if l.strip()]

    # 连续重复的包(原厂软件的重试/确认)只保留一份
    packets = []
    for l in lines:
        if not packets or packets[-1] != l:
            packets.append(l)
    return packets


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)

    pcap_path = sys.argv[1]
    template_name = sys.argv[2]
    templates_path = Path(sys.argv[3]) if len(sys.argv) > 3 else Path(__file__).parent / "templates.yaml"
    tmin = float(sys.argv[4]) if len(sys.argv) > 4 else None
    tmax = float(sys.argv[5]) if len(sys.argv) > 5 else None

    packets = extract_packets(pcap_path, tmin, tmax)
    if not packets:
        print("没有提取到任何包，检查抓包文件/时间范围是否正确")
        sys.exit(1)
    print(f"提取到 {len(packets)} 个有效包（已去除连续重复）")
    for p in packets:
        print(" ", p)

    templates = {}
    if templates_path.exists():
        with open(templates_path, "r", encoding="utf-8") as f:
            templates = yaml.safe_load(f) or {}

    templates[template_name] = packets

    with open(templates_path, "w", encoding="utf-8") as f:
        yaml.safe_dump(templates, f, allow_unicode=True, sort_keys=False)
    print(f"\n已写入模板 '{template_name}' 到 {templates_path}")
    print(f"去 config.yaml 里用 template: {template_name} 引用它")


if __name__ == "__main__":
    main()
