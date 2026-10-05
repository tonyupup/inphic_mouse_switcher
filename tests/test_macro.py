"""宏: 用 macro_key_A / macro_key_B 两份真实抓包验证编码, 并检查按反编译逻辑实现的续接/校验。"""

import unittest
import warnings
from pathlib import Path

from hub_macro import (
    CONTINUE_OPCODE, MESSAGE_LEN, SLOT_BASE, START_MODES, MacroEvent, encode_event, encode_macro,
    macro_frames, message_frames, parse_steps,
)
from hub_profile import HubProfile, build_apply_sequence, from_config
from mouse_protocol import build_frame, parse_chunk_frame

CAP = {"A": [], "B": [], "M2": [], "M3": [], "M4": []}
for line in (Path(__file__).parent / "captured_macro_frames.txt").read_text().splitlines():
    if line and not line.startswith("#"):
        tag, hexs = line.split()
        CAP[tag].append(bytes.fromhex(hexs))

STEPS_A = [{"tap": "a", "delay_ms": 50}]
STEPS_B = [{"tap": "mouse_left"}, {"tap": "mouse_right"}, {"tap": "b"}]


class AgainstCaptures(unittest.TestCase):
    def test_capture_fixtures_present(self):
        self.assertEqual({k: len(v) for k, v in CAP.items()}, {"A": 4, "B": 4, "M2": 4, "M3": 4, "M4": 4})

    def test_macro_frames_A(self):
        self.assertEqual(macro_frames(parse_steps(STEPS_A), SLOT_BASE[7]), CAP["A"][2:4])

    def test_macro_frames_B(self):
        self.assertEqual(macro_frames(parse_steps(STEPS_B), SLOT_BASE[6]), CAP["B"][2:4])

    def test_whole_sequence_A(self):
        p = HubProfile.factory_default()
        p.set_button_macro(7, parse_steps(STEPS_A))
        seq = build_apply_sequence(p)
        self.assertEqual((seq[0], seq[5], seq[6:]), (CAP["A"][0], CAP["A"][1], CAP["A"][2:4]))

    def test_whole_sequence_B(self):
        p = HubProfile.factory_default()
        p.set_button_macro(6, parse_steps(STEPS_B))
        seq = build_apply_sequence(p)
        self.assertEqual((seq[0], seq[5], seq[6:]), (CAP["B"][0], CAP["B"][1], CAP["B"][2:4]))

    def test_captured_macro_frames_roundtrip(self):
        for tag in "AB":
            for f in CAP[tag][2:4]:
                idx, data = parse_chunk_frame(f)
                self.assertIn(idx, (0, 1))

    def test_slot_table_matches_captures(self):
        self.assertEqual((SLOT_BASE[7], SLOT_BASE[6]), (26, 21))
        self.assertEqual(set(SLOT_BASE), {0, 1, 2, 3, 6, 7})

    def test_via_config(self):
        cfg = {"buttons": {"side_front": {"type": "macro", "steps": STEPS_A}}}
        seq = from_config("t", cfg).build_packets()
        self.assertEqual(seq[5], CAP["A"][1])
        self.assertEqual(seq[6:], CAP["A"][2:4])


