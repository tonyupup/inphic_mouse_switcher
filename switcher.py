"""
Inphic 鼠标前台程序自动切配置 - 轻量替代实现

做的事: 监听前台窗口变化, 在后台线程里把对应 profile 的配置命令(6 个包)发给鼠标。
不安装任何键盘/鼠标钩子, 也不常驻 HUB 的 Hook.dll。

注意: 这个脚本是否能避免"切回游戏后键盘短暂失灵"还**没有验证**。HUB 带的 Hook.dll 确实
装全局低级键盘钩子, 但它的 HID 写入跑在独立工作线程上, 所以原来"同步写 HID 卡住钩子线程"
的解释并不成立, 真正原因未确认(见项目 Gitea wiki 的 Home 页)。需要退出 HUB 和 Web 驱动后实测。

协议细节在 mouse_protocol.py(组帧/校验) 和 hub_profile.py(各命令的字段) 里;
config.hub.yaml 是语义配置示例, 不需要看到任何十六进制。
"""

import ctypes
import ctypes.wintypes as wt
import logging
import sys
import threading
import time
from pathlib import Path

import hid
import psutil

from config_loader import AppConfig, load_config
from mouse_protocol import Profile

LOG = logging.getLogger("mouse_switcher")

user32 = ctypes.windll.user32


def get_foreground_process_name() -> str | None:
    hwnd = user32.GetForegroundWindow()
    if not hwnd:
        return None
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    if not pid.value:
        return None
    try:
        return psutil.Process(pid.value).name().lower()
    except (psutil.NoSuchProcess, psutil.AccessDenied):
        return None


class HidChannel:
    """管理到鼠标厂商自定义 HID collection 的连接，串行发送原始报文。"""

    def __init__(self, vendor_id: int, product_id: int, usage_page: int, usage: int):
        self.vendor_id = vendor_id
        self.product_id = product_id
        self.usage_page = usage_page
        self.usage = usage
        self._lock = threading.Lock()
        self._dev = None

    def _find_path(self) -> bytes:
        for info in hid.enumerate(self.vendor_id, self.product_id):
            if info.get("usage_page") == self.usage_page and info.get("usage") == self.usage:
                return info["path"]
        raise RuntimeError(
            f"找不到 VID={self.vendor_id:#06x} PID={self.product_id:#06x} "
            f"usage_page={self.usage_page:#06x} usage={self.usage} 的HID设备，"
            f"检查鼠标是否插好、线缆是否接触良好"
        )

    def _ensure_open(self):
        if self._dev is None:
            dev = hid.device()
            dev.open_path(self._find_path())
            self._dev = dev
        return self._dev

    def is_present(self) -> bool:
        """设备是否还在 HID 枚举里(不打开设备, 用于发现拔插/休眠重连)。"""
        try:
            return any(
                info.get("usage_page") == self.usage_page and info.get("usage") == self.usage
                for info in hid.enumerate(self.vendor_id, self.product_id)
            )
        except Exception:
            return False

    def send_profile(self, profile: Profile, delay_s: float):
        with self._lock:
            try:
                dev = self._ensure_open()
                packets = profile.build_packets()
                for i, data in enumerate(packets):
                    # HUB 的 hid_write(h, buf, 0x41): 64 字节帧后多带 1 个 0, 共 65 字节, 这里对齐
                    written = dev.write(data + b"\x00")
                    if written is None or written < 0:
                        raise RuntimeError(f"第 {i + 1}/{len(packets)} 个包写入失败: {dev.error()}")
                    if i != len(packets) - 1:
                        time.sleep(delay_s)
            except Exception:
                self._drop_device()   # 句柄可能已失效(拔插/休眠), 丢掉, 下次重新打开
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


class ProfileSwitcher:
    RETRY_DELAY_S = 3.0        # 应用失败后多久重试
    PRESENCE_CHECK_S = 2.0     # 多久检查一次设备是否还在

    def __init__(self, config: AppConfig):
        self.config = config
        dev_cfg = config.device
        self.channel = HidChannel(
            dev_cfg["vendor_id"], dev_cfg["product_id"], dev_cfg["usage_page"], dev_cfg["usage"]
        )
        self.desired = None            # 按前台程序应该用的方案
        self.applied = None            # 已经成功写进鼠标的方案
        self._applying = False
        self._retry_at = 0.0
        self._state_lock = threading.Lock()
        self._was_present = True
        self._next_presence_check = 0.0

    def _start_apply(self, name: str):
        profile = self.config.profiles.get(name)
        if profile is None:
            LOG.warning("配置里没有名为 %s 的 profile", name)
            self.desired = None
            return
        delay_s = self.config.packet_delay_ms / 1000.0
        self._applying = True

        def worker():
            try:
                LOG.info("切换到 profile: %s (按钮: %s)", name, profile.current_buttons())
                self.channel.send_profile(profile, delay_s)
                with self._state_lock:
                    self.applied = name
                LOG.info("profile %s 应用完成", name)
            except Exception:
                LOG.exception("应用 profile %s 失败, %.0f 秒后重试", name, self.RETRY_DELAY_S)
                self._retry_at = time.monotonic() + self.RETRY_DELAY_S
            finally:
                self._applying = False

        threading.Thread(target=worker, daemon=True, name=f"apply-{name}").start()

    def _maybe_apply(self):
        if self._applying or not self.desired or self.desired == self.applied:
            return
        if time.monotonic() < self._retry_at:
            return
        self._start_apply(self.desired)

    def _poll_presence(self, now: float) -> None:
        """定期检查设备是否还在; 断开后再出现就清掉 applied, 触发重新应用。"""
        if now < self._next_presence_check or self._applying:
            return
        self._next_presence_check = now + self.PRESENCE_CHECK_S
        present = self.channel.is_present()
        if present and not self._was_present:
            LOG.info("检测到鼠标重新连接, 重新应用当前方案")
            with self._state_lock:
                self.applied = None
        elif self._was_present and not present:
            LOG.warning("鼠标已断开, 等待重新连接")
        self._was_present = present

    def run(self):
        poll_interval = self.config.poll_interval_ms / 1000.0
        last_process = object()
        LOG.info("开始监听前台窗口切换 (Ctrl+C 退出)")
        while True:
            process_name = get_foreground_process_name()
            if process_name != last_process:
                last_process = process_name
                LOG.debug("前台进程: %s", process_name)
                profile_name = self.config.resolve_profile_name(process_name)
                if profile_name:
                    self.desired = profile_name
            self._poll_presence(time.monotonic())
            if self._was_present:
                self._maybe_apply()
            time.sleep(poll_interval)


def main():
    logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
    config_path = sys.argv[1] if len(sys.argv) > 1 else Path(__file__).parent / "config.yaml"
    config = load_config(config_path)
    switcher = ProfileSwitcher(config)
    try:
        switcher.run()
    except KeyboardInterrupt:
        LOG.info("退出")
    finally:
        switcher.channel.close()


if __name__ == "__main__":
    main()
