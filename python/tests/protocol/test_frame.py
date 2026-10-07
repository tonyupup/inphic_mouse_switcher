"""帧格式: 用真实抓包验证组帧 / 解帧 / 校验。"""

import unittest

from inphic.protocol.frame import build_chunk_frame, build_frame, parse_chunk_frame, parse_frame
from tests.support import captured_frames, captured_macro_frames

FRAMES = captured_frames()
KEYMAPS = [f for f in FRAMES if f[3] == 0x08]


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

    def test_build_rejects_oversize_payload(self):
        with self.assertRaises(ValueError):
            build_frame(bytes([0x08, 60]) + bytes(58))


class ChunkFrameTests(unittest.TestCase):
    def test_every_captured_macro_frame_roundtrips(self):
        for tag, frames in captured_macro_frames().items():
            for f in frames:
                idx, data = parse_chunk_frame(f)
                self.assertEqual(build_chunk_frame(data, idx), f, tag)

    def test_normal_frame_is_chunk_zero(self):
        f = FRAMES[0]
        self.assertEqual(parse_chunk_frame(f)[0], 0)

    def test_parse_frame_rejects_a_continuation_chunk(self):
        _, _, _, c1 = captured_macro_frames()["A"]
        with self.assertRaises(ValueError):
            parse_frame(c1)                         # 分片序号 1 不是普通命令帧

    def test_chunk_limits(self):
        with self.assertRaises(ValueError):
            build_chunk_frame(bytes(60), 0)
        with self.assertRaises(ValueError):
            build_chunk_frame(b"", 256)


class KeymapInvariants(unittest.TestCase):
    """按键表帧(命令 0x08)的内层校验: frame[60:62] 是 frame[6:60] 的 16 位和, 大端。"""

    def test_have_samples(self):
        self.assertGreaterEqual(len({bytes(k) for k in KEYMAPS}), 8)

    def test_inner_checksum_rule_holds_on_all_captures(self):
        for k in KEYMAPS:
            self.assertEqual(int.from_bytes(k[60:62], "big"), sum(k[6:60]))


if __name__ == "__main__":
    unittest.main()
