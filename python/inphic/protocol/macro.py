"""
宏(快捷指令)的编码: 事件列表 -> 111 字节宏消息 -> 分片帧。

来自 Ghidra 对 INPHIC.exe 的 FUN_00461c90 / FUN_004606a0(0x09 分片路径)的反编译,
并用两份真实抓包(macro_key_A: 按下 A/延迟 50/抬起 A;  macro_key_B: 左键、右键、B 键各按下-延迟 50-抬起)
逐字节验证。验证过的部分: 键盘按键事件、鼠标左/右键事件、消息头、16 位校验、分片、按键表里的宏绑定。
**没有抓包验证**: 鼠标移动事件(0xF6)、延迟 >= 128ms 的扩展编码、多条消息续接(0xFE)、中/前进/后退键码(取自 GUI 代码)。

消息格式(M, 共 0x6f = 111 字节):
    M[0] = 0x09   M[1] = 0x6f   M[2] = 槽位
    M[3] = 启动方式(见 START_MODES; 抓包 macro_key_A/B、macro_2/3/4 验证)   M[4..6] = 恒 0
    M[7] = 重复次数(1..255, 只对"循环次数播放"有意义)   M[8] = 事件字节数/2
    M[9..]  = 事件, 最多 98 字节; 放不下时在末尾写 [槽位+1, 0xFE] 并续到下一个槽位
    M[109:111] = sum(M[3:109]) 的 16 位和, 大端
事件:
    按键: [标志|延迟, 键码]   标志 0x01=按下 0x81=抬起   (延迟 < 128ms, 取低 7 位; 标志与延迟按位或!)
          [标志|延迟%100, 键码, 延迟//100, 0x03]            (延迟 >= 128ms)
    移动: [延迟, 0xF6, x_hi, x_lo, y_hi, y_lo]               (延迟 >= 128ms 时末尾再加 [延迟//100, 0x03])
    键码: 键盘用 HID 键码; 鼠标按键用 0xF1 左 / 0xF2 右 / 0xF3 中 / 0xF4 后退 / 0xF5 前进
分片: 111 字节拆成 59 + 52 两片, 用 frame.build_chunk_frame(序号 0 和 1)。
"""

from __future__ import annotations

import warnings
from dataclasses import dataclass

from inphic.protocol.frame import build_chunk_frame
from inphic.protocol.keycodes import key_to_code

MESSAGE_LEN = 0x6F
EVENT_AREA = 9                 # M[9] 起是事件
EVENT_LIMIT = 0x62             # 单条消息事件字节上限(超过就续到下一槽位)
CHECKSUM_AT = 109
SLOTS_PER_BUTTON = 5

# 每个能绑宏的按钮的起始槽位(构造函数里的常量表 0x4ee940; 抓包里 btn7=26, btn6=21 与之吻合)。
# 其余按钮(4、5、8..17)没有槽位, 不能绑宏。
SLOT_BASE: dict[int, int] = {0: 1, 1: 6, 2: 11, 3: 16, 6: 21, 7: 26}

MOUSE_BUTTON_CODES: dict[str, int] = {
    "left": 0xF1, "right": 0xF2, "middle": 0xF3, "backward": 0xF4, "forward": 0xF5,
}
_VERIFIED_BUTTONS = {"left", "right"}

# 启动方式 = GUI 里 4 个单选项的序号(ms_3_option_repeat_mode_1..4), 处理函数把序号写进 rec+0xc 即 M[3]
START_MODES: dict[str, int] = {
    "loop": 0,               # 循环次数播放          (macro_key_A / B 抓包)
    "stop_on_any_key": 1,    # 任意键按下停止播放    (macro_2 抓包)
    "hold": 2,               # 按住播放, 松开停止    (macro_3 抓包)
    "stop_on_this_key": 3,   # 当前按键按下停止播放  (macro_4 抓包)
}

START_MODE_LABELS_CN: dict[str, str] = {      # HUB 语言包 select_macro_4/5/6/8_text
    "loop": "循环次数播放",
    "stop_on_any_key": "任意键按下停止播放",
    "hold": "按住播放，松开停止",
    "stop_on_this_key": "当前按键按下停止播放",
}

MOVE_OPCODE = 0xF6
EXT_DELAY_OPCODE = 0x03
CONTINUE_OPCODE = 0xFE
FLAG_DOWN = 0x01
FLAG_UP = 0x81

