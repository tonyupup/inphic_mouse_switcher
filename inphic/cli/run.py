"""run 子命令: 常驻运行, 按前台程序切换方案。"""

from __future__ import annotations

import logging

from inphic.config import describe_config, load_config


def run(config_path: str, verbose: bool = False) -> int:
    config = load_config(config_path)          # 先加载配置: 配置错误在碰设备/导入 hid 之前就报
    from inphic.runtime.switcher import LOG, ProfileSwitcher

    print(describe_config(config, config_path), flush=True)        # 启动时先把当前生效的设置完整打印出来
    logging.basicConfig(level=logging.DEBUG if verbose else logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
    switcher = ProfileSwitcher(config)
    try:
        switcher.run()
    except KeyboardInterrupt:
        LOG.info("退出")
    finally:
        switcher.close()
    return 0
