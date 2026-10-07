//! 配置通道的帧格式: 组帧、解帧、校验。
//!
//! 帧(64 字节, 经 HID 输出报告发给接口 2 的配置集合):
//!
//! ```text
//! [0]        0x04       report id
//! [1]        len + 5    数据长度 + 5
//! [2]        分片序号   普通命令恒 0; 宏消息拆成两片时是 0 和 1
//! [3..3+L)   数据       普通命令里 data[0]=命令字节, data[1]=L
//! [3+L,4+L)  前 L+3 字节的 16 位累加和, 大端
//! 其余补 0 到 64 字节
//! ```
//!
//! 来源: Ghidra 对 INPHIC.exe 的反编译, 并用真实 USB 抓包逐字节验证。

use crate::bail;
use crate::error::Result;

pub const PACKET_LEN: usize = 64;
pub const REPORT_ID: u8 = 0x04;

/// 把 `buf[start..end]` 的 16 位累加和(大端)写进 `buf[at..at+2]`。各命令内部的校验都用它。
pub fn put_sum16(buf: &mut [u8], start: usize, end: usize, at: usize) {
    let s = sum16(&buf[start..end]);
    buf[at] = (s >> 8) as u8;
    buf[at + 1] = (s & 0xFF) as u8;
}

pub fn sum16(data: &[u8]) -> u16 {
    data.iter().fold(0u32, |a, &b| a + b as u32) as u16
}

/// 把 payload(`payload[0]`=命令, `payload[1]`=payload 长度)组成 64 字节帧, 分片序号为 0。
pub fn build_frame(payload: &[u8]) -> Result<Vec<u8>> {
    let length = payload.len();
    if length < 2 || payload[1] as usize != length {
        let got = payload.get(1).map(|b| format!("{b:02x}")).unwrap_or_else(|| "缺失".into());
        bail!("payload[1] 必须等于 payload 长度 {length}, 实际 {got}");
    }
    build_chunk_frame(payload, 0)
}

/// 分片帧: `[04, len+5, 分片序号, 数据, 16 位和]`。普通命令是 index=0 且 data=payload 的特例。
///
/// 宏消息(111 字节)用它拆成两片: 59 字节(序号 0) + 52 字节(序号 1); 抓包验证。
pub fn build_chunk_frame(data: &[u8], index: usize) -> Result<Vec<u8>> {
    if data.len() + 5 > PACKET_LEN {
        bail!("数据太长: {} 字节, 一帧最多 {}", data.len(), PACKET_LEN - 5);
    }
    if index > 0xFF {
        bail!("分片序号 {index} 超出 0..255");
    }
    let mut frame = vec![REPORT_ID, (data.len() + 5) as u8, index as u8];
    frame.extend_from_slice(data);
    let s = sum16(&frame);
    frame.extend_from_slice(&s.to_be_bytes());
    frame.resize(PACKET_LEN, 0);
    Ok(frame)
}

/// 校验帧并返回 (分片序号, 数据); 不合法返回错误。
pub fn parse_chunk_frame(frame: &[u8]) -> Result<(u8, Vec<u8>)> {
    if frame.len() != PACKET_LEN || frame[0] != REPORT_ID {
        bail!("不是合法的 64 字节配置帧");
    }
    let n = frame[1] as i32 - 5;
    if !(0..=(PACKET_LEN as i32 - 5)).contains(&n) {
        bail!("长度字段 {} 不合法", frame[1]);
    }
    let data = frame[3..3 + n as usize].to_vec();
    if build_chunk_frame(&data, frame[2] as usize)? != frame {
        bail!("校验和或补零不对");
    }
    Ok((frame[2], data))
}

/// 校验普通命令帧(分片序号必须是 0, `data[1]` 必须等于长度), 返回 payload。
pub fn parse_frame(frame: &[u8]) -> Result<Vec<u8>> {
    let (index, data) = parse_chunk_frame(frame)?;
    if index != 0 {
        bail!("普通命令帧的分片序号应为 0, 实际 {index}");
    }
    if data.len() < 2 || data[1] as usize != data.len() {
        let got = data.get(1).map(|b| format!("{b:02x}")).unwrap_or_else(|| "缺失".into());
        bail!("长度字段不一致: payload[1]={got}, 实际 {}", data.len());
    }
    Ok(data)
}
