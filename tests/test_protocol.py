"""用真实抓包验证 mouse_protocol 的组帧/校验。运行: python -m unittest discover -s tests -t ."""

import unittest
from pathlib import Path

from mouse_protocol import (
    BUTTON_SLOT_OFFSETS, CMD_KEYMAP, ButtonTable, KeyAction, MouseFunction,
    build_frame, parse_frame, seal_frame,
)

FRAMES = [
    bytes.fromhex(l.strip())
    for l in (Path(__file__).parent / "captured_frames.txt").read_text().splitlines()
    if l.strip() and not l.startswith("#")
]
KEYMAPS = [f for f in FRAMES if f[3] == CMD_KEYMAP]


class FrameTests(unittest.TestCase):
    def test_captured_frames_roundtrip(self):
        for f in FRAMES:
            self.assertEqual(build_frame(parse_frame(f)), f)

    def test_parse_rejects_bad_checksum(self):
        bad = bytearray(FRAMES[0])
        bad[63] ^= 1
        with self.assertRaises(ValueError):
            parse_frame(bytes(bad))

    def test_build_rejects_length_mismatch(self):
        with self.assertRaises(ValueError):
            build_frame(bytes([0x08, 0x05, 0, 0]))


class KeymapTests(unittest.TestCase):
    def test_have_samples(self):
        self.assertGreaterEqual(len({bytes(k) for k in KEYMAPS}), 8)

    def test_seal_restores_both_checksums(self):
        for k in KEYMAPS:
            broken = bytearray(k)
            broken[60:62] = b"\x00\x00"  # 内层 16 位校验
            broken[62:64] = b"\x00\x00"  # 外层 16 位校验
            seal_frame(broken)
            self.assertEqual(bytes(broken), k)

    def test_rebind_reproduces_capture(self):
        # 以任意一个按键表为底板, 套上另一个抓包里的绑定, 校验后必须和那个抓包逐字节一致
        base = bytearray(KEYMAPS[0])
        for target in KEYMAPS:
            pkt = bytearray(base)
            ButtonTable(ButtonTable.read_from(bytes(target))).apply_to(pkt)
            seal_frame(pkt)
            self.assertEqual(bytes(pkt), bytes(target))

    def test_rebind_updates_high_checksum_byte(self):
        # 旧实现只写 byte63, 且不更新内层校验: 这里换成 F12 后两处都必须变
        pkt = bytearray(KEYMAPS[0])
        before = bytes(pkt)
        ButtonTable({"side_front": KeyAction("f12")}).apply_to(pkt)
        seal_frame(pkt)
        self.assertNotEqual(pkt[60:62], before[60:62])
        parse_frame(bytes(pkt))  # 外层校验必须自洽
        self.assertEqual(int.from_bytes(pkt[60:62], "big"), sum(pkt[6:60]))


class ButtonSlotTests(unittest.TestCase):
    def test_slots_decode_to_known_actions(self):
        for k in KEYMAPS:
            got = ButtonTable.read_from(k)
            self.assertEqual(set(got), set(BUTTON_SLOT_OFFSETS))
            for a in got.values():
                self.assertIsInstance(a, (KeyAction, MouseFunction))


if __name__ == "__main__":
    unittest.main()
