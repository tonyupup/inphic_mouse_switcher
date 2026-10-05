"""电量日志: BatteryTracker 的日志规则, 以及 StatusMonitor 读取/重连(假设备)。"""

import threading
import unittest

from inphic.protocol.status import Battery
from inphic.runtime.monitor import BatteryTracker, StatusMonitor


def texts(items):
    return [t for _, t in items]


class BatteryTrackerTest(unittest.TestCase):
    def test_first_reading_logged_then_only_changes(self):
        t = BatteryTracker()
        self.assertIn("电量 66%", texts(t.update(Battery(66, 1)))[0])
        self.assertEqual(t.update(Battery(66, 1)), [])
        out = t.update(Battery(65, 1))
        self.assertEqual(texts(out), ["电量 66% → 65%"])

    def test_state_change_logged(self):
        t = BatteryTracker()
        t.update(Battery(50, 1))
        self.assertIn("状态码 1 → 2", texts(t.update(Battery(50, 2)))[0])

    def test_low_battery_warns_once_per_level(self):
        t = BatteryTracker()
        t.update(Battery(25, 1))
        out = t.update(Battery(20, 1))
        self.assertEqual([lv for lv, _ in out], ["info", "warning"])
        self.assertEqual([lv for lv, _ in t.update(Battery(19, 1))], ["info"])
        out = t.update(Battery(10, 1))
        self.assertEqual([lv for lv, _ in out], ["info", "warning"])
        self.assertEqual([lv for lv, _ in t.update(Battery(9, 1))], ["info"])

    def test_warns_again_after_recharge(self):
        t = BatteryTracker()
        t.update(Battery(15, 1))
        t.update(Battery(60, 1))
        out = t.update(Battery(20, 1))
        self.assertIn("warning", [lv for lv, _ in out])

    def test_low_on_first_reading_warns(self):
        out = BatteryTracker().update(Battery(8, 1))
        self.assertEqual([lv for lv, _ in out], ["info", "warning", "warning"])

    def test_reset_makes_next_reading_first(self):
        t = BatteryTracker()
        t.update(Battery(50, 1))
        t.reset()
        self.assertTrue(texts(t.update(Battery(50, 1))))


class FakeDev:
    def __init__(self, reads):
        self.reads = list(reads)
        self.closed = False

    def read(self, size, timeout):
        if self.reads:
            item = self.reads.pop(0)
            if isinstance(item, Exception):
                raise item
            return item
        raise OSError("device gone")

    def close(self):
        self.closed = True


class StatusMonitorTest(unittest.TestCase):
    def test_reads_reports_and_reconnects(self):
        battery = [0x03, 0x93, 0x40, 0x01, 0x44] + [0] * 59
        other = [0x03, 0x93, 0x20, 0x04, 0x00] + [0] * 59
        devs = [FakeDev([battery, other]), FakeDev([battery])]
        opens = []
        got, disc = [], []
        done = threading.Event()

        def opener():
            if not devs:
                done.set()
                raise OSError("no device")
            d = devs.pop(0)
            opens.append(d)
            return d

        m = StatusMonitor(0, 0, got.append, on_disconnect=lambda: disc.append(1),
                          open_device=opener, retry_s=0.01, poll_ms=1)
        m.start()
        self.assertTrue(done.wait(3))
        m.stop()
        self.assertEqual([r.kind for r in got], [0x40, 0x20, 0x40])
        self.assertEqual(len(disc), 2)
        self.assertTrue(all(d.closed for d in opens))


if __name__ == "__main__":
    unittest.main()
