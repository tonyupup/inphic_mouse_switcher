"""前台窗口检测(仅 Windows)。只读轮询, 不安装任何键盘/鼠标钩子。"""

from __future__ import annotations


def get_foreground_process_name() -> str | None:
    """前台窗口所属进程的名字(小写, 如 "crossfire.exe"); 取不到返回 None。"""
    import ctypes
    import ctypes.wintypes as wt

    import psutil

    user32 = ctypes.windll.user32
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