class StartModes(unittest.TestCase):
    """四种启动方式只改 M[3]; macro_key_A / macro_2 / macro_3 / macro_4 四份抓包逐字节验证。"""

    CASES = (("A", "loop", 0), ("M2", "stop_on_any_key", 1), ("M3", "hold", 2), ("M4", "stop_on_this_key", 3))

    def test_mode_values(self):
        self.assertEqual(START_MODES, {"loop": 0, "stop_on_any_key": 1, "hold": 2, "stop_on_this_key": 3})

    def test_each_mode_matches_its_capture(self):
        for tag, name, value in self.CASES:
            with self.subTest(mode=name):
                p = HubProfile.factory_default()
                p.set_button_macro(7, parse_steps(STEPS_A), 1, value)
                seq = build_apply_sequence(p)
                self.assertEqual((seq[0], seq[5], seq[6:]), (CAP[tag][0], CAP[tag][1], CAP[tag][2:4]))

    def test_each_mode_matches_its_capture_via_config(self):
        for tag, name, value in self.CASES:
            with self.subTest(mode=name):
                cfg = {"buttons": {"side_front": {"type": "macro", "steps": STEPS_A, "start": name}}}
                seq = from_config("t", cfg).build_packets()
                self.assertEqual(seq[6:], CAP[tag][2:4])

    def test_only_m3_differs_between_modes(self):
        base = encode_macro(parse_steps(STEPS_A), 26, 1, 0)[0]
        for value in (1, 2, 3):
            m = encode_macro(parse_steps(STEPS_A), 26, 1, value)[0]
            self.assertEqual([i for i in range(MESSAGE_LEN) if m[i] != base[i] and not 109 <= i <= 110], [3])

    def test_mode_is_only_in_the_first_message_of_a_long_macro(self):
        msgs = encode_macro(parse_steps([{"tap": "a", "delay_ms": 50}] * 30), 21, 1, 2)
        self.assertEqual((msgs[0][3], msgs[1][3]), (2, 0))

    def test_bad_mode_rejected(self):
        for bad in ("forever", 4, -1):
            with self.subTest(mode=bad):
                with self.assertRaises(ValueError):
                    if isinstance(bad, str):
                        from_config("t", {"buttons": {7: {"type": "macro", "steps": STEPS_A, "start": bad}}})
                    else:
                        encode_macro(parse_steps(STEPS_A), 26, 1, bad)

    def test_repeat_with_non_loop_mode_warns(self):
        with self.assertWarns(UserWarning):
            from_config("t", {"buttons": {7: {"type": "macro", "steps": STEPS_A, "start": "hold", "repeat": 3}}})
        with warnings.catch_warnings():
            warnings.simplefilter("error")
            from_config("t", {"buttons": {7: {"type": "macro", "steps": STEPS_A, "start": "loop", "repeat": 3}}})

    def test_describe_shows_mode(self):
        p = from_config("t", {"buttons": {"side_front": {"type": "macro", "steps": STEPS_A, "start": "hold"}}})
        self.assertIn("hold", p.current_buttons()["side_front"])


class EncodingRules(unittest.TestCase):
    """按 FUN_00461c90 的逻辑实现的部分(没有抓包), 检查内部一致性。"""

    def test_key_event_bytes(self):
        self.assertEqual(encode_event(MacroEvent("down", 0x04, 50)), (bytes([0x33, 0x04]), 1))
        self.assertEqual(encode_event(MacroEvent("up", 0x04, 50)), (bytes([0xB3, 0x04]), 1))

    def test_extended_delay(self):
        # 250 ms = 2*100 + 50
        self.assertEqual(encode_event(MacroEvent("down", 0x04, 250)), (bytes([0x01 | 50, 0x04, 2, 0x03]), 2))
        self.assertEqual(encode_event(MacroEvent("up", 0x04, 200)), (bytes([0x81 | 1, 0x04, 2, 0x03]), 2))

    def test_move_event(self):
        self.assertEqual(encode_event(MacroEvent("move", 0, 20, 10, -5)),
                         (bytes([20, 0xF6, 0x00, 0x0A, 0xFF, 0xFB]), 3))
        self.assertEqual(encode_event(MacroEvent("move", 0, 300, 1, 2)),
                         (bytes([1, 0xF6, 0, 1, 0, 2, 3, 0x03]), 4))

    def test_odd_key_delays_are_rejected_not_rounded(self):
        for d in (51, 1, 127, 151, 301):
            with self.subTest(d=d), self.assertRaises(ValueError):
                encode_event(MacroEvent("down", 4, d))
        for d in (-2, 25600, 50.0, True):
            with self.subTest(d=d), self.assertRaises(ValueError):
                encode_event(MacroEvent("down", 4, d))

    def test_message_header_and_checksum(self):
        m = encode_macro(parse_steps([{"tap": "a"}]), first_slot=1, repeat=3)[0]
        self.assertEqual((len(m), m[0], m[1], m[2], m[7], m[8]), (MESSAGE_LEN, 9, 0x6F, 1, 3, 2))
        self.assertEqual(int.from_bytes(m[109:111], "big"), sum(m[3:109]))

    def test_long_macro_continues_in_next_slot(self):
        steps = [{"tap": "a", "delay_ms": 50}] * 30          # 120 字节事件, 超过单条 98 字节
        msgs = encode_macro(parse_steps(steps), first_slot=21)
        self.assertEqual(len(msgs), 2)
        self.assertEqual((msgs[0][2], msgs[1][2]), (21, 22))
        # 第一条以 [slot+1, 0xFE] 结尾; 第二条不带头部属性
        area = msgs[0][9:]
        end = 2 * msgs[0][8]
        self.assertEqual(area[end - 2:end], bytes([22, CONTINUE_OPCODE]))
        self.assertEqual((msgs[1][7], msgs[1][3:7]), (0, bytes(4)))
        # 事件一个不丢: 第一条(去掉标记)+第二条的事件数 == 总数 60
        n_first = (end - 2) // 2
        n_second = msgs[1][8]
        self.assertEqual(n_first + n_second, 60)
        for m in msgs:
            self.assertEqual(int.from_bytes(m[109:111], "big"), sum(m[3:109]))

    def test_too_long_macro_is_rejected(self):
        steps = [{"tap": "a", "delay_ms": 50}] * 300
        with self.assertRaises(ValueError):
            encode_macro(parse_steps(steps), first_slot=1)

    def test_message_frames_split_59_52(self):
        m = encode_macro(parse_steps([{"tap": "a"}]), first_slot=1)[0]
        f0, f1 = message_frames(m)
        self.assertEqual((f0[1], f0[2], f1[1], f1[2]), (0x40, 0, 0x39, 1))


