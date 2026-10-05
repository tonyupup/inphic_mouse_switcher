"""
蓝牙(BLE)通道: 鼠标走蓝牙时, 配置命令写进 GATT 服务 0xFEE0 的特征值 FEE3, 状态从 FEE4 通知来。

来源: HUB 的 FUN_00409b20(写) / FUN_0040a310(订阅), 以及对 Inphic IN10 的真机只读验证(见 wiki)。
    FEE3  写     值 = 原始命令载荷(不带 04 L+5 帧头, 不带校验), 宏消息 111 字节整条写
    FEE4  通知   内容 = 状态报告去掉开头的 report id 03, 如 93 40 01 42
    蓝牙下不发回报率命令(0x06): HUB 也不发, 官方 UI 里蓝牙状态下回报率设置无效

⚠ 同一时间只能有一个程序打开 FEE0: HUB(INPHIC.exe)或 Web 驱动(Inphic.exe)在运行时会被拒绝访问。

只用 Windows 自带的 WinRT(pip 包 winrt-*), 没有 bleak: bleak 按地址连接已连上的 LE 设备会走扫描而失败。
"""

from __future__ import annotations

import asyncio
import logging
import threading
import time

from inphic.protocol.bluetooth import NOTIFY_UUID, SERVICE_UUID, WRITE_UUID, parse_address, report_from_notification

LOG = logging.getLogger("inphic.ble")

SUCCESS = 0                    # GattCommunicationStatus.SUCCESS
ACCESS_DENIED = 3              # GattCommunicationStatus.ACCESS_DENIED
CONNECTED = 1                  # BluetoothConnectionStatus.CONNECTED


class WinrtSession:
    """一次到鼠标 FEE0 服务的 GATT 会话(已订阅 FEE4)。所有方法都是 async, 由 BleChannel 的事件循环调用。"""

    def __init__(self, device, write_char, notify_char, token):
        self.device = device
        self.write_char = write_char
        self.notify_char = notify_char
        self._token = token

    @staticmethod
    async def open(address: int, on_notify) -> "WinrtSession":
        # 延迟导入: 只有 Windows 且装了 winrt-* 才有
        from winrt.windows.devices.bluetooth import BluetoothCacheMode, BluetoothLEDevice
        from winrt.windows.devices.bluetooth.genericattributeprofile import (
            GattClientCharacteristicConfigurationDescriptorValue as Cccd,
        )
        from winrt.windows.storage.streams import DataReader
        import uuid

        device = await BluetoothLEDevice.from_bluetooth_address_async(address)
        if device is None:
            raise RuntimeError("系统里没有这个蓝牙设备, 先在 Windows 设置里配对")
        if int(device.connection_status) != CONNECTED:
            raise RuntimeError("鼠标没有通过蓝牙连接(休眠了? 动一下鼠标)")
        svc_res = await device.get_gatt_services_for_uuid_async(uuid.UUID(SERVICE_UUID))
        if int(svc_res.status) == ACCESS_DENIED:
            raise RuntimeError("FEE0 服务被拒绝访问: 请先退出 INPHIC HUB(INPHIC.exe)和 Inphic.exe")
        if int(svc_res.status) != SUCCESS or svc_res.services.size == 0:
            raise RuntimeError(f"读不到 FEE0 服务(状态 {int(svc_res.status)})")
        service = svc_res.services[0]
        chars = {}
        for name, u in (("write", WRITE_UUID), ("notify", NOTIFY_UUID)):
            res = await service.get_characteristics_for_uuid_async(uuid.UUID(u))
            if int(res.status) != SUCCESS or res.characteristics.size == 0:
                raise RuntimeError(f"FEE0 下没有 {u[4:8].upper()} 特征值(状态 {int(res.status)})")
            chars[name] = res.characteristics[0]

        def handler(_sender, args):
            reader = DataReader.from_buffer(args.characteristic_value)
            on_notify(bytes(reader.read_byte() for _ in range(args.characteristic_value.length)))

        token = chars["notify"].add_value_changed(handler)
        status = await chars["notify"].write_client_characteristic_configuration_descriptor_async(Cccd.NOTIFY)
        if int(status) != SUCCESS:
            chars["notify"].remove_value_changed(token)
            raise RuntimeError(f"订阅 FEE4 失败(状态 {int(status)})")
        return WinrtSession(device, chars["write"], chars["notify"], token)

    @property
    def connected(self) -> bool:
        return int(self.device.connection_status) == CONNECTED

    async def write(self, payload: bytes) -> None:
        from winrt.windows.devices.bluetooth.genericattributeprofile import GattWriteOption
        from winrt.windows.storage.streams import DataWriter

        writer = DataWriter()
        writer.write_bytes(bytearray(payload))
        res = await self.write_char.write_value_with_result_async(writer.detach_buffer(), GattWriteOption.WRITE_WITH_RESPONSE)
        if int(res.status) != SUCCESS:
            raise RuntimeError(f"写 FEE3 失败(状态 {int(res.status)}, 协议错误码 {res.protocol_error})")

    async def close(self) -> None:
        try:
            self.notify_char.remove_value_changed(self._token)
        except Exception:
            pass
        try:
            self.device.close()
        except Exception:
            pass


