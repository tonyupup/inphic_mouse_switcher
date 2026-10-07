"""用 HUB 存档(pro.data 的两个方案)生成命令, 与真实抓包逐字节对比。"""

import unittest
import warnings

from inphic.profile import (
    LIGHT_MODE_PARAMS, LIGHT_MODES, PROFILE_SIZE, HubProfile, build_apply_sequence, cmd_keymap, cmd_light, cmd_polling_rate, from_config,
)
from inphic.protocol.keycodes import key_to_code
from inphic.protocol.functions import MEDIA_ACTIONS, MOUSE_ACTIONS, SHORTCUT_ACTIONS
from inphic.protocol.frame import build_frame, parse_frame
from tests.support import profile_switch_frames, profiles_5_6

_blob = profiles_5_6()
DEFAULT = HubProfile(bytearray(_blob[:PROFILE_SIZE]))
CF = HubProfile(bytearray(_blob[PROFILE_SIZE:]))
CAPTURED = profile_switch_frames()


def _tpl(name):
    return CAPTURED[name]


class AgainstCaptures(unittest.TestCase):
    def test_names(self):
        self.assertEqual(DEFAULT.name, "配置1")
        self.assertEqual(CF.name, "CF")

    def test_default_profile_reproduces_all_six_frames(self):
        self.assertEqual(build_apply_sequence(DEFAULT), _tpl("default"))

    def test_cf_profile_reproduces_capture_when_light_mode_matches(self):
        # 抓包时 CF 方案的灯光模式是 5(常亮DPI), 存档里现在被改成 0(关闭)
        p = HubProfile(bytearray(CF.raw))
        p.set_i32(0x288C, 5)
        self.assertEqual(build_apply_sequence(p), _tpl("cf"))

    def test_cf_profile_now_differs_only_in_light_frame(self):
        got, cap = build_apply_sequence(CF), _tpl("cf")
        self.assertEqual([i for i in range(6) if got[i] != cap[i]], [2])

    def test_all_built_frames_are_valid(self):
        for f in build_apply_sequence(CF):
            parse_frame(f)


class Builders(unittest.TestCase):
    def test_polling_rate_codes(self):
        expect = {0: 0x20, 1: 0x10, 2: 0x08, 3: 0x04, 4: 0x02, 5: 0x01, 6: 0x40}
        for idx, code in expect.items():
            p = HubProfile(bytearray(DEFAULT.raw))
            p.set_i32(0x2878, idx)
            pl = cmd_polling_rate(p)
            self.assertEqual((pl[3], pl[4]), (code, (~code) & 0xFF))

    def test_light_mode_off_is_zero_high_nibble(self):
        p = HubProfile(bytearray(CF.raw))
        p.set_i32(0x288C, 0)
        self.assertEqual(cmd_light(p)[3], 0x00)

    def test_rebinding_updates_both_keymap_checksums(self):
        p = HubProfile(bytearray(CF.raw))
        p.set_button_key(7, key_to_code("f12"))
        frame = build_frame(cmd_keymap(p))
        parse_frame(frame)
        self.assertEqual(frame[3 + 3 + 3 * 7: 3 + 3 + 3 * 7 + 3], bytes((0x11, 0x00, 0x45)))
        self.assertEqual(int.from_bytes(frame[60:62], "big"), sum(frame[6:60]))

    def test_macro_variant_0x18_is_refused_not_guessed(self):
        p = HubProfile(bytearray(CF.raw))
        p.raw[0x28E4 + 0x1338 * 3 + 1] = 0x18
        with self.assertRaises(NotImplementedError):
            cmd_keymap(p)

    def test_macro_binding_without_content_is_refused(self):
        p = HubProfile(bytearray(CF.raw))
        p.raw[0x28E4 + 0x1338 * 3 + 1] = 0x11
        with self.assertRaises(ValueError):
            cmd_keymap(p)


