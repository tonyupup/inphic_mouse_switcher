"""
命令构造器: 一个方案 -> 设备命令 payload -> 64 字节帧。

    FUN_00461760 -> 0x04 (DPI 表)       FUN_00461ac0 -> 0x05 (灯光)
    FUN_00461be0 -> 0x06 (回报率)       FUN_004612a0 内联 -> 0x0c (开始) / 0x07
    FUN_004620f0 -> 0x08 (按键表; payload[57:59] 是 payload[3:57] 的 16 位和)

全部来自 Ghidra 对 INPHIC.exe 的反编译, 并用真实抓包逐字节验证过(见 tests/profile)。
"""

from __future__ import annotations

from inphic.profile.model import (
    BUTTON_COUNT, DPI_LEVELS, KEY_FUNCTION_ID, MACRO_FUNCTION_ID, MACRO_VARIANT_ID, POLLING_RATES, HubProfile,
)
from inphic.protocol.frame import build_frame, put_sum16
from inphic.protocol.functions import FUNCTION_WIRE
from inphic.protocol.macro import SLOT_BASE, macro_frames


# ---------------- 命令构造器: 都返回 payload (payload[0]=命令, payload[1]=长度) ----------------

def cmd_begin() -> bytes:
    """命令 0x0c: 开始写入, 常量。"""
    return bytes([0x0C, 0x0A, 0x01, 0xFE, 0x01, 0xFE, 0, 0, 0, 0])


def cmd_polling_rate(p: HubProfile) -> bytes:
    """命令 0x06: 回报率。payload[3]=码, payload[4]=~码。"""
    idx = p.polling_rate_index
    if idx not in POLLING_RATES:
        raise ValueError(f"回报率序号 {idx} 不在 0..6")
    code = POLLING_RATES[idx][1]
    return bytes([0x06, 0x09, 0x01, code, (~code) & 0xFF, 0, 0, 0, 0])


def cmd_misc07(p: HubProfile) -> bytes:
    """命令 0x07: B 取自 +0x289c(含义未知, 抓包恒为 1)。"""
    b = p.u8(0x289C)
    return bytes([0x07, 0x08, b, (~b) & 0xFF, 0x00, 0xFF, 0, 0])


def cmd_light(p: HubProfile) -> bytes:
    """命令 0x05: 灯光。字段含义为按偏移和取值推断。"""
    mode = p.i32(0x288C)
    c = p.u8(0x286C)                       # 取 int 的低字节
    pl = bytearray(15)
    pl[0], pl[1], pl[2] = 0x05, 0x0F, 0x01
    pl[3] = (mode << 4) & 0xFF
    pl[4] = (c & 0xF0) | ((9 - p.u8(0x2868)) & 0xFF)
    pl[5] = ((c << 4) & 0xFF) | (p.u8(0x2864) if mode in (1, 5) else 8)
    pl[6], pl[7], pl[8] = p.light_rgb
    pl[9] = p.u8(0x2870)
    pl[10] = p.u8(0x2898)
    put_sum16(pl, 3, 11, 11)
    return bytes(pl)


def cmd_dpi(p: HubProfile) -> bytes:
    """命令 0x04: DPI 表。8 档低字节/高字节分开放, 8 档 RGB, 末尾 16 位和。"""
    pl = bytearray(0x38)
    pl[0], pl[1], pl[2] = 0x04, 0x38, 0x01
    pl[3] = p.u8(0x2884)
    pl[4] = p.u8(0x2880)
    mask = 0
    for i in range(DPI_LEVELS):
        if p.dpi_enabled(i):
            mask |= 1 << i
    pl[5] = mask
    pl[6] = p.u8(0x2884)
    pl[7] = p.u8(0x2888)
    for i in range(DPI_LEVELS):
        v = p.dpi_value(i) - 1
        if v >= 0:
            pl[8 + i] = v & 0xFF
            pl[16 + i] = (v >> 8) & 0xFF
    pl[24] = (p.i32(0x2874) + 1) & 0xFF
    for i in range(DPI_LEVELS):
        pl[25 + 3 * i:28 + 3 * i] = bytes(p.dpi_color(i))
    pl[49] = 1
    put_sum16(pl, 3, 50, 50)
    return bytes(pl)


def cmd_keymap(p: HubProfile) -> bytes:
    """命令 0x08: 按键表, 18 个按钮 x 3 字节, 末尾 payload[57:59] 为 payload[3:57] 的 16 位和(大端)。"""
    pl = bytearray(0x3B)
    pl[0], pl[1], pl[2] = 0x08, 0x3B, 0x01
    for i in range(BUTTON_COUNT):
        rec = p.button_record(i)
        fid = rec[1]
        if fid == MACRO_VARIANT_ID:
            raise NotImplementedError("宏的\"左键按下播放\"变体(功能 id 0x18)没有抓包, 不支持")
        b0, b1, b2 = FUNCTION_WIRE.get(fid, (0, 0, 0))
        if fid == MACRO_FUNCTION_ID:
            if i not in p.macros:
                raise ValueError(f"按钮 {i} 绑定了宏但没有宏内容, 请用 set_button_macro / type: macro")
            b1, b2 = 0, SLOT_BASE[i]                    # 线上: [0x12, 0x00, 槽位]  (抓包: 12 00 1a / 12 00 15)
        if fid == KEY_FUNCTION_ID:
            b1, b2 = rec[7], rec[3]
        pl[3 + 3 * i:6 + 3 * i] = bytes((b0, b1, b2))
    put_sum16(pl, 3, 57, 57)
    return bytes(pl)


def build_apply_sequence(p: HubProfile) -> list[bytes]:
    """完整应用一个方案: 与 FUN_004612a0(which=0) 的发送顺序一致, 返回 64 字节帧列表。

    注意顺序: 0x0c, 0x04, 0x05, 0x06, 0x07, 0x08 —— 与抓包里的包序一致(包序在抓包里是
    0c,04,05,06,07,08)。宏数据(FUN_00461c90)接在最后。
    """
    payloads = [cmd_begin(), cmd_dpi(p), cmd_light(p), cmd_polling_rate(p), cmd_misc07(p), cmd_keymap(p)]
    frames = [build_frame(pl) for pl in payloads]
    # 宏数据放在最后, 按按钮序号(与 HUB 一致); 抓包里宏帧也是紧跟在按键表之后
    for i in sorted(p.macros):
        mc = p.macros[i]
        frames += macro_frames(list(mc.events), SLOT_BASE[i], mc.repeat, mc.mode)
    return frames
