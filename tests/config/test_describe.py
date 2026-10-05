"""启动横幅(中文设置摘要)的内容检查。"""

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
