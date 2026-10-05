"""run 子命令: 常驻运行, 按前台程序切换方案。"""

from __future__ import annotations

import logging

from inphic.config import load_config


def run(config_path: str) -> int:
    config = load_config(config_path)          # 先加载配置: 配置错误在碰设备/导入 hid 之前就报
    from inphic.runtime.switcher import LOG, ProfileSwitcher

    logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
    switcher = ProfileSwitcher(config)
    try:
        switcher.run()
    except KeyboardInterrupt:
        LOG.info("退出")
    finally:
        switcher.channel.close()
    return 0