class BleChannel:
    """到鼠标蓝牙通道的连接。接口与 HidChannel 对齐: is_present / send_profile / close。

    一个后台线程跑 asyncio 事件循环, 对外是同步方法。会话懒打开, 写失败就丢掉会话下次重开。
    listener(report) 在通知线程里被调用; on_disconnect() 在会话丢失时调用。
    open_session 可注入(测试用), 签名 async (address, on_notify) -> 会话。
    """

    kind = "bluetooth"
    label = "蓝牙"
    supports_polling_rate = False

    def __init__(self, address: str, *, packet_delay_ms: int = 200, listener=None, on_disconnect=None,
                 open_session=None, timeout_s: float = 15.0):
        self.address = parse_address(address)
        self.packet_delay_s = packet_delay_ms / 1000.0
        self.listener = listener
        self.on_disconnect = on_disconnect
        self._open_session = open_session or WinrtSession.open
        self._timeout = timeout_s
        self._lock = threading.Lock()
        self._session = None
        self._loop: asyncio.AbstractEventLoop | None = None
        self._thread: threading.Thread | None = None

    # ---- 事件循环 ----
    def _ensure_loop(self) -> asyncio.AbstractEventLoop:
        if self._loop is None:
            loop = asyncio.new_event_loop()
            self._thread = threading.Thread(target=loop.run_forever, daemon=True, name="ble-loop")
            self._thread.start()
            self._loop = loop
        return self._loop

    def _run(self, coro, timeout: float | None = None):
        fut = asyncio.run_coroutine_threadsafe(coro, self._ensure_loop())
        return fut.result(timeout or self._timeout)

    def _on_notify(self, data: bytes) -> None:
        report = report_from_notification(data)
        if report is not None and self.listener is not None:
            try:
                self.listener(report)
            except Exception:
                LOG.exception("处理蓝牙状态通知出错")

    # ---- 会话 ----
    def _ensure_session(self):
        if self._session is not None and not getattr(self._session, "connected", True):
            self._drop_session()
        if self._session is None:
            self._session = self._run(self._open_session(self.address, self._on_notify))
            LOG.debug("蓝牙会话已建立")
        return self._session

    def _drop_session(self):
        session, self._session = self._session, None
        if session is not None:
            try:
                self._run(session.close(), 5)
            except Exception:
                pass
            if self.on_disconnect is not None:
                self.on_disconnect()

    def connect(self) -> None:
        """建立会话(订阅状态通知); 失败抛异常。"""
        with self._lock:
            self._ensure_session()

    def is_present(self) -> bool:
        """鼠标是否通过蓝牙连着(不打开 FEE0, 所以不会和 HUB 抢)。"""
        if self._session is not None:
            return bool(getattr(self._session, "connected", True))
        try:
            return self._run(self._probe(), 5)
        except Exception:
            return False

    async def _probe(self) -> bool:
        from winrt.windows.devices.bluetooth import BluetoothLEDevice
        device = await BluetoothLEDevice.from_bluetooth_address_async(self.address)
        if device is None:
            return False
        try:
            return int(device.connection_status) == CONNECTED
        finally:
            device.close()

    # ---- 发送 ----
    def send_payloads(self, payloads: list[bytes], delay_s: float | None = None) -> None:
        """依次把载荷写进 FEE3, 间隔 delay_s(默认 200 ms, 与 HUB 一致)。失败丢掉会话并重新抛出。"""
        delay = self.packet_delay_s if delay_s is None else delay_s
        with self._lock:
            try:
                session = self._ensure_session()
                for i, payload in enumerate(payloads):
                    self._run(session.write(bytes(payload)))
                    if i != len(payloads) - 1:
                        time.sleep(delay)
            except Exception:
                self._drop_session()
                raise

    def send_profile(self, profile, delay_s: float | None = None) -> None:
        self.send_payloads(profile.build_payloads(include_polling=False), delay_s)

    def release(self) -> None:
        """放掉 FEE0 会话(让 HUB 能用), 事件循环保留, 下次发送会自动重连。"""
        with self._lock:
            self._drop_session()

    def close(self) -> None:
        with self._lock:
            self._drop_session()
        if self._loop is not None:
            self._loop.call_soon_threadsafe(self._loop.stop)
            if self._thread is not None:
                self._thread.join(timeout=3)
            if not self._loop.is_running():
                self._loop.close()
            self._loop = None
            self._thread = None
