"""
查看宏库(只读, 不碰设备)。

    uv run python macro_tool.py list [config.hub.yaml]
    uv run python macro_tool.py show fps.rapid_fire [config.hub.yaml]

show 会展开 use/锚点, 打印最终步骤、事件数、要占几个槽位、大约播放多久。
"""

import sys
from pathlib import Path

import yaml

from hub_macro import EVENT_LIMIT, encode_macro, parse_steps
from macro_library import build_library


def load(config_arg):
    path = Path(config_arg) if config_arg else Path(__file__).parent / "config.hub.yaml"
    raw = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    if not raw.get("macro_files"):
        raise SystemExit(f"{path} 里没有 macro_files, 没有宏库可看")
    return build_library(raw["macro_files"], path.parent)


def main(argv):
    if not argv or argv[0] not in ("list", "show"):
        print(__doc__)
        return 1
    if argv[0] == "list":
        lib = load(argv[1] if len(argv) > 1 else None)
        for group, names in lib.groups().items():
            print(f"[{group}]")
            for n in names:
                d = lib.resolve(f"{group}.{n}")
                print(f"  {group}.{n:<24} {d.start:<17} {len(d.steps):>3} 步  {d.description}")
        return 0
    if len(argv) < 2:
        print("用法: macro_tool.py show <分组.名字> [配置文件]")
        return 1
    lib = load(argv[2] if len(argv) > 2 else None)
    d = lib.resolve(argv[1])
    events = parse_steps(list(d.steps))
    msgs = encode_macro(events, first_slot=1, repeat=d.repeat, mode={"loop": 0, "stop_on_any_key": 1, "hold": 2, "stop_on_this_key": 3}[d.start])
    total_ms = sum(e.delay_ms for e in events)
    print(f"{d.name}  ({d.source})")
    if d.description:
        print(f"  说明: {d.description}")
    print(f"  启动方式: {d.start}   循环次数: {d.repeat}")
    print(f"  展开后 {len(d.steps)} 步 -> {len(events)} 个事件, 占 {len(msgs)}/5 个槽位 (单条最多约 {EVENT_LIMIT} 字节事件)")
    print(f"  一遍大约 {total_ms} ms")
    for i, s in enumerate(d.steps, 1):
        print(f"  {i:>3}. {s}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
