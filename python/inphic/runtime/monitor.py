"""
状态监听: 后台线程读鼠标主动上报的状态报告(约每 2 秒一个), 电量变化时写日志。

- 单独打开状态集合(Col03, 只读), 和写配置的句柄(Col04)互不干扰。
- 设备拔出/休眠后自动重连, 重连后的第一个读数会重新写一条日志。
- 日志只在值变化时写, 不会每 2 秒一行: 启动后的第一个读数、电量百分比变化、状态码变化、
  降到 20% / 10% 以下时的低电量警告(充电回升后再降下来会再次警告)。
"""

from __future__ import annotations

import logging
import threading

from inphic.protocol.status import Battery, parse_report

LOG = logging.getLogger("inphic.monitor")

LOW_BATTERY_LEVELS = (20, 10)


def _open_status_collection(vendor_id: int, product_id: int):
    from inphic.runtime.channel import CONFIG_INTERFACE, STATUS_USAGE_PAGE, open_collection   # 延迟导入: 需要 hid

    return open_collection(vendor_id, product_id, usage_page=STATUS_USAGE_PAGE, interface=CONFIG_INTERFACE)


class BatteryTracker:
    """判断每个电量读数要写哪些日志。纯逻辑, 返回 [(日志级别, 文字), ...]。"""

    def __init__(self):
        self._last: Battery | None = None
        self._warned: set[int] = set()

    def reset(self) -> None:
        """设备重连后调用: 下一个读数当作"第一个读数"处理。"""
        self._last = None

    def update(self, bat: Battery) -> list[tuple[str, str]]:
        out: list[tuple[str, str]] = []
        last = self._last
        if last is None:
            out.append(("info", f"电量 {bat.percent}%（状态码 {bat.state}）"))
        else:
            if bat.percent != last.percent:
                out.append(("info", f"电量 {last.percent}% → {bat.percent}%"))
            if bat.state != last.state:
                out.append(("info", f"电量状态码 {last.state} → {bat.state}（含义未确认，可能是充电状态变化）"))
        for level in LOW_BATTERY_LEVELS:
            if bat.percent > level:
                self._warned.discard(level)              # 充上去了, 下次再降下来要再警告
            elif level not in self._warned:
                self._warned.add(level)
                out.append(("warning", f"电量低: {bat.percent}%（已不高于 {level}%），请及时充电"))
        self._last = bat
        return out


class StatusMonitor:
    """后台线程: 打开状态集合, 把解析出的 StatusReport 交给 on_report。"""

    def __init__(self, vendor_id: int, product_id: int, on_report, *, on_raw=None, on_disconnect=None,
                 open_device=None, retry_s: float = 3.0, poll_ms: int = 500):
        self._on_report = on_report
        self._on_raw = on_raw
        self._on_disconnect = on_disconnect
        self._open = open_device or (lambda: _open_status_collection(vendor_id, product_id))
        self._retry_s = retry_s
        self._poll_ms = poll_ms
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        if self._thread is None:
            self._thread = threading.Thread(target=self._run, daemon=True, name="status-monitor")
            self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=3)
            self._thread = None

    def _run(self) -> None:
        waiting_logged = False
        while not self._stop.is_set():
            try:
                dev = self._open()
            except Exception:
                if not waiting_logged:
                    LOG.warning("状态监听: 找不到鼠标的状态通道, 每 %.0f 秒重试", self._retry_s)
                    waiting_logged = True
                self._stop.wait(self._retry_s)
                continue
            waiting_logged = False
            LOG.debug("状态监听: 已连接")
            try:
                while not self._stop.is_set():
                    data = dev.read(64, self._poll_ms)
                    if not data:
                        continue
                    rep = parse_report(data)
                    if rep is not None:
                        self._on_report(rep)
                    elif self._on_raw is not None:           # 不是状态报告的数据也不丢: 交给调用方记录
                        self._on_raw(bytes(data))
            except Exception:
                LOG.warning("状态监听: 读取失败(鼠标断开或休眠?), 等待重连")
            finally:
                try:
                    dev.close()
                except Exception:
                    pass
                if self._on_disconnect is not None:
                    self._on_disconnect()
            self._stop.wait(self._retry_s)
