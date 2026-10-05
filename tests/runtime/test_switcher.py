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
