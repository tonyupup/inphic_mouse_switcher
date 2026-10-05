"""启动横幅(中文设置摘要)的内容检查。"""

import re
import unittest

from inphic.config import describe_config, load_config
from tests.support import ROOT


class DescribeConfigTest(unittest.TestCase):
    def test_banner_contains_key_facts(self):
        cfg_path = ROOT / "config.yaml"
        text = describe_config(load_config(cfg_path), cfg_path)
        for needle in ("VID 0x1D57", "PID 0xFA65", "回报率", "灯光", "DPI", "按钮", "方案「cf」", "★"):
            self.assertIn(needle, text)


if __name__ == "__main__":
    unittest.main()


class ShowPayloads(unittest.TestCase):
    def test_payload_dump_lists_both_links_and_skips_polling_on_bluetooth(self):
        import contextlib
        import io

        from inphic.cli.show import show

        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            show(str(ROOT / "config.yaml"), payloads=True)
        text = out.getvalue()
        self.assertIn("将要发送的字节", text)
        self.assertIn("蓝牙（逐条写进 FEE3，不含回报率命令）", text)
        bt = text.split("蓝牙（逐条写进 FEE3")[1].split("个 64 字节帧")[0]
        rows = [line for line in bt.splitlines() if re.match(r"\s+\d+\. ", line)]
        self.assertEqual(len(rows), 5)                               # 蓝牙 5 条: 没有回报率那一条
        self.assertFalse(any("回报率" in line for line in rows))
