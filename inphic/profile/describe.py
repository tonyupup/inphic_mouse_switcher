"""把一个方案用中文描述出来(启动时打印当前生效的设置)。纯文本生成, 不碰设备。

描述的是**实际生效值**: 配置里没写的项会显示出厂默认, 这样看到的就是鼠标最终会被设成什么样。
"""

from __future__ import annotations

from inphic.profile.model import (
    BUTTON_LABELS_CN, DPI_LEVELS, KEY_FUNCTION_ID, LIGHT_MODE_PARAMS, LIGHT_MODES, MACRO_FUNCTION_ID,
    MACRO_VARIANT_ID, POLLING_RATES, HubProfile,
)
from inphic.protocol.functions import ACTION_LABELS_CN, MEDIA_ACTIONS, MODIFIER_BITS, MOUSE_ACTIONS, SHORTCUT_ACTIONS
from inphic.protocol.keycodes import code_to_key
from inphic.protocol.macro import START_MODE_LABELS_CN, START_MODES

_MODIFIER_LABELS = {"ctrl": "Ctrl", "shift": "Shift", "alt": "Alt", "win": "Win"}


def _on_off(v) -> str:
    return "开" if v else "关"


def _num(v: float) -> str:
    return f"{v:g}"


def describe_function(p: HubProfile, i: int) -> str:
    """按钮 i 当前绑定的中文描述, 如"按键 Ctrl+C"、"复制"、"宏 fps.rapid_fire（按住播放，松开停止，2 个事件）"。"""
    rec = p.button_record(i)
    fid = rec[1]
    if fid == KEY_FUNCTION_ID:
        mods = [_MODIFIER_LABELS[n] for n, bit in MODIFIER_BITS.items() if rec[7] & bit]
        key = code_to_key(rec[3]).upper().replace("_", " ")
        return "按键 " + "+".join(mods + [key])
    if fid == MACRO_FUNCTION_ID and i in p.macros:
        mc = p.macros[i]
        mode_name = next(n for n, v in START_MODES.items() if v == mc.mode)
        extra = f"，循环 {mc.repeat} 次" if mc.mode == 0 else ""
        return f"宏 {mc.name or '(内联)'}（{START_MODE_LABELS_CN[mode_name]}{extra}，{len(mc.events)} 个事件）"
    if fid in (MACRO_FUNCTION_ID, MACRO_VARIANT_ID):
        return f"宏（功能 {fid:#04x}，内容未定义）"
    for table in (MOUSE_ACTIONS, SHORTCUT_ACTIONS, MEDIA_ACTIONS):
        for name, v in table.items():
            if v == fid:
                return ACTION_LABELS_CN[name]
    return f"功能 {fid:#04x}（出厂默认）"


def describe_light(p: HubProfile) -> str:
    mode = p.light_mode
    parts = [LIGHT_MODES[mode]]
    used = LIGHT_MODE_PARAMS[mode]
    if "brightness" in used:
        parts.append(f"亮度 {p.i32(0x2864)}")
    if "speed" in used:
        parts.append(f"速度 {p.i32(0x2868)}")
    if "color" in used:
        parts.append("颜色 RGB(%d, %d, %d)" % p.light_rgb)
    if mode in (5, 6):
        parts.append("颜色跟随当前 DPI 档")
    return " · ".join(parts)


def describe_dpi(p: HubProfile) -> str:
    cur = p.dpi_current
    stages = [
        f"{i + 1}档 {p.dpi(i)}" + ("（当前）" if i == cur else "")
        for i in range(DPI_LEVELS) if p.dpi_enabled(i)
    ]
    off = DPI_LEVELS - len(stages)
    tail = f" · 另有 {off} 档未启用" if off else ""
    return f"当前第 {cur + 1} 档 = {p.dpi(cur)} · " + " / ".join(stages) + tail


def describe_performance(p: HubProfile) -> str:
    return " · ".join([
        f"休眠 {_num(p.sleep_minutes)} 分钟",
        f"深度睡眠 {p.deep_sleep_minutes} 分钟",
        f"按键响应 {p.debounce_ms} 毫秒",
        f"波纹控制 {_on_off(p.i32(0x2880))}",
        f"直线修正 {_on_off(p.i32(0x2884))}",
        f"移动同步 {_on_off(p.i32(0x2888))}",
    ])


def describe_profile(p: HubProfile) -> list[str]:
    """一个方案的全部生效设置, 每项一行(按钮一个一行); 与出厂默认不同的按钮标 ★。"""
    factory = HubProfile.factory_default()
    lines = [
        f"回报率  {POLLING_RATES[p.polling_rate_index][0]} Hz",
        f"灯光    {describe_light(p)}",
        f"DPI     {describe_dpi(p)}",
        f"性能    {describe_performance(p)}",
    ]
    for n, i in enumerate(BUTTON_LABELS_CN):
        changed = i in p.macros or bytes(p.button_record(i)) != bytes(factory.button_record(i))
        head = "按钮    " if n == 0 else "        "
        lines.append(f"{head}{BUTTON_LABELS_CN[i]} → {describe_function(p, i)}" + ("  ★" if changed else ""))
    return lines
