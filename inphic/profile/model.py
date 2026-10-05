"""
方案模型: HUB 配置存档(Data/ms_N/pro.data)里一个方案的内存结构, 以及字段读写、按钮绑定。

不含命令构造(见 commands.py)和 yaml 配置解析(见 build.py)。

pro.data 布局: 4 字节头 + 7 个方案, 每个方案 PROFILE_SIZE 字节(4 + 7*0x1f624 == 899840)。
本机的"配置1"(默认)是第 5 个, "CF"是第 6 个; 其余槽位全 0。

下面所有 `+0xNNNN` 都是方案结构体内的偏移, 来自 Ghidra 对 INPHIC.exe(MS_3 页面)的反编译;
字段含义里"推断"二字表示是按偏移和取值猜的名字, 没有逐个核对过 GUI 控件。
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from inphic.profile.defaults import DEFAULT_FIELDS
from inphic.protocol.functions import (
    CATEGORY_KEYBOARD, CATEGORY_MACRO, FUNCTION_WIRE, MEDIA_ACTIONS, MODIFIER_BITS, MOUSE_ACTIONS,
    SHORTCUT_ACTIONS, category_of,
)
from inphic.protocol.keycodes import code_to_key
from inphic.protocol.macro import SLOT_BASE, START_MODES, Macro, macro_frames

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
        """把按钮 i 绑定为宏。只有按钮 0、1、2、3、6、7 有宏槽位(见 protocol.macro.SLOT_BASE)。"""
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

    @staticmethod
    def factory_default() -> "HubProfile":
        """工厂默认方案: 全 0 的结构体 + defaults.DEFAULT_FIELDS。"""
        raw = bytearray(PROFILE_SIZE)
        for off, data in DEFAULT_FIELDS.items():
            raw[off:off + len(data)] = data
        return HubProfile(raw)


def load_pro_data(path: str | Path) -> list[HubProfile]:
    data = Path(path).read_bytes()
    expected = HEADER_SIZE + PROFILE_COUNT * PROFILE_SIZE
    if len(data) != expected:
        raise ValueError(f"pro.data 大小 {len(data)} != 预期 {expected}, 格式可能随版本变了")
    return [
        HubProfile(bytearray(data[HEADER_SIZE + k * PROFILE_SIZE:HEADER_SIZE + (k + 1) * PROFILE_SIZE]))
        for k in range(PROFILE_COUNT)
    ]
