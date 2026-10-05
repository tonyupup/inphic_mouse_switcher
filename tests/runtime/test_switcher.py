"""switcher 的失败重试/断线重连逻辑, 用假的 hid 模块, 不碰真实设备。"""

import importlib
import sys
import time
import types
import unittest

from inphic.config import load_config
from tests.support import ROOT


class FakeHid(types.ModuleType):
    """可编程的 hid 模块: 控制 enumerate 是否有设备、write 的返回值。"""

    def __init__(self):
        super().__init__("hid")
        self.present = True
        self.write_results = []        # 依次弹出; 空了就成功
        self.opened = []               # 所有打开过的 device
        self.writes = 0
        outer = self

        class device:
            def __init__(self):
                self.closed = False

            def open_path(self, path):
                outer.opened.append(self)

            def write(self, data):
                outer.writes += 1
                if outer.write_results:
                    return outer.write_results.pop(0)
                return len(data)

            def error(self):
                return "fake error"

            def close(self):
                self.closed = True

        self.device = device

    def enumerate(self, vid, pid):
        if not self.present:
            return []
        return [{"usage_page": 0xFF00, "usage": 1, "path": b"fake"}]


def _load_switcher(fake_hid):
    """用假的 hid 模块导入运行时模块(它们在导入时就 import hid, 所以要先清掉缓存)。"""
    sys.modules["hid"] = fake_hid
    for name in ("inphic.runtime.channel", "inphic.runtime.switcher"):
        sys.modules.pop(name, None)
    return importlib.import_module("inphic.runtime.switcher")


class SwitcherRetry(unittest.TestCase):
    def setUp(self):
        self.hid = FakeHid()
        self.sw = _load_switcher(self.hid)
        cfg = load_config(ROOT / "config.yaml")
        cfg.raw["packet_delay_ms"] = 0
        cfg.raw["device"].pop("bluetooth", None)          # 真实配置里写了蓝牙地址; 测试不碰真实蓝牙
        self.s = self.sw.ProfileSwitcher(cfg)

    def _wait_idle(self):
        for _ in range(500):
            if not self.s._applying:
                return
            time.sleep(0.01)
        self.fail("apply 没有结束")

    def test_success_marks_applied_and_does_not_repeat(self):
        self.s.desired = "default"
        self.s._maybe_apply(); self._wait_idle()
        self.assertEqual(self.s.applied, "default")
        n = self.hid.writes
        self.s._maybe_apply(); self._wait_idle()
        self.assertEqual(self.hid.writes, n)           # 已应用, 不重复写

    def test_failed_write_drops_handle_and_retries_later(self):
        self.hid.write_results = [-1]
        self.s.desired = "cf"
        with self.assertLogs("inphic.switcher", level="ERROR"):
            self.s._maybe_apply(); self._wait_idle()
        self.assertIsNone(self.s.applied)              # 失败不能算已应用
        self.assertTrue(self.hid.opened[0].closed)     # 失效句柄被丢弃
        self.s._maybe_apply(); self._wait_idle()
        self.assertEqual(len(self.hid.opened), 1)      # 还在退避期, 没有再试
        self.s._retry_at = 0
        self.s._maybe_apply(); self._wait_idle()
        self.assertEqual(self.s.applied, "cf")
        self.assertEqual(len(self.hid.opened), 2)      # 重新打开了新句柄

    def test_replug_triggers_reapply(self):
        self.s.desired = "default"
        self.s._maybe_apply(); self._wait_idle()
        n = self.hid.writes
        self.hid.present = False
        self.s._poll_presence(100.0)
        self.assertFalse(self.s._was_present)
        self.hid.present = True
        self.s._poll_presence(200.0)
        self.assertIsNone(self.s.applied)              # 重新连接后要重新应用
        self.s._maybe_apply(); self._wait_idle()
        self.assertGreater(self.hid.writes, n)
        self.assertEqual(self.s.applied, "default")

    def test_profile_change_applies_new_profile(self):
        self.s.desired = "default"
        self.s._maybe_apply(); self._wait_idle()
        self.s.desired = "cf"
        self.s._maybe_apply(); self._wait_idle()
        self.assertEqual(self.s.applied, "cf")

    def test_unknown_profile_name_is_ignored_safely(self):
        self.s.desired = "nope"
        self.s._maybe_apply(); self._wait_idle()
        self.assertIsNone(self.s.desired)


