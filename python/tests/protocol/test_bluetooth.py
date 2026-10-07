"""蓝牙协议的纯函数: 地址解析、FEE4 通知 -> 状态报告。"""

import unittest

from inphic.protocol.bluetooth import parse_address, report_from_notification
from inphic.protocol.status import battery_of


class ParseAddress(unittest.TestCase):
    def test_formats(self):
        for text in ("78:87:11:75:9D:56", "78-87-11-75-9d-56", "788711759D56"):
            self.assertEqual(parse_address(text), 0x788711759D56)

    def test_rejects_bad(self):
        for text in ("", "78:87:11", "ZZ:87:11:75:9D:56"):
            with self.assertRaises(ValueError):
                parse_address(text)


class Notification(unittest.TestCase):
    def test_real_battery_notification(self):
        # 真机 FEE4 收到的 93 40 01 42 = 电量状态码 1, 66%
        bat = battery_of(report_from_notification(bytes.fromhex("93400142")))
        self.assertEqual((bat.percent, bat.state), (66, 1))

    def test_other_data_ignored(self):
        self.assertIsNone(report_from_notification(b"\x00\x01"))
        self.assertIsNone(report_from_notification(bytes.fromhex("11223344")))


if __name__ == "__main__":
    unittest.main()
