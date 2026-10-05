"""BleChannel: 用假 GATT 会话验证写入内容/顺序/失败处理/通知转发, 不碰真实蓝牙。"""

import time
import unittest

from inphic.profile import from_config
from inphic.runtime.ble import BleChannel


class FakeSession:
    def __init__(self, fail_on=None):
        self.writes = []
        self.connected = True
        self.closed = False
        self.fail_on = fail_on
        self.on_notify = None

    async def write(self, payload):
        if self.fail_on is not None and len(self.writes) == self.fail_on:
            raise RuntimeError("write failed")
        self.writes.append(payload)

    async def close(self):
        self.closed = True


def make_channel(sessions, **kw):
    opened = []

    async def open_session(address, on_notify):
        s = sessions.pop(0)
        s.on_notify = on_notify
        opened.append(s)
        return s

    ch = BleChannel("78:87:11:75:9D:56", packet_delay_ms=0, open_session=open_session, **kw)
    return ch, opened


class BleChannelTest(unittest.TestCase):
    def setUp(self):
        self.profile = from_config("t", {"polling_rate": 1000, "light": {"mode": "spectrum_cycle"}})

    def test_sends_raw_payloads_without_polling(self):
        ch, opened = make_channel([FakeSession()])
        self.addCleanup(ch.close)
        ch.send_profile(self.profile)
        sent = opened[0].writes
        self.assertEqual([p[0] for p in sent], [0x0C, 0x04, 0x05, 0x07, 0x08])     # 没有 0x06 回报率
        self.assertEqual(sent, self.profile.build_payloads(include_polling=False))
        self.assertTrue(all(len(p) == p[1] for p in sent))                          # 原始载荷: 长度字节 = 长度, 无帧头

    def test_failed_write_drops_session_and_reopens(self):
        first, second = FakeSession(fail_on=2), FakeSession()
        ch, opened = make_channel([first, second])
        self.addCleanup(ch.close)
        with self.assertRaises(RuntimeError):
            ch.send_profile(self.profile)
        self.assertTrue(first.closed)
        ch.send_profile(self.profile)
        self.assertEqual(len(opened), 2)
        self.assertEqual(len(second.writes), 5)

    def test_notifications_reach_listener(self):
        got = []
        ch, opened = make_channel([FakeSession()], listener=got.append)
        self.addCleanup(ch.close)
        ch.connect()
        opened[0].on_notify(bytes.fromhex("93400142"))
        opened[0].on_notify(b"\x01\x02")                    # 不是状态报告, 被忽略
        self.assertEqual([(r.kind, r.a, r.b) for r in got], [(0x40, 1, 0x42)])

    def test_unparseable_notifications_go_to_on_raw(self):
        got, raws = [], []
        ch, opened = make_channel([FakeSession()], listener=got.append, on_raw=raws.append)
        self.addCleanup(ch.close)
        ch.connect()
        opened[0].on_notify(b"\x01\x02")
        opened[0].on_notify(bytes.fromhex("93400142ff"))        # 多出来的字节也在 raw 里
        self.assertEqual(raws, [b"\x01\x02"])
        self.assertEqual(got[0].raw, bytes.fromhex("93400142ff"))

    def test_is_present_follows_session_and_release_notifies(self):
        gone = []
        ch, opened = make_channel([FakeSession()], on_disconnect=lambda: gone.append(1))
        self.addCleanup(ch.close)
        ch.connect()
        self.assertTrue(ch.is_present())
        opened[0].connected = False
        self.assertFalse(ch.is_present())
        ch.release()
        self.assertTrue(opened[0].closed)
        self.assertEqual(gone, [1])


if __name__ == "__main__":
    unittest.main()


class WinrtApiShape(unittest.TestCase):
    """假会话测不到 winrt 的方法名/参数个数(上次就栽在这); 装了 winrt 的机器上核对 WinrtSession 用到的接口。不碰设备。"""

    def test_methods_used_by_session_exist(self):
        try:
            from winrt.windows.devices.bluetooth import BluetoothLEDevice
            from winrt.windows.devices.bluetooth.genericattributeprofile import (
                GattCharacteristic, GattDeviceService, GattWriteOption,
            )
            from winrt.windows.storage.streams import DataReader, DataWriter
        except ImportError:
            self.skipTest("没有安装 winrt-*(非 Windows)")
        for cls, names in (
            (BluetoothLEDevice, ["from_bluetooth_address_async", "get_gatt_services_for_uuid_async"]),
            (GattDeviceService, ["get_characteristics_for_uuid_async"]),
            (GattCharacteristic, ["write_value_with_result_and_option_async", "add_value_changed",
                                  "remove_value_changed", "write_client_characteristic_configuration_descriptor_async"]),
            (DataWriter, ["write_bytes", "detach_buffer"]),
            (DataReader, ["from_buffer", "read_byte"]),
        ):
            for name in names:
                self.assertTrue(hasattr(cls, name), f"{cls.__name__}.{name} 不存在")
        self.assertEqual(int(GattWriteOption.WRITE_WITH_RESPONSE), 0)