if __name__ == "__main__":
    unittest.main()


class FakeBle:
    """BleChannel 的替身: 记录发送的方案, 可控制是否在线。"""

    kind = "bluetooth"
    label = "蓝牙"
    supports_polling_rate = False

    def __init__(self):
        self.present = True
        self.sent = []
        self.released = 0

    def is_present(self):
        return self.present

    def send_profile(self, profile, delay_s=None):
        self.sent.append(profile.name)

    def release(self):
        self.released += 1

    def close(self):
        pass


class SwitcherBluetooth(unittest.TestCase):
    def setUp(self):
        self.hid = FakeHid()
        self.hid.present = False                       # 接收器没插, 只有蓝牙
        self.sw = _load_switcher(self.hid)
        cfg = load_config(ROOT / "config.yaml")
        cfg.raw["packet_delay_ms"] = 0
        cfg.raw["device"].pop("bluetooth", None)          # 真实配置里写了蓝牙地址; 测试不碰真实蓝牙
        self.s = self.sw.ProfileSwitcher(cfg)
        self.ble = FakeBle()
        self.s.ble = self.ble
        self.s.channels = {"bluetooth": self.ble, "dongle": self.s.channel}      # 蓝牙优先

    def _wait_idle(self):
        for _ in range(500):
            if not self.s._applying:
                return
            time.sleep(0.01)
        self.fail("apply 没有结束")

    def test_uses_bluetooth_when_dongle_absent(self):
        self.s.desired = "cf"
        with self.assertLogs("inphic.switcher", level="INFO") as logs:
            self.s._poll_presence(100.0)
            self.s._maybe_apply(); self._wait_idle()
        self.assertEqual(self.s.kind, "bluetooth")
        self.assertEqual(self.ble.sent, ["cf"])
        self.assertEqual(self.hid.writes, 0)
        text = "\n".join(logs.output)
        self.assertIn("连接方式: 蓝牙", text)
        self.assertIn("回报率设置无效", text)

    def test_bluetooth_preferred_and_switching_reapplies(self):
        self.hid.present = True                         # 接收器一直插着, 但鼠标此刻连着蓝牙 → 用蓝牙
        self.s.desired = "default"
        self.s._poll_presence(100.0)
        self.assertEqual(self.s.kind, "bluetooth")
        self.s._maybe_apply(); self._wait_idle()
        self.assertEqual(self.ble.sent, ["default"])
        self.assertEqual(self.hid.writes, 0)
        self.ble.present = False                        # 鼠标切回 2.4G → 放掉蓝牙, 重新应用
        self.s._poll_presence(200.0)
        self.assertEqual(self.s.kind, "dongle")
        self.assertEqual(self.ble.released, 1)
        self.assertIsNone(self.s.applied)
        self.s._maybe_apply(); self._wait_idle()
        self.assertGreater(self.hid.writes, 0)
        self.assertEqual(self.s.applied, "default")

    def test_nothing_present_logs_disconnect(self):
        self.ble.present = False
        with self.assertLogs("inphic.switcher", level="WARNING"):
            self.s._poll_presence(100.0)
        self.assertFalse(self.s._was_present)


class SwitcherStatusLog(unittest.TestCase):
    def setUp(self):
        self.sw = _load_switcher(FakeHid())
        cfg = load_config(ROOT / "config.yaml")
        cfg.raw["device"].pop("bluetooth", None)
        self.s = self.sw.ProfileSwitcher(cfg)
        self.s.applied = "default"

    def _report(self, kind, a, b=0):
        from inphic.protocol.status import StatusReport
        return StatusReport(kind=kind, a=a, b=b)

    def test_dpi_key_press_is_logged_with_dpi_value(self):
        with self.assertLogs("inphic.switcher", level="INFO") as logs:
            self.s._on_status(self._report(0x10, 3))          # 第 3 档
        self.assertIn("第 3 档 = 3200 DPI", logs.output[0])

    def test_out_of_range_dpi_stage_reported_not_crashed(self):
        with self.assertLogs("inphic.switcher", level="INFO") as logs:
            self.s._on_status(self._report(0x10, 9))
        self.assertIn("超出", logs.output[0])

    def test_unknown_kind_is_silent(self):
        with self.assertNoLogs("inphic.switcher", level="INFO"):
            self.s._on_status(self._report(0x99, 1))
