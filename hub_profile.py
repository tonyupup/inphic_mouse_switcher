"""
INPHIC HUB 配置存档(Data/ms_N/pro.data)的解析, 以及把一个方案转成设备命令负载的构造器。

全部来自 Ghidra 对 INPHIC.exe(MS_3 页面)的反编译, 并用真实抓包逐字节验证过:
    FUN_00461760 -> 命令 0x04 (DPI 表)       FUN_00461ac0 -> 命令 0x05 (灯光)
    FUN_00461be0 -> 命令 0x06 (回报率)       FUN_004612a0 内联 -> 命令 0x0c / 0x07
    FUN_004620f0 -> 命令 0x08 (按键表)

pro.data 布局: 4 字节头 + 7 个方案, 每个方案 PROFILE_SIZE 字节(4 + 7*0x1f624 == 899840)。
本机的"配置1"(默认)是第 5 个, "CF"是第 6 个; 其余槽位全 0。

下面所有 `+0xNNNN` 都是方案结构体内的偏移, 字段含义里"推断"二字表示是按偏移和取值猜的名字,
没有逐个核对过 GUI 控件。
"""

from __future__ import annotations

import warnings
from dataclasses import dataclass, field
from pathlib import Path

from hid_keycodes import code_to_key, key_to_code
from hub_defaults import DEFAULT_FIELDS
from hub_functions import (
    CATEGORY_KEYBOARD, CATEGORY_MACRO, FUNCTION_WIRE, MEDIA_ACTIONS, MODIFIER_BITS, MOUSE_ACTIONS, SHORTCUT_ACTIONS, category_of,
)
from hub_macro import SLOT_BASE, START_MODES, Macro, MacroEvent, macro_frames, parse_steps
from mouse_protocol import build_frame

HEADER_SIZE = 4
PROFILE_SIZE = 0x1F624
PROFILE_COUNT = 7

DPI_LEVELS = 8
# GUI 里显示的 DPI = 存档值 * 50, 输入限制 50..26000 (FUN_004561c0: 值/0x32 存回, 滑条 1..520)
DPI_STEP = 50
DPI_MIN = 50
DPI_MAX = 26000
BUTTON_COUNT = 18
BUTTON_STRIDE = 0x1338
BUTTON_BASE = 0x28E4

# 灯光模式(GUI 下拉序号, 来自 res/lan/CN.xml 的 ms_lightmodeN)
LIGHT_MODES = {
    0: "关闭", 1: "常亮", 2: "呼吸", 3: "霓虹", 4: "循环呼吸", 5: "常亮DPI", 6: "呼吸DPI",
}

# 每种灯光模式在 GUI 里会显示哪些参数控件(FUN_004333b0: 先全部隐藏, 再按模式显示)。
# brightness = 亮度滑条(1..8, 方案 +0x2864), speed = 速度滑条(1..8, +0x2868), color = RGB(+0x28a0)。
# 与 cmd_light 的组包一致: 只有模式 1、5 才会把亮度写进灯光包, 其余模式亮度位固定为 8。
# 模式 8..11 本机界面里隐藏, 这里不收录。
LIGHT_MODE_PARAMS = {
    0: frozenset(),                                  # 关闭
    1: frozenset({"brightness", "color"}),           # 常亮
    2: frozenset({"speed", "color"}),                # 呼吸
    3: frozenset({"speed"}),                         # 霓虹
    4: frozenset({"speed"}),                         # 循环呼吸
    5: frozenset({"brightness"}),                    # 常亮DPI (颜色跟随当前 DPI 档位的颜色)
    6: frozenset({"speed"}),                         # 呼吸DPI (颜色跟随当前 DPI 档位的颜色)
}

# 回报率: GUI 序号 -> (Hz, 线上码)。线上码 C 以 C, ~C 两个字节发出
POLLING_RATES = {
    0: (125, 0x20), 1: (250, 0x10), 2: (500, 0x08), 3: (1000, 0x04),
    4: (2000, 0x02), 5: (4000, 0x01), 6: (8000, 0x40),
}