class SemanticConfig(unittest.TestCase):
    CF_CFG = {
        "polling_rate": 1000,
        "light": {"mode": "off"},
        "buttons": {
            "middle_click": {"type": "key", "key": "1"},
            "side_back": {"type": "key", "key": "6"},
            "side_front": {"type": "key", "key": "5"},
        },
    }

    def test_config_reproduces_real_cf_profile(self):
        # 只靠 config 里的几项 + 工厂默认, 必须和 HUB 实际保存的 CF 方案生成同样的 6 个帧
        self.assertEqual(from_config("cf", self.CF_CFG).build_packets(), build_apply_sequence(CF))

    def test_empty_config_is_factory_default(self):
        self.assertEqual(from_config("d", {}).build_packets(), build_apply_sequence(DEFAULT))

    def test_bad_config_is_rejected_not_ignored(self):
        for bad in (
            {"polling_rat": 1000},
            {"polling_rate": 999},
            {"light": {"mode": "disco"}},
            {"light": {"rgb": [0, 256, 0]}},
            {"light": {"colour": 1}},
            {"light": {"brightness": 0}},
            {"light": {"brightness": 9}},
            {"light": {"speed": 9}},
            {"dpi": {"levels": [1, 2, 3]}},
            {"dpi": {"levels": [801, 1600, 3200, 6500, 13000, 26000, 500, 500]}},
            {"dpi": {"levels": [800, 1600, 3200, 6500, 13000, 26050, 500, 500]}},
            {"dpi": {"levels": [0, 1600, 3200, 6500, 13000, 26000, 500, 500]}},
            {"dpi": {"current": 0}},
            {"dpi": {"current": 9}},
            {"dpi": {"current": 7}},
            {"dpi": {"enabled": [True, False, True, True, True, True, False, False]}},   # 禁用了当前档(第 2 档)
            {"dpi": {"levels": []}},
            {"dpi": {"levels": [800] * 9}},
            {"dpi": {"current": 1700}},            # 启用的档里没有 1700
            {"dpi": {"colors": [[0, 0, 300]] * 8}},
            {"dpi": {"step": 1}},
            {"performance": {"sleep": 1}},
            {"performance": {"sleep_minutes": 0.3}},
            {"performance": {"sleep_minutes": 31}},
            {"performance": {"deep_sleep_minutes": 0}},
            {"performance": {"deep_sleep_minutes": 61}},
            {"performance": {"deep_sleep_minutes": 2.5}},
            {"performance": {"debounce_ms": 3}},
            {"performance": {"debounce_ms": 52}},
            {"performance": {"ripple_control": "yes"}},
            {"buttons": {"nonexistent": {"type": "key", "key": "a"}}},
            {"buttons": {"middle_click": {"type": "macro"}}},
            {"buttons": {99: {"type": "key", "key": "a"}}},
        ):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                from_config("x", bad)

    def test_light_mode_by_name_and_index_are_equal(self):
        a = from_config("a", {"light": {"mode": "breathing"}}).build_packets()
        b = from_config("b", {"light": {"mode": 2}}).build_packets()
        self.assertEqual(a, b)


class LightModeParams(unittest.TestCase):
    def _light(self, mode, **kw):
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return from_config("t", {"light": {"mode": mode, **kw}}).build_packets()[2]

    def test_table_covers_every_known_mode(self):
        self.assertEqual(set(LIGHT_MODE_PARAMS), set(LIGHT_MODES))

    def test_brightness_only_reaches_the_wire_for_modes_that_show_it(self):
        # GUI 里显示亮度滑条的模式, 恰好就是亮度会进入灯光包的模式
        for mode in LIGHT_MODES:
            changes = self._light(mode, brightness=1) != self._light(mode, brightness=8)
            self.assertEqual(changes, "brightness" in LIGHT_MODE_PARAMS[mode], f"mode {mode}")

    def test_speed_encoding_is_nine_minus_slider(self):
        for v in range(1, 9):
            pl = parse_frame(self._light(2, speed=v))
            self.assertEqual(pl[4] & 0x0F, 9 - v)

    def test_static_brightness_goes_to_low_nibble_of_byte5(self):
        for v in range(1, 9):
            self.assertEqual(parse_frame(self._light(1, brightness=v))[5] & 0x0F, v)

    def test_unused_parameter_warns_and_used_does_not(self):
        with self.assertWarns(UserWarning):
            from_config("t", {"light": {"mode": "neon", "rgb": [1, 2, 3]}})
        with warnings.catch_warnings():
            warnings.simplefilter("error")
            from_config("t", {"light": {"mode": "breathing", "rgb": [1, 2, 3], "speed": 4}})


