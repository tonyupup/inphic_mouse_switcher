"""macros 子命令: 查看宏库(只读, 不碰设备)。show 会展开 use/锚点, 打印最终步骤、事件数、占几个槽位、大约播放多久。"""

from __future__ import annotations

from inphic.config import load_config
from inphic.protocol.macro import EVENT_LIMIT, START_MODES, encode_macro, parse_steps


def _library(config_path: str):
    lib = load_config(config_path).macro_library
    if lib is None:
        raise ValueError(f"{config_path} 里没有 macro_files, 没有宏库可看")
    return lib


def macros_list(config_path: str) -> int:
    lib = _library(config_path)
    for group, names in lib.groups().items():
        print(f"[{group}]")
        for n in names:
            d = lib.resolve(f"{group}.{n}")
            print(f"  {group}.{n:<24} {d.start:<17} {len(d.steps):>3} 步  {d.description}")
    return 0


def macros_show(config_path: str, ref: str) -> int:
    d = _library(config_path).resolve(ref)
    events = parse_steps(list(d.steps))
    msgs = encode_macro(events, first_slot=1, repeat=d.repeat, mode=START_MODES[d.start])
    print(f"{d.name}  ({d.source})")
    if d.description:
        print(f"  说明: {d.description}")
    print(f"  启动方式: {d.start}   循环次数: {d.repeat}")
    print(f"  展开后 {len(d.steps)} 步 -> {len(events)} 个事件, 占 {len(msgs)}/5 个槽位 (单条最多约 {EVENT_LIMIT} 字节事件)")
    print(f"  一遍大约 {sum(e.delay_ms for e in events)} ms")
    for i, s in enumerate(d.steps, 1):
        print(f"  {i:>3}. {s}")
    return 0