# 配置文件里的按钮名 -> 按钮序号。只有这三个的物理对应关系是实测确认过的; 其余用数字序号 0..17
BUTTON_NAMES = {"middle_click": 2, "side_back": 6, "side_front": 7}

# 灯光模式名(配置里可写名字或序号)
LIGHT_MODE_NAMES = {
    "off": 0, "static": 1, "breathing": 2, "neon": 3, "spectrum_cycle": 4,
    "dpi_static": 5, "dpi_breathing": 6,
}

KEY_FUNCTION_ID = 0x10          # 功能 id: 模拟一次键盘按键(线上类型 0x11)
MACRO_FUNCTION_ID = 0x11        # 功能 id: 宏(线上类型 0x12, 参数是槽位); 抓包验证
MACRO_VARIANT_ID = 0x18         # "左键按下播放"变体(线上类型 0x3a): 没有抓包, 不支持
MACRO_FUNCTION_IDS = (MACRO_FUNCTION_ID, MACRO_VARIANT_ID)


@dataclass
class HubProfile:
    raw: bytearray
    # 按钮序号 -> Macro(事件, 重复次数, 启动方式)。宏内容在 GUI 里存在别处(macro.data), 这里只保存要上传的那份
    macros: dict = field(default_factory=dict)

    # --- 低层读写 ---
    def u8(self, off: int) -> int:
        return self.raw[off]

    def i32(self, off: int) -> int:
        return int.from_bytes(self.raw[off:off + 4], "little", signed=True)

    def set_u8(self, off: int, v: int) -> None:
        self.raw[off] = v & 0xFF

    def set_i32(self, off: int, v: int) -> None:
        self.raw[off:off + 4] = int(v).to_bytes(4, "little", signed=True)

    @property
    def name(self) -> str:
        return bytes(self.raw[:64]).decode("utf-16-le", errors="replace").split("\0")[0]

    # --- 语义字段(名字为推断) ---
    # DPI 档位: 存的是 DPI/25 (16 -> 400, 130 -> 3250), 设备收到的是 值-1
    def dpi_value(self, i: int) -> int:
        return self.i32(0x2804 + 4 * i)

    def dpi(self, i: int) -> int:
        """第 i 档(0 起算)的实际 DPI。"""
        return self.dpi_value(i) * DPI_STEP

    @property
    def dpi_current(self) -> int:
        """当前选中的档位(0 起算), 对应 GUI 里点选的那一档。"""
        return self.i32(0x2874)

    def dpi_enabled(self, i: int) -> bool:
        return self.i32(0x2844 + 4 * i) == 1

    def dpi_color(self, i: int) -> tuple[int, int, int]:
        o = 0x28A4 + 4 * i
        return self.raw[o], self.raw[o + 1], self.raw[o + 2]

    @property
    def light_mode(self) -> int:
        return self.i32(0x288C)

    @property
    def polling_rate_index(self) -> int:
        return self.i32(0x2878)

    @property
    def sleep_minutes(self) -> float:
        return self.i32(0x2870) * 0.5

    @property
    def deep_sleep_minutes(self) -> int:
        return self.i32(0x286C)

    @property
    def debounce_ms(self) -> int:
        return self.i32(0x2898) * 2

    @property
    def light_rgb(self) -> tuple[int, int, int]:
        return self.raw[0x28A0], self.raw[0x28A1], self.raw[0x28A2]

    # --- 按键 ---
    def button_record(self, i: int) -> memoryview:
        o = BUTTON_BASE + BUTTON_STRIDE * i
        return memoryview(self.raw)[o:o + 8]

    def button_function_id(self, i: int) -> int:
        return self.button_record(i)[1]

    def set_button_key(self, i: int, hid_code: int, modifier: int = 0) -> None:
        """把按钮 i 绑定为键盘按键(HID 键码) + 可选修饰键掩码(ctrl=1 shift=2 alt=4 win=8)。"""
        if not 0 <= modifier <= 0x0F:
            raise ValueError(f"修饰键掩码 {modifier:#x} 超出 0..0xf")
        o = BUTTON_BASE + BUTTON_STRIDE * i
        self.macros.pop(i, None)
        self.raw[o + 1] = KEY_FUNCTION_ID
        self.raw[o + 2] = CATEGORY_KEYBOARD
        self.raw[o + 3] = hid_code
        self.raw[o + 7] = modifier

    def set_button_function(self, i: int, function_id: int) -> None:
        """把按钮 i 绑定为表里的功能 id(鼠标功能/快捷键/多媒体等)。"""
        if function_id not in FUNCTION_WIRE:
            raise ValueError(f"未知功能 id {function_id:#x}")
        if function_id in MACRO_FUNCTION_IDS:
            raise ValueError("宏请用 type: macro 配置, 不能直接指定宏的功能 id")
        o = BUTTON_BASE + BUTTON_STRIDE * i
        self.macros.pop(i, None)
        self.raw[o + 1] = function_id
        self.raw[o + 2] = category_of(function_id)
        self.raw[o + 3] = 0
        self.raw[o + 7] = 0

    def set_button_macro(self, i: int, events: list, repeat: int = 1, mode: int = 0, name: str = "") -> None:
        """把按钮 i 绑定为宏。只有按钮 0、1、2、3、6、7 有宏槽位(见 hub_macro.SLOT_BASE)。"""
        if i not in SLOT_BASE:
            raise ValueError(f"按钮 {i} 没有宏槽位, 只有 {sorted(SLOT_BASE)} 可以绑宏")
        macro_frames(events, SLOT_BASE[i], repeat, mode)      # 先试编码一遍, 尽早报错(太长、延迟不合法等)
        o = BUTTON_BASE + BUTTON_STRIDE * i
        self.raw[o + 1] = MACRO_FUNCTION_ID
        self.raw[o + 2] = CATEGORY_MACRO
        self.raw[o + 3] = 0
        self.raw[o + 7] = 0
        self.macros[i] = Macro(tuple(events), repeat, mode, name)

    def describe_button(self, i: int) -> str:
        """按钮当前绑定的可读描述, 用于日志。"""
        rec = self.button_record(i)
        fid = rec[1]
        if fid == KEY_FUNCTION_ID:
            mods = [n for n, bit in MODIFIER_BITS.items() if rec[7] & bit]
            return "+".join(mods + [code_to_key(rec[3])])
        if fid == MACRO_FUNCTION_ID and i in self.macros:
            mc = self.macros[i]
            mode = next(n for n, v in START_MODES.items() if v == mc.mode)
            tag = f"{mc.name}: " if mc.name else ""
            return f"macro({tag}{len(mc.events)} 个事件, {mode}" + (f", 重复 {mc.repeat} 次)" if mc.mode == 0 else ")")
        if fid in MACRO_FUNCTION_IDS:
            return f"macro({fid:#x}, 内容未定义)"
        for table in (MOUSE_ACTIONS, SHORTCUT_ACTIONS, MEDIA_ACTIONS):
            for name, v in table.items():
                if v == fid:
                    return name
        return f"function {fid:#x}"

    def set_button_action(self, i: int, action: dict, label: str = "", library=None) -> None:
        """配置里一个按钮的 action 字典 -> 写入记录。type: key / mouse / shortcut / media / function。"""
        kind = action.get("type")
        allowed = {"type", "key", "modifiers", "action", "id", "steps", "repeat", "default_delay_ms", "start", "ref"}
        extra = set(action) - allowed
        if extra:
            raise ValueError(f"按钮 {label or i} 里有不认识的键: {sorted(extra)}")
        if kind == "key":
            mods = action.get("modifiers") or []
            bad = [m for m in mods if m not in MODIFIER_BITS]
            if bad:
                raise ValueError(f"按钮 {label or i} 的修饰键 {bad} 不认识, 可选: {sorted(MODIFIER_BITS)}")
            mask = 0
            for m in mods:
                mask |= MODIFIER_BITS[m]
            self.set_button_key(i, key_to_code(action["key"]), mask)
        elif kind in ("mouse", "shortcut", "media"):
            table = {"mouse": MOUSE_ACTIONS, "shortcut": SHORTCUT_ACTIONS, "media": MEDIA_ACTIONS}[kind]
            name = action.get("action")
            if name not in table:
                raise ValueError(f"按钮 {label or i} 的 {kind} 动作 {name!r} 不认识, 可选: {sorted(table)}")
            self.set_button_function(i, table[name])
        elif kind == "function":
            self.set_button_function(i, int(action["id"]))
        elif kind == "macro":
            ref = action.get("ref")
            if ref is not None and "steps" in action:
                raise ValueError(f"按钮 {label or i}: ref 和 steps 只能写一个")
            if ref is not None:
                if library is None:
                    raise ValueError(f"按钮 {label or i}: 用了 ref {ref!r} 但没有加载宏库, 请在配置里写 macro_files")
                mdef = library.resolve(ref)
                steps, start, repeat, name = mdef.steps, mdef.start, mdef.repeat, mdef.name
                steps = list(steps)
            else:
                steps, start, repeat, name = action.get("steps"), "loop", 1, ""
            start = action.get("start", start)                 # 绑定处可以覆盖宏自己的设置
            repeat = action.get("repeat", repeat)
            if start not in START_MODES:
                raise ValueError(f"按钮 {label or i} 的宏启动方式 {start!r} 不认识, 可选: {sorted(START_MODES)}")
            if start != "loop" and repeat != 1:
                warnings.warn(f"按钮 {label or i}: repeat 只对 start: loop 有意义, 其他启动方式会被忽略", stacklevel=2)
            events = parse_steps(steps, action.get("default_delay_ms", 50))
            self.set_button_macro(i, events, repeat, START_MODES[start], name)
        else:
            raise ValueError(
                f"按钮 {label or i} 的 type {kind!r} 不支持, 可选: key / mouse / shortcut / media / function / macro"
            )

    @staticmethod
    def factory_default() -> "HubProfile":
        """工厂默认方案: 全 0 的结构体 + hub_defaults 里的字段。"""
        raw = bytearray(PROFILE_SIZE)
        for off, data in DEFAULT_FIELDS.items():
            raw[off:off + len(data)] = data
        return HubProfile(raw)

    def apply_config(self, cfg: dict, library=None) -> None:
        """把 config.yaml 里一个方案的语义配置套到本方案上; 不认识的键直接报错, 不悄悄忽略。"""
        known = {"polling_rate", "light", "dpi", "buttons", "performance", "template"}
        unknown = set(cfg) - known
        if unknown:
            raise ValueError(f"方案里有不认识的配置项: {sorted(unknown)}, 可选: {sorted(known - {'template'})}")

        if "polling_rate" in cfg:
            hz = int(cfg["polling_rate"])
            by_hz = {v[0]: k for k, v in POLLING_RATES.items()}
            if hz not in by_hz:
                raise ValueError(f"回报率 {hz} 不支持, 可选: {sorted(by_hz)}")
            self.set_i32(0x2878, by_hz[hz])

        light = cfg.get("light") or {}
        extra = set(light) - {"mode", "rgb", "brightness", "speed"}
        if extra:
            raise ValueError(f"light 里有不认识的键: {sorted(extra)}, 可选: mode, rgb, brightness, speed")
        if "mode" in light:
            m = light["mode"]
            if isinstance(m, str):
                if m not in LIGHT_MODE_NAMES:
                    raise ValueError(f"灯光模式 {m!r} 不认识, 可选: {sorted(LIGHT_MODE_NAMES)}")
                m = LIGHT_MODE_NAMES[m]
            if m not in LIGHT_MODES:
                raise ValueError(f"灯光模式序号 {m} 不在 0..6")
            self.set_i32(0x288C, m)
        if "rgb" in light:
            r, g, b = light["rgb"]
            for v in (r, g, b):
                if not 0 <= v <= 255:
                    raise ValueError(f"rgb 分量 {v} 超出 0..255")
            self.raw[0x28A0:0x28A3] = bytes((r, g, b))
        for key, off in (("brightness", 0x2864), ("speed", 0x2868)):
            if key in light:
                v = int(light[key])
                if not 1 <= v <= 8:
                    raise ValueError(f"light.{key} = {v} 超出 GUI 滑条范围 1..8")
                self.set_i32(off, v)
        # 配了当前模式用不到的参数: 合法但不起作用, 警告而不是悄悄忽略
        used = LIGHT_MODE_PARAMS[self.light_mode]
        for key, param in (("rgb", "color"), ("brightness", "brightness"), ("speed", "speed")):
            if key in light and param not in used:
                warnings.warn(
                    f"灯光模式 {LIGHT_MODES[self.light_mode]!r} 不使用 {key}, 该设置不会起作用",
                    stacklevel=2,
                )

        dpi = cfg.get("dpi") or {}
        extra = set(dpi) - {"levels", "enabled", "current", "colors"}
        if extra:
            raise ValueError(f"dpi 里有不认识的键: {sorted(extra)}, 可选: levels, enabled, current, colors")
        if "levels" in dpi:
            if len(dpi["levels"]) != DPI_LEVELS:
                raise ValueError(f"dpi.levels 需要正好 {DPI_LEVELS} 个值")
            for i, v in enumerate(dpi["levels"]):
                if not (DPI_MIN <= v <= DPI_MAX and v % DPI_STEP == 0):
                    raise ValueError(f"dpi 档位 {v} 不合法: 需要 {DPI_MIN}..{DPI_MAX} 之间 {DPI_STEP} 的整数倍")
                self.set_i32(0x2804 + 4 * i, v // DPI_STEP)
        if "enabled" in dpi:
            if len(dpi["enabled"]) != DPI_LEVELS:
                raise ValueError(f"dpi.enabled 需要正好 {DPI_LEVELS} 个布尔值")
            for i, on in enumerate(dpi["enabled"]):
                self.set_i32(0x2844 + 4 * i, 1 if on else 0)
        if "current" in dpi:
            cur = int(dpi["current"])
            if not 1 <= cur <= DPI_LEVELS:
                raise ValueError(f"dpi.current = {cur} 超出 1..{DPI_LEVELS}")
            self.set_i32(0x2874, cur - 1)
        if "colors" in dpi:
            if len(dpi["colors"]) != DPI_LEVELS:
                raise ValueError(f"dpi.colors 需要正好 {DPI_LEVELS} 个 [r, g, b]")
            for i, rgb in enumerate(dpi["colors"]):
                r, g, b = rgb
                if not all(0 <= v <= 255 for v in (r, g, b)):
                    raise ValueError(f"dpi.colors[{i}] = {rgb} 超出 0..255")
                self.raw[0x28A4 + 4 * i:0x28A7 + 4 * i] = bytes((r, g, b))
        if dpi and not self.dpi_enabled(self.dpi_current):
            # GUI 不允许取消勾选当前档位, 这里同样拒绝, 否则鼠标会停在一个被禁用的档位上
            raise ValueError(f"当前档位 {self.dpi_current + 1} 被禁用: 请在 dpi.enabled 里启用它, 或改 dpi.current")

        perf = cfg.get("performance") or {}
        extra = set(perf) - {"sleep_minutes", "deep_sleep_minutes", "debounce_ms",
                             "ripple_control", "angle_snap", "motion_sync"}
        if extra:
            raise ValueError(
                f"performance 里有不认识的键: {sorted(extra)}, 可选: sleep_minutes, deep_sleep_minutes, "
                "debounce_ms, ripple_control, angle_snap, motion_sync"
            )
        if "sleep_minutes" in perf:
            # 存档值 = 分钟 / 0.5 (GUI 滑条 1..60 -> 0.5..30 分钟)
            v = float(perf["sleep_minutes"]) * 2
            if v != int(v) or not 1 <= v <= 60:
                raise ValueError(f"performance.sleep_minutes = {perf['sleep_minutes']} 不合法: 0.5..30 之间 0.5 的整数倍")
            self.set_i32(0x2870, int(v))
        if "deep_sleep_minutes" in perf:
            v = perf["deep_sleep_minutes"]
            if not isinstance(v, int) or not 1 <= v <= 60:
                raise ValueError(f"performance.deep_sleep_minutes = {v} 不合法: 1..60 的整数")
            self.set_i32(0x286C, v)
        if "debounce_ms" in perf:
            # 存档值 = 毫秒 / 2 (GUI 滑条 2..25 -> 4..50 ms)
            v = perf["debounce_ms"]
            if not isinstance(v, int) or v % 2 or not 4 <= v <= 50:
                raise ValueError(f"performance.debounce_ms = {v} 不合法: 4..50 之间的偶数")
            self.set_i32(0x2898, v // 2)
        for key, off in (("ripple_control", 0x2880), ("angle_snap", 0x2884), ("motion_sync", 0x2888)):
            if key in perf:
                if not isinstance(perf[key], bool):
                    raise ValueError(f"performance.{key} 需要 true / false")
                self.set_i32(off, 1 if perf[key] else 0)

        for name, action in (cfg.get("buttons") or {}).items():
            idx = BUTTON_NAMES.get(name, name)
            if not isinstance(idx, int) or not 0 <= idx < BUTTON_COUNT:
                raise ValueError(f"按钮 {name!r} 不认识, 可用名字 {sorted(BUTTON_NAMES)} 或序号 0..{BUTTON_COUNT - 1}")
            self.set_button_action(idx, action, label=str(name), library=library)


def load_pro_data(path: str | Path) -> list[HubProfile]:
    data = Path(path).read_bytes()
    expected = HEADER_SIZE + PROFILE_COUNT * PROFILE_SIZE
    if len(data) != expected:
        raise ValueError(f"pro.data 大小 {len(data)} != 预期 {expected}, 格式可能随版本变了")
    return [
        HubProfile(bytearray(data[HEADER_SIZE + k * PROFILE_SIZE:HEADER_SIZE + (k + 1) * PROFILE_SIZE]))
        for k in range(PROFILE_COUNT)
    ]


def _seal16(payload: bytearray, start: int, end: int, at: int) -> None:
    s = sum(payload[start:end]) & 0xFFFF
    payload[at] = s >> 8
    payload[at + 1] = s & 0xFF


# ---------------- 命令构造器: 都返回 payload (payload[0]=命令, payload[1]=长度) ----------------

def cmd_begin() -> bytes:
    """命令 0x0c: 开始写入, 常量。"""
    return bytes([0x0C, 0x0A, 0x01, 0xFE, 0x01, 0xFE, 0, 0, 0, 0])


def cmd_polling_rate(p: HubProfile) -> bytes:
    """命令 0x06: 回报率。payload[3]=码, payload[4]=~码。"""
    idx = p.polling_rate_index
    if idx not in POLLING_RATES:
        raise ValueError(f"回报率序号 {idx} 不在 0..6")
    code = POLLING_RATES[idx][1]
    return bytes([0x06, 0x09, 0x01, code, (~code) & 0xFF, 0, 0, 0, 0])


def cmd_misc07(p: HubProfile) -> bytes:
    """命令 0x07: B 取自 +0x289c(含义未知, 抓包恒为 1)。"""
    b = p.u8(0x289C)
    return bytes([0x07, 0x08, b, (~b) & 0xFF, 0x00, 0xFF, 0, 0])


def cmd_light(p: HubProfile) -> bytes:
    """命令 0x05: 灯光。字段含义为按偏移和取值推断。"""
    mode = p.i32(0x288C)
    c = p.u8(0x286C)                       # 取 int 的低字节
    pl = bytearray(15)
    pl[0], pl[1], pl[2] = 0x05, 0x0F, 0x01
    pl[3] = (mode << 4) & 0xFF
    pl[4] = (c & 0xF0) | ((9 - p.u8(0x2868)) & 0xFF)
    pl[5] = ((c << 4) & 0xFF) | (p.u8(0x2864) if mode in (1, 5) else 8)
    pl[6], pl[7], pl[8] = p.light_rgb
    pl[9] = p.u8(0x2870)
    pl[10] = p.u8(0x2898)
    _seal16(pl, 3, 11, 11)
    return bytes(pl)


def cmd_dpi(p: HubProfile) -> bytes:
    """命令 0x04: DPI 表。8 档低字节/高字节分开放, 8 档 RGB, 末尾 16 位和。"""
    pl = bytearray(0x38)
    pl[0], pl[1], pl[2] = 0x04, 0x38, 0x01
    pl[3] = p.u8(0x2884)
    pl[4] = p.u8(0x2880)
    mask = 0
    for i in range(DPI_LEVELS):
        if p.dpi_enabled(i):
            mask |= 1 << i
    pl[5] = mask
    pl[6] = p.u8(0x2884)
    pl[7] = p.u8(0x2888)
    for i in range(DPI_LEVELS):
        v = p.dpi_value(i) - 1
        if v >= 0:
            pl[8 + i] = v & 0xFF
            pl[16 + i] = (v >> 8) & 0xFF
    pl[24] = (p.i32(0x2874) + 1) & 0xFF
    for i in range(DPI_LEVELS):
        pl[25 + 3 * i:28 + 3 * i] = bytes(p.dpi_color(i))
    pl[49] = 1
    _seal16(pl, 3, 50, 50)
    return bytes(pl)


def cmd_keymap(p: HubProfile) -> bytes:
    """命令 0x08: 按键表, 18 个按钮 x 3 字节, 末尾 payload[57:59] 为 payload[3:57] 的 16 位和(大端)。"""
    pl = bytearray(0x3B)
    pl[0], pl[1], pl[2] = 0x08, 0x3B, 0x01
    for i in range(BUTTON_COUNT):
        rec = p.button_record(i)
        fid = rec[1]
        if fid == MACRO_VARIANT_ID:
            raise NotImplementedError("宏的\"左键按下播放\"变体(功能 id 0x18)没有抓包, 不支持")
        b0, b1, b2 = FUNCTION_WIRE.get(fid, (0, 0, 0))
        if fid == MACRO_FUNCTION_ID:
            if i not in p.macros:
                raise ValueError(f"按钮 {i} 绑定了宏但没有宏内容, 请用 set_button_macro / type: macro")
            b1, b2 = 0, SLOT_BASE[i]                    # 线上: [0x12, 0x00, 槽位]  (抓包: 12 00 1a / 12 00 15)
        if fid == KEY_FUNCTION_ID:
            b1, b2 = rec[7], rec[3]
        pl[3 + 3 * i:6 + 3 * i] = bytes((b0, b1, b2))
    _seal16(pl, 3, 57, 57)
    return bytes(pl)


def build_apply_sequence(p: HubProfile) -> list[bytes]:
    """完整应用一个方案: 与 FUN_004612a0(which=0) 的发送顺序一致, 返回 64 字节帧列表。

    注意顺序: 0x0c, 0x04, 0x05, 0x06, 0x07, 0x08 —— 与抓包里的包序一致(包序在抓包里是
    0c,04,05,06,07,08)。宏数据(FUN_00461c90)接在最后。
    """
    payloads = [cmd_begin(), cmd_dpi(p), cmd_light(p), cmd_polling_rate(p), cmd_misc07(p), cmd_keymap(p)]
    frames = [build_frame(pl) for pl in payloads]
    # 宏数据放在最后, 按按钮序号(与 HUB 一致); 抓包里宏帧也是紧跟在按键表之后
    for i in sorted(p.macros):
        mc = p.macros[i]
        frames += macro_frames(list(mc.events), SLOT_BASE[i], mc.repeat, mc.mode)
    return frames


class ConfiguredProfile:
    """与 mouse_protocol.Profile 接口兼容(build_packets / current_buttons), 供 switcher.py 使用。"""

    def __init__(self, name: str, profile: HubProfile):
        self.name = name
        self.profile = profile

    def build_packets(self) -> list[bytes]:
        return build_apply_sequence(self.profile)

    def current_buttons(self) -> dict[str, str]:
        return {name: self.profile.describe_button(idx) for name, idx in BUTTON_NAMES.items()}


def from_config(name: str, cfg: dict, library=None) -> ConfiguredProfile:
    p = HubProfile.factory_default()
    p.apply_config(cfg, library)
    return ConfiguredProfile(name, p)
