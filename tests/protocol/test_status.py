"""状态报告(电量)解析: 用 macro 抓包里真实出现的报告。"""

import unittest

from inphic.protocol.status import (
    KIND_BATTERY, KIND_DPI_STAGE, Battery, StatusReport, battery_of, parse_report, read_battery, read_reports,
)

# 抓包里 EP 0x83 的真实报告: 电量(UI 当时显示 69%), 以及每个 OUT 帧后的两个确认
BATTERY_69 = bytes.fromhex("0393400145")
ACK_C0 = bytes.fromhex("0406c00100cb")
ACK_C1 = bytes.fromhex("0406c10100cc")


class FakeDev:
    def __init__(self, reports):
        self.reports = list(reports)

    def read(self, n, timeout_ms=0):
        return list(self.reports.pop(0)) if self.reports else []


class Parse(unittest.TestCase):
    def test_real_battery_report(self):
        rep = parse_report(BATTERY_69)
        self.assertEqual(rep, StatusReport(KIND_BATTERY, 0x01, 0x45))
        self.assertEqual(battery_of(rep), Battery(percent=69, state=1))
        self.assertEqual(str(battery_of(rep)), "69% (state 1)")

    def test_acks_are_not_status_reports(self):
        self.assertIsNone(parse_report(ACK_C0))
        self.assertIsNone(parse_report(ACK_C1))

    def test_accepts_list_from_hid_read(self):
        self.assertIsNotNone(parse_report(list(BATTERY_69)))

    def test_short_or_wrong_marker_rejected(self):
        for bad in (b"", b"\x03\x93\x40\x01", b"\x03\x92\x40\x01\x45", b"\x04\x93\x40\x01\x45"):
            self.assertIsNone(parse_report(bad))

    def test_percent_over_100_is_not_a_battery(self):
        self.assertIsNone(battery_of(parse_report(bytes([3, 0x93, 0x40, 1, 101]))))

    def test_other_kinds_keep_raw_values(self):
        rep = parse_report(bytes([3, 0x93, 0x10, 3, 0]))
        self.assertEqual((rep.kind, rep.name, rep.value), (KIND_DPI_STAGE, "dpi_stage", 3))
        self.assertIsNone(battery_of(rep))
        self.assertEqual(parse_report(bytes([3, 0x93, 0x99, 1, 2])).name, "unknown_0x99")


class Reading(unittest.TestCase):
    def test_read_battery_skips_acks_and_empty_reads(self):
        dev = FakeDev([ACK_C0, ACK_C1, b"", BATTERY_69])
        self.assertEqual(read_battery(dev, timeout_s=1.0), Battery(69, 1))

    def test_read_battery_times_out(self):
        self.assertIsNone(read_battery(FakeDev([ACK_C0]), timeout_s=0.3))

    def test_read_reports_collects_only_status(self):
        got = read_reports(FakeDev([ACK_C0, BATTERY_69, ACK_C1, BATTERY_69]), timeout_s=0.3)
        self.assertEqual(len(got), 2)


if __name__ == "__main__":
    unittest.main()
