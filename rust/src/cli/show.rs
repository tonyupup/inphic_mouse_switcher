//! show 子命令: 用中文打印当前生效的设置。只读, 不连接鼠标, 在任何平台上都能用。
//!
//! `--payloads`: 另外打印每个方案"将要写给鼠标的字节"(干跑, 不发送)——蓝牙下是写进 FEE3 的原始载荷, 2.4G 下是 64 字节帧。

use std::path::Path;

use anyhow::Result;

use crate::config::{describe_config, load_config, AppConfig};
use crate::protocol::hex_spaced;

fn cmd_name(cmd: u8) -> &'static str {
    match cmd {
        0x0C => "开始",
        0x04 => "DPI",
        0x05 => "灯光",
        0x06 => "回报率",
        0x07 => "杂项",
        0x08 => "按键表",
        0x09 => "宏",
        _ => "?",
    }
}

/// 汉字占两列, 对应 Python `f"{s:<4}"` 按字符数补空格(这里同样按字符数)。
fn pad(s: &str, width: usize) -> String {
    let n = s.chars().count();
    format!("{s}{}", " ".repeat(width.saturating_sub(n)))
}

fn describe_payloads(config: &AppConfig) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    for (name, cp) in &config.profiles {
        lines.push(String::new());
        lines.push(format!("方案「{name}」将要发送的字节（干跑，没有发送）:"));
        lines.push("  蓝牙（逐条写进 FEE3，不含回报率命令）:".into());
        for (i, payload) in cp.build_payloads(false)?.iter().enumerate() {
            lines.push(format!(
                "    {:>2}. {} {:>3} 字节  {}",
                i + 1,
                pad(cmd_name(payload[0]), 4),
                payload.len(),
                hex_spaced(payload)
            ));
        }
        let frames = cp.build_packets()?;
        lines.push(format!("  2.4G 接收器（{} 个 64 字节帧，下面只列前 16 字节）:", frames.len()));
        for (i, frame) in frames.iter().enumerate() {
            lines.push(format!("    {:>2}. {} ...", i + 1, hex_spaced(&frame[..16])));
        }
    }
    Ok(lines)
}

pub fn show(config_path: &str, payloads: bool) -> Result<u8> {
    let config = load_config(config_path)?;
    println!("{}", describe_config(&config, Path::new(config_path)));
    if payloads {
        println!("{}", describe_payloads(&config)?.join("\n"));
    }
    Ok(0)
}