class DpiConfig(unittest.TestCase):
    FACTORY_LEVELS = [800, 1600, 3200, 6500, 13000, 26000, 500, 500]

    def test_factory_dpi_values_are_stored_value_times_50(self):
        p = HubProfile.factory_default()
        self.assertEqual([p.dpi(i) for i in range(8)], self.FACTORY_LEVELS)
        self.assertEqual(p.dpi_current, 1)  # 第 2 档 = 1600 DPI

    def test_real_dpi_config_matches_factory_frames(self):
        got = from_config("x", {"dpi": {"levels": self.FACTORY_LEVELS, "current": 2}}).build_packets()
        self.assertEqual(got, build_apply_sequence(HubProfile.factory_default()))

    def test_wire_encoding_is_stored_value_minus_one(self):
        lv = list(self.FACTORY_LEVELS)
        lv[0] = 400                                   # 存档值 8 -> 线上 7
        pl = parse_frame(from_config("x", {"dpi": {"levels": lv}}).build_packets()[1])
        self.assertEqual(pl[8], 7)

    def test_short_levels_disable_the_rest(self):
        p = from_config("x", {"dpi": {"levels": [800, 1600]}}).profile
        self.assertEqual([p.dpi_enabled(i) for i in range(8)], [True, True] + [False] * 6)
        self.assertEqual((p.dpi(0), p.dpi(1), p.dpi_current), (800, 1600, 0))

    def test_current_accepts_dpi_value(self):
        p = from_config("x", {"dpi": {"levels": [800, 1600, 3200], "current": 1600}}).profile
        self.assertEqual(p.dpi_current, 1)
        p = from_config("x", {"dpi": {"levels": [800, 1600, 3200], "current": 3}}).profile     # 小数字仍是档位号
        self.assertEqual(p.dpi_current, 2)

    def test_scalar_dpi_shorthand(self):
        p = from_config("x", {"dpi": 1600}).profile
        self.assertEqual((p.dpi(0), p.dpi_current), (1600, 0))
        self.assertEqual([p.dpi_enabled(i) for i in range(8)], [True] + [False] * 7)

    def test_max_dpi_uses_both_bytes(self):
        pl = parse_frame(from_config("x", {"dpi": {"levels": self.FACTORY_LEVELS}}).build_packets()[1])
        self.assertEqual((pl[8 + 5], pl[16 + 5]), (0x07, 0x02))   # 520-1 = 0x207

    def test_disabling_a_non_current_stage_is_fine(self):
        from_config("x", {"dpi": {"enabled": [False, True, True, True, True, True, False, False]}})

    def test_current_stage_and_enable_mask(self):
        pl = parse_frame(from_config("x", {"dpi": {"current": 1}}).build_packets()[1])
        self.assertEqual(pl[24], 1)                    # 线上 = 档位序号(0 起算)+1
        pl = parse_frame(from_config("x", {"dpi": {"enabled": [True] * 8}}).build_packets()[1])
        self.assertEqual(pl[5], 0xFF)

    def test_stage_colors(self):
        cols = [[i, 255 - i, 7] for i in range(8)]
        pl = parse_frame(from_config("x", {"dpi": {"colors": cols}}).build_packets()[1])
        for i in range(8):
            self.assertEqual(list(pl[25 + 3 * i:28 + 3 * i]), cols[i])


class ButtonActions(unittest.TestCase):
    # HUB 自己的快捷键条目 = 哪个键 + 哪些修饰键 (名字取自 GUI 标签的常识含义)
    SHORTCUT_AS_KEYS = {
        "cut": ("x", ["ctrl"]), "copy": ("c", ["ctrl"]), "paste": ("v", ["ctrl"]),
        "open": ("o", ["ctrl"]), "save": ("s", ["ctrl"]), "find": ("f", ["ctrl"]),
        "redo": ("y", ["ctrl"]), "undo": ("z", ["ctrl"]), "select_all": ("a", ["ctrl"]),
        "print": ("p", ["ctrl"]), "close_window": ("f4", ["alt"]), "swap_windows": ("tab", ["alt"]),
        "show_desktop": ("d", ["win"]), "run_command": ("r", ["win"]), "lock_pc": ("l", ["win"]),
        "screen_capture": ("s", ["win", "shift"]),
    }

    def _wire(self, cfg):
        p = from_config("t", {"buttons": {0: cfg}}).profile
        pl = parse_frame(build_frame(cmd_keymap(p)))
        return bytes(pl[3:6])      # 按钮 0 的 3 字节

    def test_shortcut_table_matches_key_plus_modifiers(self):
        # 独立验证修饰键位掩码(ctrl=1 shift=2 alt=4 win=8)和键码: 两条路径必须发出相同字节
        self.assertEqual(set(self.SHORTCUT_AS_KEYS), set(SHORTCUT_ACTIONS))
        for name, (key, mods) in self.SHORTCUT_AS_KEYS.items():
            with self.subTest(shortcut=name):
                self.assertEqual(
                    self._wire({"type": "shortcut", "action": name}),
                    self._wire({"type": "key", "key": key, "modifiers": mods}),
                )

    def test_factory_default_buttons_have_expected_meaning(self):
        p = HubProfile.factory_default()
        self.assertEqual(p.describe_button(2), "wheel_click")
        self.assertEqual(p.describe_button(6), "backward")
        self.assertEqual(p.describe_button(7), "forward")

    def test_action_names_resolve_to_distinct_ids(self):
        for table in (MOUSE_ACTIONS, SHORTCUT_ACTIONS, MEDIA_ACTIONS):
            self.assertEqual(len(set(table.values())), len(table))

    def test_every_named_action_has_a_wire_code(self):
        for table in (MOUSE_ACTIONS, SHORTCUT_ACTIONS, MEDIA_ACTIONS):
            for name in table:
                self._wire({"type": {id(MOUSE_ACTIONS): "mouse", id(SHORTCUT_ACTIONS): "shortcut"}.get(id(table), "media"),
                            "action": name})

    def test_describe_roundtrip_for_keys(self):
        p = from_config("t", {"buttons": {"middle_click": {"type": "key", "key": "c", "modifiers": ["ctrl", "shift"]}}})
        self.assertEqual(p.current_buttons()["middle_click"], "ctrl+shift+c")

    def test_bad_action_configs(self):
        for bad in (
            {"type": "key", "key": "c", "modifiers": ["meta"]},
            {"type": "mouse", "action": "teleport"},
            {"type": "shortcut", "action": "left_click"},
            {"type": "media", "action": "copy"},
            {"type": "key", "key": "c", "typo": 1},
            {"type": "function", "id": 0x7777},
        ):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                from_config("t", {"buttons": {"middle_click": bad}})


