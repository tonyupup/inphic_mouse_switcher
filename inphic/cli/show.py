"""show 子命令: 用中文打印当前生效的设置。只读, 不连接鼠标, 在没有 hid 的环境里也能用。"""

from __future__ import annotations

from inphic.config import describe_config, load_config


def show(config_path: str) -> int:
    print(describe_config(load_config(config_path), config_path))
    return 0