class StepsAndConfig(unittest.TestCase):
    def test_bad_steps(self):
        for bad in (
            [], None, [{"tap": "a", "typo": 1}], [{}], [{"tap": "a", "down": "b"}],
            [{"tap": "mouse_laser"}], [{"move": [1]}], [{"tap": "nonexistent_key"}],
            [{"tap": "a", "delay_ms": 51}],
        ):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse_steps(bad)

    def test_buttons_without_a_slot_cannot_hold_a_macro(self):
        for idx in (4, 5, 8, 17):
            with self.subTest(button=idx), self.assertRaises(ValueError):
                from_config("t", {"buttons": {idx: {"type": "macro", "steps": STEPS_A}}})

    def test_repeat_range(self):
        for bad in (0, 256, -1):
            with self.subTest(repeat=bad), self.assertRaises(ValueError):
                from_config("t", {"buttons": {7: {"type": "macro", "steps": STEPS_A, "repeat": bad}}})
        from_config("t", {"buttons": {7: {"type": "macro", "steps": STEPS_A, "repeat": 255}}})

    def test_unverified_mouse_buttons_warn(self):
        with self.assertWarns(UserWarning):
            parse_steps([{"tap": "mouse_middle"}])
        with warnings.catch_warnings():
            warnings.simplefilter("error")
            parse_steps([{"tap": "mouse_left"}, {"tap": "mouse_right"}])

    def test_rebinding_a_macro_button_clears_the_macro(self):
        p = from_config("t", {"buttons": {7: {"type": "macro", "steps": STEPS_A}}}).profile
        p.set_button_key(7, 4)
        self.assertEqual(p.macros, {})
        self.assertEqual(len(build_apply_sequence(p)), 6)

    def test_function_id_cannot_smuggle_a_macro(self):
        with self.assertRaises(ValueError):
            from_config("t", {"buttons": {7: {"type": "function", "id": 0x11}}})

    def test_apply_sequence_appends_macros_in_button_order(self):
        cfg = {"buttons": {7: {"type": "macro", "steps": STEPS_A}, 6: {"type": "macro", "steps": STEPS_A}}}
        seq = from_config("t", cfg).build_packets()
        self.assertEqual(len(seq), 6 + 4)
        self.assertEqual([parse_chunk_frame(f)[1][2] for f in (seq[6], seq[8])], [21, 26])


if __name__ == "__main__":
    unittest.main()
