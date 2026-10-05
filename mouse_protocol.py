"""
Inphic KP 8K 鼠标的配置包协议 —— 面向对象封装。

背景: 用 Ghidra 反编译官方驱动确认了校验和算法（对前62字节求和，
取低字节写入第64字节），用抓包实测确认了按键表的 3 个槽位偏移。
这一层把"按钮 N 绑定成某个键/某个内置功能"这种人类可读的描述，
编码成设备认的原始字节，调用方（switcher.py）完全不用看到十六进制。
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Union

from hid_keycodes import key_to_code, code_to_key

PACKET_LEN = 64

# 帧格式(Ghidra 反编译 hid_dispatch_cmd 的 0x82 分支 + 47 个抓包验证, 全部吻合):
#   [0]=0x04 report id   [1]=L+5   [2]=0x00
#   [3..3+L) = payload, payload[0]=命令字节, payload[1]=L(payload 总长)
#   [3+L], [4+L] = 前 L+3 个字节的 16 位累加和, 大端
#   其余补 0 到 64 字节
REPORT_ID = 0x04
CMD_KEYMAP = 0x08  # 按键表包(第6个包), payload 长 59

# 按键表包额外有一个内层 16 位校验: buf[60:62] = sum(buf[6:60]), 大端
# (来自 FUN_004620f0: payload[57:59] = sum(payload[3:57]); 8 种按键表上全部验证过。
#  之前误把 buf[60] 当成"含义未知的字段", 其实它就是这个 16 位和的高字节)
KEYMAP_INNER_SUM_RANGE = (6, 60)
KEYMAP_INNER_SUM_POS = 60

# 按键表包(第6个包)里，每个按钮占3字节，这3个偏移是抓包+改绑实测确认的，
# 物理按钮对应关系也实测确认过了。鼠标上还有没测到的按钮(左右键等)，
# 这里先不收录。
BUTTON_SLOT_OFFSETS = {
    "middle_click": 12,  # 中键
    "side_back": 24,     # 侧键(后)
    "side_front": 27,    # 侧键(前)
}

KEY_ACTION_TYPE = 0x11  # 该字节表示"这个按钮模拟一次键盘按键"


@dataclass(frozen=True)
class KeyAction:
    """按钮绑定为模拟一次键盘按键，比如 KeyAction("f")。"""

    key: str

    def encode(self) -> bytes:
        return bytes([KEY_ACTION_TYPE, 0x00, key_to_code(self.key)])

    @staticmethod
    def decode(raw: bytes) -> "KeyAction | None":
        if raw[0] == KEY_ACTION_TYPE:
            return KeyAction(code_to_key(raw[2]))
        return None

    def __repr__(self):
        return f"KeyAction(key={self.key!r})"


@dataclass(frozen=True)
class MouseFunction:
    """
    按钮绑定为鼠标内置功能(不是模拟键盘)，比如 DPI 切换/前进/后退。
    具体 id 对应哪个功能目前只是从抓包里原样记录下来的，还没有逐个
    验证过名字，所以用数字 id，想确认含义可以把某个按钮改绑成不同
    id 后实际按一下看鼠标做了什么，然后在 FUNCTION_NAMES 里补上名字。
    """

    function_id: int

    def encode(self) -> bytes:
        return bytes([self.function_id, 0x00, 0x00])

    @staticmethod
    def decode(raw: bytes) -> "MouseFunction | None":
        if raw[0] != KEY_ACTION_TYPE and raw[0] != 0x3C:
            return MouseFunction(raw[0])
        return None

    def __repr__(self):
        name = FUNCTION_NAMES.get(self.function_id)
        label = f"{self.function_id:#04x}" + (f" ({name})" if name else "")
        return f"MouseFunction(id={label})"


# 确认了含义之后往这里加，比如 FUNCTION_NAMES[4] = "DPI+"
FUNCTION_NAMES: dict[int, str] = {}

ButtonAction = Union[KeyAction, MouseFunction]


class ButtonTable:
    """三个已确认按钮槽位的绑定集合。"""

    def __init__(self, bindings: dict[str, ButtonAction]):
        unknown = set(bindings) - set(BUTTON_SLOT_OFFSETS)
        if unknown:
            raise ValueError(f"不认识的按钮名: {unknown}，可选: {list(BUTTON_SLOT_OFFSETS)}")
        self.bindings = bindings

    def apply_to(self, packet: bytearray) -> None:
        for name, action in self.bindings.items():
            offset = BUTTON_SLOT_OFFSETS[name]
            packet[offset:offset + 3] = action.encode()

    @staticmethod
    def read_from(packet: bytes) -> dict[str, ButtonAction]:
        result = {}
        for name, offset in BUTTON_SLOT_OFFSETS.items():
            raw = packet[offset:offset + 3]
            action = KeyAction.decode(raw) or MouseFunction.decode(raw)
            if action is not None:
                result[name] = action
        return result


def build_frame(payload: bytes) -> bytes:
    """把 payload(payload[0]=命令, payload[1]=len(payload)) 组成 64 字节的 HID 输出帧。"""
    length = len(payload)
    if length < 2 or payload[1] != length:
        raise ValueError(f"payload[1] 必须等于 payload 长度 {length}, 实际 {payload[1:2].hex() or '缺失'}")
    if length + 5 > PACKET_LEN:
        raise ValueError(f"payload 太长: {length} 字节, 最多 {PACKET_LEN - 5}")
    body = bytes([REPORT_ID, length + 5, 0x00]) + payload
    return (body + (sum(body) & 0xFFFF).to_bytes(2, "big")).ljust(PACKET_LEN, b"\x00")


def build_chunk_frame(data: bytes, index: int) -> bytes:
    """分片帧: [04, len+5, 分片序号, 数据, 16 位和]。普通命令是 index=0 且 data=payload 的特例。

    宏消息(111 字节)就是用它拆成两片: 59 字节(序号 0) + 52 字节(序号 1); 抓包验证。"""
    if len(data) + 5 > PACKET_LEN:
        raise ValueError(f"分片数据太长: {len(data)} 字节, 最多 {PACKET_LEN - 5}")
    if not 0 <= index <= 0xFF:
        raise ValueError(f"分片序号 {index} 超出 0..255")
    body = bytes([REPORT_ID, len(data) + 5, index]) + bytes(data)
    return (body + (sum(body) & 0xFFFF).to_bytes(2, "big")).ljust(PACKET_LEN, b"\x00")


def parse_chunk_frame(frame: bytes) -> tuple[int, bytes]:
    """校验分片帧并返回 (分片序号, 数据)。"""
    if len(frame) != PACKET_LEN or frame[0] != REPORT_ID:
        raise ValueError("不是合法的 64 字节配置帧")
    n = frame[1] - 5
    if not 0 <= n <= PACKET_LEN - 5:
        raise ValueError(f"长度字段 {frame[1]} 不合法")
    data = bytes(frame[3:3 + n])
    if build_chunk_frame(data, frame[2]) != bytes(frame):
        raise ValueError("校验和或补零不对")
    return frame[2], data


def parse_frame(frame: bytes) -> bytes:
    """校验帧格式和外层校验和, 返回 payload; 不合法就抛 ValueError。"""
    if len(frame) != PACKET_LEN or frame[0] != REPORT_ID or frame[2] != 0x00:
        raise ValueError("不是合法的 64 字节配置帧")
    length = frame[4]
    if frame[1] != length + 5 or length + 5 > PACKET_LEN:
        raise ValueError(f"长度字段不一致: [1]={frame[1]} [4]={length}")
    if build_frame(frame[3:3 + length]) != bytes(frame):
        raise ValueError("外层校验和或补零不对")
    return bytes(frame[3:3 + length])


def seal_frame(packet: bytearray) -> None:
    """改完内容后就地重算校验: 按键表包先更新内层校验, 然后重算外层 16 位校验。"""
    if packet[3] == CMD_KEYMAP:
        start, end = KEYMAP_INNER_SUM_RANGE
        packet[KEYMAP_INNER_SUM_POS:KEYMAP_INNER_SUM_POS + 2] = (sum(packet[start:end]) & 0xFFFF).to_bytes(2, "big")
    length = packet[4]
    packet[length + 3:length + 5] = (sum(packet[:length + 3]) & 0xFFFF).to_bytes(2, "big")


class Profile:
    """
    一个完整的配置方案：一组固定的模板包(DPI/灯效等，来自某次干净的抓包)
    + 可选的按钮绑定(覆盖模板包里最后一个包的按钮表部分)。
    """

    def __init__(self, name: str, template_packets: list[str], buttons: ButtonTable | None = None):
        self.name = name
        self.template_packets = template_packets  # 十六进制字符串列表，只在这一层出现
        self.buttons = buttons

    def build_packets(self) -> list[bytes]:
        packets = [bytearray.fromhex(p) for p in self.template_packets]
        if self.buttons is not None:
            keymap_packet = packets[-1]
            self.buttons.apply_to(keymap_packet)
            seal_frame(keymap_packet)
        return [bytes(p) for p in packets]

    def current_buttons(self) -> dict[str, ButtonAction]:
        if self.buttons is not None:
            return self.buttons.bindings
        return ButtonTable.read_from(bytes.fromhex(self.template_packets[-1]))
