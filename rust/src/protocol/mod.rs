//! 线上协议: 帧、键码、功能码表、宏编码、状态报告。纯函数, 不碰设备、不读配置。

pub mod bluetooth;
pub mod frame;
pub mod functions;
pub mod keycodes;
pub mod macros;
pub mod status;

/// 十六进制串(允许空格) -> 字节。只用于内置常量和测试夹具, 输入不合法直接 panic。
pub fn hex_decode(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    assert!(digits.len() % 2 == 0, "十六进制串长度必须是偶数: {s}");
    digits
        .chunks(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).expect("不是十六进制"))
        .collect()
}

/// 字节 -> 用空格分隔的小写十六进制, 如 `93 40 01 42`。
pub fn hex_spaced(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}
