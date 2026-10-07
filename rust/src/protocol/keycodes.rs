//! 标准 USB HID 键盘 Usage Table, 键名 -> 扫描码。只收录常用的单键。

use std::sync::LazyLock;

use crate::bail;
use crate::error::Result;

pub static HID_KEYCODES: LazyLock<Vec<(String, u8)>> = LazyLock::new(|| {
    let mut t: Vec<(String, u8)> = Vec::new();
    for c in b'a'..=b'z' {
        t.push(((c as char).to_string(), 0x04 + (c - b'a')));
    }
    let fixed: &[(&str, u8)] = &[
        ("1", 0x1E), ("2", 0x1F), ("3", 0x20), ("4", 0x21), ("5", 0x22),
        ("6", 0x23), ("7", 0x24), ("8", 0x25), ("9", 0x26), ("0", 0x27),
        ("enter", 0x28), ("esc", 0x29), ("backspace", 0x2A), ("tab", 0x2B), ("space", 0x2C),
        ("-", 0x2D), ("=", 0x2E), ("[", 0x2F), ("]", 0x30), ("\\", 0x31),
        (";", 0x33), ("'", 0x34), ("`", 0x35), (",", 0x36), (".", 0x37), ("/", 0x38),
        ("capslock", 0x39),
        ("f1", 0x3A), ("f2", 0x3B), ("f3", 0x3C), ("f4", 0x3D), ("f5", 0x3E), ("f6", 0x3F),
        ("f7", 0x40), ("f8", 0x41), ("f9", 0x42), ("f10", 0x43), ("f11", 0x44), ("f12", 0x45),
        ("left_ctrl", 0xE0), ("left_shift", 0xE1), ("left_alt", 0xE2), ("left_win", 0xE3),
        ("printscreen", 0x46), ("scrolllock", 0x47), ("pause", 0x48),
        ("insert", 0x49), ("home", 0x4A), ("pageup", 0x4B), ("delete", 0x4C), ("end", 0x4D), ("pagedown", 0x4E),
        ("right", 0x4F), ("left", 0x50), ("down", 0x51), ("up", 0x52),
        ("numlock", 0x53), ("kp_divide", 0x54), ("kp_multiply", 0x55), ("kp_minus", 0x56), ("kp_plus", 0x57),
        ("kp_enter", 0x58), ("kp_1", 0x59), ("kp_2", 0x5A), ("kp_3", 0x5B), ("kp_4", 0x5C), ("kp_5", 0x5D),
        ("kp_6", 0x5E), ("kp_7", 0x5F), ("kp_8", 0x60), ("kp_9", 0x61), ("kp_0", 0x62), ("kp_period", 0x63),
        ("menu", 0x65),
    ];
    t.extend(fixed.iter().map(|&(k, v)| (k.to_string(), v)));
    for n in 13..=24u8 {
        t.push((format!("f{n}"), 0x68 + (n - 13)));
    }
    let tail: &[(&str, u8)] = &[
        ("right_ctrl", 0xE4), ("right_shift", 0xE5), ("right_alt", 0xE6), ("right_win", 0xE7),
    ];
    t.extend(tail.iter().map(|&(k, v)| (k.to_string(), v)));
    t
});

pub fn key_to_code(key: &str) -> Result<u8> {
    let k = key.trim().to_lowercase();
    match HID_KEYCODES.iter().find(|(name, _)| *name == k) {
        Some(&(_, code)) => Ok(code),
        None => bail!("不认识的键名: {key:?}，可以在 src/protocol/keycodes.rs 里加"),
    }
}

pub fn code_to_key(code: u8) -> String {
    match HID_KEYCODES.iter().find(|(_, c)| *c == code) {
        Some((name, _)) => name.clone(),
        None => format!("未知键码 {code:#04x}"),
    }
}
