"""Inphic KP 8K 鼠标自制驱动: 逆向官方 HUB, 按前台程序切换配置, 不依赖官方软件。

分层(下层不依赖上层):
    protocol  线上协议: 帧、键码、功能码表、宏编码、状态报告   (纯函数)
    profile   方案: 模型、命令构造、从配置构建
    config    yaml 加载、宏库
    runtime   与设备/系统打交道: HID 通道、前台检测、切换循环
    cli       命令行入口: python -m inphic run | status | macros
"""

__version__ = "0.2.0"
