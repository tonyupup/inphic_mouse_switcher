"""
与鼠标的 HID 通信: 按条件找到 HID 集合、打开、串行写帧、检测设备是否还在。

Windows 把鼠标的接口 2 拆成 4 个 HID 集合(Col01–Col04), 按 Report ID 分流:
    Col04 (usage_page 0xFF00)  发配置命令(Report ID 4, 设备的确认应答也是 4)
    Col03 (usage_page 0x000A)  设备主动上报的状态(电量等, Report ID 3)
读错集合会一直收不到数据。
"""

from __future__ import annotations

import threading
import time

import hid

CONFIG_INTERFACE = 2
STATUS_USAGE_PAGE = 0x000A


def find_collection(vendor_id: int, product_id: int, *, usage_page: int,
                    usage: int | None = None, interface: int | None = None) -> bytes | None:
    """返回第一个匹配的 HID 集合路径, 没有则返回 None。"""
    for info in hid.enumerate(vendor_id, product_id):
        if info.get("usage_page") != usage_page:
            continue
        if usage is not None and info.get("usage") != usage:
            continue
        if interface is not None and info.get("interface_number") != interface:
            continue
        return info["path"]
    return None


def open_collection(vendor_id: int, product_id: int, **match):
    """打开匹配的集合并返回 hid.device; 找不到时抛 RuntimeError, 附带检查建议。"""
    path = find_collection(vendor_id, product_id, **match)
    if path is None:
        raise RuntimeError(
            f"找不到 VID={vendor_id:#06x} PID={product_id:#06x} 的 HID 集合 {match}, "
            "检查鼠标/接收器是否已连接"
        )
    dev = hid.device()
    dev.open_path(path)
    return dev


class HidChannel:
    """到鼠标配置集合的连接: 懒打开、串行发送帧、写失败就丢掉句柄下次重开。"""

    def __init__(self, vendor_id: int, product_id: int, usage_page: int, usage: int):
        self.vendor_id = vendor_id
        self.product_id = product_id
        self.usage_page = usage_page
        self.usage = usage
        self._lock = threading.Lock()
        self._dev = None

    def _ensure_open(self):
        if self._dev is None:
            self._dev = open_collection(
                self.vendor_id, self.product_id, usage_page=self.usage_page, usage=self.usage
            )
        return self._dev

    def is_present(self) -> bool:
        """设备是否还在 HID 枚举里(不打开设备, 用于发现拔插/休眠重连)。"""
        try:
            return find_collection(
                self.vendor_id, self.product_id, usage_page=self.usage_page, usage=self.usage
            ) is not None
        except Exception:
            return False

    def send_frames(self, frames: list[bytes], delay_s: float) -> None:
        """依次写入 frames, 帧间隔 delay_s。任何一步失败都丢弃句柄(可能已失效)并重新抛出。"""
        with self._lock:
            try:
                dev = self._ensure_open()
                for i, data in enumerate(frames):
                    # HUB 的 hid_write(h, buf, 0x41): 64 字节帧后多带 1 个 0, 共 65 字节, 这里对齐
                    written = dev.write(data + b"\x00")
                    if written is None or written < 0:
                        raise RuntimeError(f"第 {i + 1}/{len(frames)} 个包写入失败: {dev.error()}")
                    if i != len(frames) - 1:
                        time.sleep(delay_s)
            except Exception:
                self._drop_device()
                raise

    def _drop_device(self):
        if self._dev is not None:
            try:
                self._dev.close()
            except Exception:
                pass
            self._dev = None

    def close(self):
        with self._lock:
            self._drop_device()
