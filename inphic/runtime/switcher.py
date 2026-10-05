"""
前台程序切换方案: 监听前台窗口变化, 在后台线程里把对应方案的命令帧发给鼠标。

不安装任何键盘/鼠标钩子, 也不常驻 HUB 的 Hook.dll。
实测(用户确认): 用本程序后切回游戏时键盘不再短暂失灵; HUB 的 Hook.dll 装有全局低级键盘钩子,
但它的 HID 写入跑在独立工作线程上, 具体机制未查清(见项目 Gitea wiki 的 Home 页)。

可靠性: 写失败就丢掉句柄、每 3 秒重试; 鼠标拔出再插回会重新应用当前方案。
"""

from __future__ import annotations

import logging
import threading
import time

from inphic.config import AppConfig
from inphic.runtime.channel import HidChannel
from inphic.runtime.foreground import get_foreground_process_name

LOG = logging.getLogger("inphic.switcher")


class ProfileSwitcher:
    RETRY_DELAY_S = 3.0        # 应用失败后多久重试
    PRESENCE_CHECK_S = 2.0     # 多久检查一次设备是否还在

    def __init__(self, config: AppConfig, foreground=get_foreground_process_name):
        self.config = config
        self._foreground = foreground
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
                self.channel.send_frames(profile.build_packets(), delay_s)
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
            process_name = self._foreground()
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
