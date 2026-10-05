"""
命令行入口:

    python -m inphic run     [-c config.yaml]            按前台程序切换方案(要常驻运行); 启动时先打印当前设置
    python -m inphic show    [-c config.yaml] [--payloads]  只打印当前设置(中文), 不连接鼠标; --payloads 另打印将发送的字节(干跑)
    python -m inphic status  [-c config.yaml] [--watch] [--via bluetooth]  读电量等状态(只读)
    python -m inphic macros list|show <分组.名字> [-c config.yaml]   查看宏库(只读)
"""

from __future__ import annotations

import argparse
import sys

DEFAULT_CONFIG = "config.yaml"


def build_parser() -> argparse.ArgumentParser:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("-c", "--config", default=DEFAULT_CONFIG, help=f"配置文件(默认 {DEFAULT_CONFIG})")

    parser = argparse.ArgumentParser(prog="inphic", description="Inphic KP 8K 鼠标自制驱动")
    sub = parser.add_subparsers(dest="command", required=True, metavar="{run,show,status,macros}")

    sub.add_parser("run", parents=[common], help="按前台程序自动切换鼠标方案")
    p_cfg = sub.add_parser("show", parents=[common], help="用中文打印当前设置, 不连接鼠标")
    p_cfg.add_argument("--payloads", action="store_true", help="另外打印每个方案将要发送的字节(干跑, 不发送)")

    p_status = sub.add_parser("status", parents=[common], help="读取鼠标状态(电量等), 只读")
    p_status.add_argument("--via", choices=("auto", "dongle", "bluetooth"), default="auto",
                          help="从哪条连接读: 默认有接收器读接收器, 否则读蓝牙")
    p_status.add_argument("--watch", action="store_true", help="持续打印全部状态报告(观察 DPI 键等变化)")

    p_macros = sub.add_parser("macros", help="查看宏库, 只读")
    msub = p_macros.add_subparsers(dest="macros_command", required=True, metavar="{list,show}")
    msub.add_parser("list", parents=[common], help="列出所有分组和宏")
    p_show = msub.add_parser("show", parents=[common], help="展开一个宏: 步骤、槽位、时长")
    p_show.add_argument("ref", help="分组.名字, 如 fps.rapid_fire")
    return parser


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "run":
            from inphic.cli.run import run
            return run(args.config)
        if args.command == "show":
            from inphic.cli.show import show
            return show(args.config, payloads=args.payloads)
        if args.command == "status":
            from inphic.cli.status import status
            return status(args.config, watch=args.watch, via=args.via)
        from inphic.cli.macros import macros_list, macros_show
        return macros_list(args.config) if args.macros_command == "list" else macros_show(args.config, args.ref)
    except ValueError as e:                       # 配置/宏库里的错误: 显示成一句人话, 不要 traceback
        print(f"配置错误: {e}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        return 130
