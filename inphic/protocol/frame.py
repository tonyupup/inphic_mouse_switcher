"""
配置通道的帧格式: 组帧、解帧、校验。

帧(64 字节, 经 HID 输出报告发给接口 2 的配置集合):

    [0]       0x04            report id
    [1]       len + 5         数据长度 + 5
    [2]       分片序号        普通命令恒 0; 宏消息拆成两片时是 0 和 1
    [3..3+L)  数据            普通命令里 data[0]=命令字节, data[1]=L
    [3+L, 4+L) 前 L+3 字节的 16 位累加和, 大端
    其余补 0 到 64 字节

来源: Ghidra 对 INPHIC.exe 的反编译, 并用真实 USB 抓包逐字节验证。
"""

from __future__ import annotations

PACKET_LEN = 64
REPORT_ID = 0x04


def put_sum16(buf: bytearray, start: int, end: int, at: int) -> None:
    """把 buf[start:end] 的 16 位累加和(大端)写进 buf[at:at+2]。各命令内部的校验都用它。"""
    s = sum(buf[start:end]) & 0xFFFF
    buf[at] = s >> 8
    buf[at + 1] = s & 0xFF


def build_frame(payload: bytes) -> bytes:
    """把 payload(payload[0]=命令, payload[1]=len(payload)) 组成 64 字节帧, 分片序号为 0。"""
    length = len(payload)
    if length < 2 or payload[1] != length:
        raise ValueError(f"payload[1] 必须等于 payload 长度 {length}, 实际 {payload[1:2].hex() or '缺失'}")
    return build_chunk_frame(payload, 0)


def build_chunk_frame(data: bytes, index: int) -> bytes:
    """分片帧: [04, len+5, 分片序号, 数据, 16 位和]。普通命令是 index=0 且 data=payload 的特例。

    宏消息(111 字节)用它拆成两片: 59 字节(序号 0) + 52 字节(序号 1); 抓包验证。"""
    if len(data) + 5 > PACKET_LEN:
        raise ValueError(f"数据太长: {len(data)} 字节, 一帧最多 {PACKET_LEN - 5}")
    if not 0 <= index <= 0xFF:
        raise ValueError(f"分片序号 {index} 超出 0..255")
    body = bytes([REPORT_ID, len(data) + 5, index]) + bytes(data)
    return (body + (sum(body) & 0xFFFF).to_bytes(2, "big")).ljust(PACKET_LEN, b"\x00")


def parse_chunk_frame(frame: bytes) -> tuple[int, bytes]:
    """校验帧并返回 (分片序号, 数据); 不合法抛 ValueError。"""
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
    """校验普通命令帧(分片序号必须是 0, data[1] 必须等于长度), 返回 payload。"""
    index, data = parse_chunk_frame(frame)
    if index != 0:
        raise ValueError(f"普通命令帧的分片序号应为 0, 实际 {index}")
    if len(data) < 2 or data[1] != len(data):
        raise ValueError(f"长度字段不一致: payload[1]={data[1:2].hex() or '缺失'}, 实际 {len(data)}")
    return data
