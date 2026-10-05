"""
从语义配置(yaml 里的一个方案)构建方案: 解析并校验 polling_rate / light / dpi / performance / buttons。

不认识的键、越界的值一律报错, 不悄悄忽略; 合法但不起作用的设置(如当前灯光模式用不到的参数)给出警告。
"""

from __future__ import annotations

import warnings

from inphic.profile.commands import build_apply_sequence
from inphic.profile.model import (
    BUTTON_COUNT, BUTTON_NAMES, DPI_LEVELS, DPI_MAX, DPI_MIN, DPI_STEP, LIGHT_MODE_NAMES, LIGHT_MODE_PARAMS,
    LIGHT_MODES, POLLING_RATES, HubProfile,
)
from inphic.protocol.functions import MEDIA_ACTIONS, MODIFIER_BITS, MOUSE_ACTIONS, SHORTCUT_ACTIONS
from inphic.protocol.keycodes import key_to_code
from inphic.protocol.macro import START_MODES, parse_steps


def set_button_action(profile: HubProfile, i: int, action: dict, label: str = "", library=None) -> None:
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
        profile.set_button_key(i, key_to_code(action["key"]), mask)
    elif kind in ("mouse", "shortcut", "media"):
        table = {"mouse": MOUSE_ACTIONS, "shortcut": SHORTCUT_ACTIONS, "media": MEDIA_ACTIONS}[kind]
        name = action.get("action")
        if name not in table:
            raise ValueError(f"按钮 {label or i} 的 {kind} 动作 {name!r} 不认识, 可选: {sorted(table)}")
        profile.set_button_function(i, table[name])
    elif kind == "function":
        profile.set_button_function(i, int(action["id"]))
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
        profile.set_button_macro(i, events, repeat, START_MODES[start], name)
    else:
        raise ValueError(
            f"按钮 {label or i} 的 type {kind!r} 不支持, 可选: key / mouse / shortcut / media / function / macro"
        )


def apply_config(profile: HubProfile, cfg: dict, library=None) -> None:
    """把 config.yaml 里一个方案的语义配置套到本方案上; 不认识的键直接报错, 不悄悄忽略。"""
    known = {"polling_rate", "light", "dpi", "buttons", "performance"}
    unknown = set(cfg) - known
    if unknown:
        raise ValueError(f"方案里有不认识的配置项: {sorted(unknown)}, 可选: {sorted(known)}")

    if "polling_rate" in cfg:
        hz = int(cfg["polling_rate"])
        by_hz = {v[0]: k for k, v in POLLING_RATES.items()}
        if hz not in by_hz:
            raise ValueError(f"回报率 {hz} 不支持, 可选: {sorted(by_hz)}")
        profile.set_i32(0x2878, by_hz[hz])

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
        profile.set_i32(0x288C, m)
    if "rgb" in light:
        r, g, b = light["rgb"]
        for v in (r, g, b):
            if not 0 <= v <= 255:
                raise ValueError(f"rgb 分量 {v} 超出 0..255")
        profile.raw[0x28A0:0x28A3] = bytes((r, g, b))
    for key, off in (("brightness", 0x2864), ("speed", 0x2868)):
        if key in light:
            v = int(light[key])
            if not 1 <= v <= 8:
                raise ValueError(f"light.{key} = {v} 超出 GUI 滑条范围 1..8")
            profile.set_i32(off, v)
    # 配了当前模式用不到的参数: 合法但不起作用, 警告而不是悄悄忽略
    used = LIGHT_MODE_PARAMS[profile.light_mode]
    for key, param in (("rgb", "color"), ("brightness", "brightness"), ("speed", "speed")):
        if key in light and param not in used:
            warnings.warn(
                f"灯光模式 {LIGHT_MODES[profile.light_mode]!r} 不使用 {key}, 该设置不会起作用",
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
            profile.set_i32(0x2804 + 4 * i, v // DPI_STEP)
    if "enabled" in dpi:
        if len(dpi["enabled"]) != DPI_LEVELS:
            raise ValueError(f"dpi.enabled 需要正好 {DPI_LEVELS} 个布尔值")
        for i, on in enumerate(dpi["enabled"]):
            profile.set_i32(0x2844 + 4 * i, 1 if on else 0)
    if "current" in dpi:
        cur = int(dpi["current"])
        if not 1 <= cur <= DPI_LEVELS:
            raise ValueError(f"dpi.current = {cur} 超出 1..{DPI_LEVELS}")
        profile.set_i32(0x2874, cur - 1)
    if "colors" in dpi:
        if len(dpi["colors"]) != DPI_LEVELS:
            raise ValueError(f"dpi.colors 需要正好 {DPI_LEVELS} 个 [r, g, b]")
        for i, rgb in enumerate(dpi["colors"]):
            r, g, b = rgb
            if not all(0 <= v <= 255 for v in (r, g, b)):
                raise ValueError(f"dpi.colors[{i}] = {rgb} 超出 0..255")
            profile.raw[0x28A4 + 4 * i:0x28A7 + 4 * i] = bytes((r, g, b))
    if dpi and not profile.dpi_enabled(profile.dpi_current):
        # GUI 不允许取消勾选当前档位, 这里同样拒绝, 否则鼠标会停在一个被禁用的档位上
        raise ValueError(f"当前档位 {profile.dpi_current + 1} 被禁用: 请在 dpi.enabled 里启用它, 或改 dpi.current")

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
        profile.set_i32(0x2870, int(v))
    if "deep_sleep_minutes" in perf:
        v = perf["deep_sleep_minutes"]
        if not isinstance(v, int) or not 1 <= v <= 60:
            raise ValueError(f"performance.deep_sleep_minutes = {v} 不合法: 1..60 的整数")
        profile.set_i32(0x286C, v)
    if "debounce_ms" in perf:
        # 存档值 = 毫秒 / 2 (GUI 滑条 2..25 -> 4..50 ms)
        v = perf["debounce_ms"]
        if not isinstance(v, int) or v % 2 or not 4 <= v <= 50:
            raise ValueError(f"performance.debounce_ms = {v} 不合法: 4..50 之间的偶数")
        profile.set_i32(0x2898, v // 2)
    for key, off in (("ripple_control", 0x2880), ("angle_snap", 0x2884), ("motion_sync", 0x2888)):
        if key in perf:
            if not isinstance(perf[key], bool):
                raise ValueError(f"performance.{key} 需要 true / false")
            profile.set_i32(off, 1 if perf[key] else 0)

    for name, action in (cfg.get("buttons") or {}).items():
        idx = BUTTON_NAMES.get(name, name)
        if not isinstance(idx, int) or not 0 <= idx < BUTTON_COUNT:
            raise ValueError(f"按钮 {name!r} 不认识, 可用名字 {sorted(BUTTON_NAMES)} 或序号 0..{BUTTON_COUNT - 1}")
        set_button_action(profile, idx, action, label=str(name), library=library)


class ConfiguredProfile:
    """配置里的一个具名方案: 给运行时用的最小接口(build_packets / current_buttons)。"""

    def __init__(self, name: str, profile: HubProfile):
        self.name = name
        self.profile = profile

    def build_packets(self) -> list[bytes]:
        return build_apply_sequence(self.profile)

    def current_buttons(self) -> dict[str, str]:
        return {name: self.profile.describe_button(idx) for name, idx in BUTTON_NAMES.items()}


def from_config(name: str, cfg: dict, library=None) -> ConfiguredProfile:
    p = HubProfile.factory_default()
    apply_config(p, cfg, library)
    return ConfiguredProfile(name, p)
