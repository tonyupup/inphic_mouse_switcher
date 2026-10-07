//! 把整份配置用中文描述出来: 配置文件、鼠标、宏库、切换规则、各方案的实际生效设置。

use std::path::Path;

use crate::config::loader::{AppConfig, Connection};
use crate::profile::describe_profile;

const WIDTH: usize = 72;

/// Python `str.center`: 按字符数居中, 两侧用 `fill` 补满 `WIDTH` 列。
fn center(text: &str, fill: char) -> String {
    let n = text.chars().count();
    let pad = WIDTH.saturating_sub(n);
    let left = pad / 2;
    let mut out = String::new();
    out.extend(std::iter::repeat(fill).take(left));
    out.push_str(text);
    out.extend(std::iter::repeat(fill).take(pad - left));
    out
}

pub fn describe_rules(config: &AppConfig) -> Vec<String> {
    config
        .rules
        .iter()
        .enumerate()
        .map(|(n, rule)| {
            let who = if rule.process == "*" { "其他所有程序".to_string() } else { format!("前台是 {}", rule.process) };
            format!("  {}. {who} → 方案「{}」", n + 1, rule.profile)
        })
        .collect()
}

pub fn describe_connection(config: &AppConfig) -> String {
    let mode = config.device.connection;
    let Some(bt) = config.device.bluetooth.as_ref().filter(|_| mode != Connection::Dongle) else {
        return format!("2.4G 接收器{}", if mode == Connection::Auto { "（蓝牙未配置）" } else { "" });
    };
    let addr = format!("蓝牙 {}", bt.address);
    if mode == Connection::Bluetooth {
        return format!("{addr}（只用蓝牙；蓝牙下回报率设置无效；需先退出 HUB）");
    }
    format!("自动 —— 鼠标连着蓝牙时用 {addr}，否则用 2.4G 接收器（蓝牙下回报率设置无效；蓝牙时需先退出 HUB）")
}

/// 去掉 Windows `canonicalize` 加的 `\\?\` 前缀, 显示更干净。
fn display_path(path: &Path) -> String {
    let p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").map(String::from).unwrap_or(s)
}

pub fn describe_config(config: &AppConfig, config_path: &Path) -> String {
    let dev = &config.device;
    let mut lines = vec![
        center(" inphic 当前设置 ", '='),
        format!("配置文件  {}", display_path(config_path)),
        format!(
            "鼠标      VID 0x{:04X} · PID 0x{:04X} · 配置通道 usage_page 0x{:04X}",
            dev.vendor_id, dev.product_id, dev.usage_page
        ),
        format!("连接方式  {}", describe_connection(config)),
    ];
    if let Some(lib) = &config.macro_library {
        let groups: Vec<String> = lib.groups().iter().map(|(g, names)| format!("{g} {} 个", names.len())).collect();
        lines.push(format!("宏库      共 {} 个宏（{}）", lib.names().len(), groups.join(" · ")));
    }
    lines.push(String::new());
    lines.push("切换规则（从上往下，第一个匹配的生效）:".into());
    lines.extend(describe_rules(config));
    for (name, cp) in &config.profiles {
        lines.push(String::new());
        lines.push(format!("方案「{name}」"));
        lines.extend(describe_profile(&cp.profile).into_iter().map(|l| format!("  {l}")));
    }
    lines.push(String::new());
    lines.push("★ = 与出厂默认不同。未在配置里写的项，显示的是出厂默认值。".into());
    lines.push("=".repeat(WIDTH));
    lines.join("\n")
}