MAX_DELAY_MS = 25599           # 延迟//100 要放进一个字节


@dataclass(frozen=True)
class Macro:
    """一个要上传的宏: 事件、重复次数(仅 loop 模式有意义)、启动方式。"""
    events: tuple
    repeat: int = 1
    mode: int = START_MODES["loop"]
    name: str = ""             # 宏库里的 "分组.名字", 只用于日志


@dataclass(frozen=True)
class MacroEvent:
    kind: str                  # "down" | "up" | "move"
    code: int = 0              # 键码(move 不用)
    delay_ms: int = 0
    x: int = 0
    y: int = 0


def _check_delay(kind: str, d: int) -> None:
    if not isinstance(d, int) or isinstance(d, bool) or not 0 <= d <= MAX_DELAY_MS:
        raise ValueError(f"延迟 {d!r} 不合法: 需要 0..{MAX_DELAY_MS} 的整数毫秒")
    if kind == "move":
        return
    # 按键事件的第一个字节是 标志|延迟, 延迟的最低位会被标志位占掉, 所以奇数会被悄悄改成偶数
    low = d if d < 0x80 else d % 100
    if low % 2:
        raise ValueError(
            f"按键延迟 {d} ms 不合法: 编码里延迟最低位与标志位重合, 小于 128 的值需要是偶数, "
            "大于等于 128 的值其对 100 取余的部分需要是偶数"
        )