class PerformanceSettings(unittest.TestCase):
    def _frames(self, **perf):
        return [parse_frame(f) for f in from_config("t", {"performance": perf}).build_packets()]

    def test_defaults_are_identity(self):
        got = from_config("t", {"performance": {
            "sleep_minutes": 1.0, "deep_sleep_minutes": 10, "debounce_ms": 8,
            "ripple_control": True, "angle_snap": False, "motion_sync": True,
        }}).build_packets()
        self.assertEqual(got, build_apply_sequence(HubProfile.factory_default()))

    def test_sleep_minutes_goes_to_light_byte9_in_half_minutes(self):
        light = self._frames(sleep_minutes=3.0)[2]
        self.assertEqual(light[9], 6)

    def test_deep_sleep_is_split_across_two_nibbles(self):
        light = self._frames(deep_sleep_minutes=0x1E)[2]      # 30 分钟
        self.assertEqual((light[4] & 0xF0, light[5] & 0xF0), (0x10, 0xE0))

    def test_debounce_is_half_of_ms(self):
        self.assertEqual(self._frames(debounce_ms=20)[2][10], 10)

    def test_three_switches_land_in_dpi_packet(self):
        dpi = self._frames(ripple_control=False, angle_snap=True, motion_sync=False)[1]
        self.assertEqual((dpi[3], dpi[4], dpi[6], dpi[7]), (1, 0, 1, 0))

    def test_extremes_are_accepted(self):
        self._frames(sleep_minutes=0.5, deep_sleep_minutes=60, debounce_ms=4)
        self._frames(sleep_minutes=30, deep_sleep_minutes=1, debounce_ms=50)


if __name__ == "__main__":
    unittest.main()


class ApplyPayloads(unittest.TestCase):
    """build_apply_payloads: 蓝牙下写进 FEE3 的原始载荷, 和 2.4G 的帧序列同源。"""

    def test_payload_order_and_polling_switch(self):
        from inphic.profile import build_apply_payloads, from_config

        p = from_config("t", {"polling_rate": 1000}).profile
        self.assertEqual([x[0] for x in build_apply_payloads(p)], [0x0C, 0x04, 0x05, 0x06, 0x07, 0x08])
        self.assertEqual(
            [x[0] for x in build_apply_payloads(p, include_polling=False)], [0x0C, 0x04, 0x05, 0x07, 0x08]
        )

    def test_frames_are_derived_from_payloads(self):
        from inphic.profile import build_apply_payloads, build_apply_sequence, from_config
        from inphic.protocol.frame import parse_frame

        p = from_config("t", {"light": {"mode": "static", "rgb": [1, 2, 3]}}).profile
        self.assertEqual([parse_frame(f) for f in build_apply_sequence(p)], build_apply_payloads(p))

    def test_macro_payload_is_one_111_byte_message(self):
        from inphic.profile import build_apply_payloads, build_apply_sequence, from_config

        cfg = {"buttons": {"side_front": {"type": "macro", "start": "loop", "steps": [{"tap": "a"}]}}}
        p = from_config("t", cfg).profile
        macro = [x for x in build_apply_payloads(p) if x[0] == 0x09]
        self.assertEqual([len(x) for x in macro], [111])                    # 蓝牙整条一次写
        self.assertEqual(len(build_apply_sequence(p)), 6 + 2)               # 2.4G 拆成两个分片帧
