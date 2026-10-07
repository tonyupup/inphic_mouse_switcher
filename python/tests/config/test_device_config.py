"""device.connection / device.bluetooth 的校验与启动横幅里的连接方式。"""

import tempfile
import unittest
from pathlib import Path

import yaml

from inphic.config import describe_config, load_config
from tests.support import ROOT


def load_with(**device_extra):
    raw = yaml.safe_load((ROOT / "config.yaml").read_text(encoding="utf-8"))
    raw["device"].pop("bluetooth", None)             # 真实配置里可能已经写了蓝牙地址, 这里从干净状态开始
    raw["device"].pop("connection", None)
    raw["device"].update(device_extra)
    raw.pop("macro_files", None)
    raw["profiles"] = {"default": {}, "cf": {}}
    with tempfile.TemporaryDirectory() as d:
        path = Path(d) / "c.yaml"
        path.write_text(yaml.safe_dump(raw, allow_unicode=True), encoding="utf-8")
        return load_config(path), path


class DeviceConfig(unittest.TestCase):
    def test_default_is_dongle_auto_without_bluetooth(self):
        cfg, _ = load_with()
        self.assertEqual(cfg.connection, "auto")
        self.assertIsNone(cfg.bluetooth)

    def test_bluetooth_section_accepted(self):
        cfg, path = load_with(bluetooth={"address": "78:87:11:75:9D:56"})
        self.assertEqual(cfg.bluetooth["address"], "78:87:11:75:9D:56")
        text = describe_config(cfg, path)
        self.assertIn("蓝牙", text)
        self.assertIn("78:87:11:75:9D:56", text)

    def test_rejects_bad_values(self):
        for extra in (
            {"connection": "wifi"},
            {"connection": "bluetooth"},                        # 选了蓝牙但没写地址
            {"bluetooth": {}},
            {"bluetooth": {"address": "12:34"}},
        ):
            with self.assertRaises(ValueError, msg=str(extra)):
                load_with(**extra)


if __name__ == "__main__":
    unittest.main()