def encode_event(ev: MacroEvent) -> tuple[bytes, int]:
    """返回 (事件字节, 计数单位)。计数单位 = 字节数/2, 累加后写进 M[8]。"""
    _check_delay(ev.kind, ev.delay_ms)
    d = ev.delay_ms
    if ev.kind == "move":
        for v in (ev.x, ev.y):
            if not -32768 <= v <= 32767:
                raise ValueError(f"鼠标移动坐标 {v} 超出 16 位范围")
        xy = (ev.x & 0xFFFF).to_bytes(2, "big") + (ev.y & 0xFFFF).to_bytes(2, "big")
        if d < 0x80:
            return bytes([d or 1, MOVE_OPCODE]) + xy, 3
        return bytes([(d % 100) or 1, MOVE_OPCODE]) + xy + bytes([d // 100, EXT_DELAY_OPCODE]), 4
    flag = FLAG_DOWN if ev.kind == "down" else FLAG_UP
    if ev.kind not in ("down", "up"):
        raise ValueError(f"未知事件类型 {ev.kind!r}")
    if d < 0x80:
        return bytes([flag | (d or 1), ev.code]), 1
    return bytes([flag | ((d % 100) or 1), ev.code, d // 100, EXT_DELAY_OPCODE]), 2


def encode_macro(events: list[MacroEvent], first_slot: int, repeat: int = 1, mode: int = 0) -> list[bytes]:
    """事件列表 -> 一条或多条 111 字节宏消息(最多 SLOTS_PER_BUTTON 条)。"""
    if not events:
        raise ValueError("宏至少需要一个事件")
    if not 1 <= repeat <= 255:
        raise ValueError(f"重复次数 {repeat} 超出 1..255")
    if mode not in START_MODES.values():
        raise ValueError(f"启动方式 {mode!r} 不合法, 可选 {START_MODES}")
    messages: list[bytes] = []
    slot, start = first_slot, 0
    while True:
        if len(messages) >= SLOTS_PER_BUTTON:
            raise ValueError(f"宏太长: 一个按钮最多 {SLOTS_PER_BUTTON} 条消息(每条约 {EVENT_LIMIT} 字节事件)")
        m = bytearray(MESSAGE_LEN + 16)          # 多留一点, 溢出的事件字节先写进来再清掉
        m[0], m[1], m[2] = 0x09, MESSAGE_LEN, slot
        if start == 0:                      # 属性只在宏的第一条消息里
            m[3], m[7] = mode, repeat
        pos = count = 0
        i, more, next_start = start, False, 0
        while i < len(events):
            data, units = encode_event(events[i])
            new_pos = pos + len(data)
            m[EVENT_AREA + pos:EVENT_AREA + new_pos] = data
            count += units
            if new_pos > EVENT_LIMIT:
                # 放不下: 清掉这个事件, 在它的位置写续接标记 [slot+1, 0xFE](计数里占 1 个单位)
                m[EVENT_AREA + pos:EVENT_AREA + new_pos] = bytes(len(data))
                count -= units - 1
                m[EVENT_AREA + pos], m[EVENT_AREA + pos + 1] = slot + 1, CONTINUE_OPCODE
                more, next_start = True, i
                break
            pos = new_pos
            i += 1
        m[8] = count
        m[CHECKSUM_AT:CHECKSUM_AT + 2] = (sum(m[3:CHECKSUM_AT]) & 0xFFFF).to_bytes(2, "big")
        messages.append(bytes(m[:MESSAGE_LEN]))
        if not more:
            return messages
        slot, start = slot + 1, next_start


def message_frames(message: bytes) -> list[bytes]:
    """111 字节宏消息 -> 2 个 64 字节帧(59 + 52 字节, 分片序号 0 和 1)。"""
    if len(message) != MESSAGE_LEN:
        raise ValueError(f"宏消息需要 {MESSAGE_LEN} 字节, 实际 {len(message)}")
    return [build_chunk_frame(message[:59], 0), build_chunk_frame(message[59:], 1)]


def macro_frames(events: list[MacroEvent], first_slot: int, repeat: int = 1, mode: int = 0) -> list[bytes]:
    return [f for m in encode_macro(events, first_slot, repeat, mode) for f in message_frames(m)]


# ---------------- 配置里的 steps -> 事件 ----------------

def _resolve_target(spec: str) -> int:
    """'a' / 'left_shift' -> 键盘 HID 键码; 'mouse_left' 等 -> 0xF1.. 鼠标按键码。"""
    if spec.startswith("mouse_"):
        name = spec[len("mouse_"):]
        if name not in MOUSE_BUTTON_CODES:
            raise ValueError(f"鼠标按键 {spec!r} 不认识, 可选: " + ", ".join("mouse_" + n for n in MOUSE_BUTTON_CODES))
        if name not in _VERIFIED_BUTTONS:
            warnings.warn(
                f"宏里的 {spec} 键码 {MOUSE_BUTTON_CODES[name]:#x} 取自 GUI 代码, 没有抓包验证", stacklevel=3
            )
        return MOUSE_BUTTON_CODES[name]
    return key_to_code(spec)


def parse_steps(steps: list[dict], default_delay_ms: int = 50) -> list[MacroEvent]:
    """steps 里每一项恰好有一个动作键:  down / up / tap / move,  可选 delay_ms。

        - { tap: "a", delay_ms: 50 }          # 按下再抬起, 两个事件各带这个延迟(和 HUB 的"默认延迟"一致)
        - { down: "left_shift" }  /  { up: "left_shift" }
        - { tap: "mouse_left" }                # 鼠标按键
        - { move: [10, -5], delay_ms: 20 }     # 相对移动(没有抓包验证)
    """
    if not isinstance(steps, list) or not steps:
        raise ValueError("macro.steps 需要是非空列表")
    events: list[MacroEvent] = []
    for n, step in enumerate(steps, 1):
        if not isinstance(step, dict):
            raise ValueError(f"steps[{n}] 需要是字典")
        extra = set(step) - {"down", "up", "tap", "move", "delay_ms"}
        if extra:
            raise ValueError(f"steps[{n}] 里有不认识的键: {sorted(extra)}")
        actions = [k for k in ("down", "up", "tap", "move") if k in step]
        if len(actions) != 1:
            raise ValueError(f"steps[{n}] 需要恰好一个动作 (down / up / tap / move), 实际: {actions}")
        delay = step.get("delay_ms", default_delay_ms)
        action = actions[0]
        if action == "move":
            try:
                x, y = step["move"]
            except (TypeError, ValueError):
                raise ValueError(f"steps[{n}].move 需要 [x, y]") from None
            events.append(MacroEvent("move", 0, delay, int(x), int(y)))
            continue
        code = _resolve_target(step[action])
        if action in ("down", "tap"):
            events.append(MacroEvent("down", code, delay))
        if action in ("up", "tap"):
            events.append(MacroEvent("up", code, delay))
    for n, ev in enumerate(events, 1):          # 配置错误在解析时就报, 不等到编码
        try:
            _check_delay(ev.kind, ev.delay_ms)
        except ValueError as e:
            raise ValueError(f"steps 展开后第 {n} 个事件: {e}") from None
    return events
