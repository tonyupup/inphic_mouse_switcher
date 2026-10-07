//! 命令构造器: 一个方案 -> 设备命令 payload -> 64 字节帧。
//!
//! ```text
//! FUN_00461760 -> 0x04 (DPI 表)       FUN_00461ac0 -> 0x05 (灯光)
//! FUN_00461be0 -> 0x06 (回报率)       FUN_004612a0 内联 -> 0x0c (开始) / 0x07
//! FUN_004620f0 -> 0x08 (按键表; payload[57:59] 是 payload[3:57] 的 16 位和)
//! ```
//!
//! 全部来自 Ghidra 对 INPHIC.exe 的反编译, 并用真实抓包逐字节验证过(见 tests/)。

use crate::bail;
use crate::error::Result;
use crate::profile::model::{
    HubProfile, BUTTON_COUNT, DPI_LEVELS, KEY_FUNCTION_ID, MACRO_FUNCTION_ID, MACRO_VARIANT_ID, POLLING_RATES,
};
use crate::protocol::frame::{build_frame, put_sum16};
use crate::protocol::functions::function_wire;
use crate::protocol::macros::{encode_macro, message_frames, slot_base};

// ---------------- 命令构造器: 都返回 payload (payload[0]=命令, payload[1]=长度) ----------------

/// 命令 0x0c: 开始写入, 常量。
pub fn cmd_begin() -> Vec<u8> {
    vec![0x0C, 0x0A, 0x01, 0xFE, 0x01, 0xFE, 0, 0, 0, 0]
}

/// 命令 0x06: 回报率。payload[3]=码, payload[4]=~码。
pub fn cmd_polling_rate(p: &HubProfile) -> Result<Vec<u8>> {
    let idx = p.polling_rate_index();
    let Some(&(_, code)) = usize::try_from(idx).ok().and_then(|i| POLLING_RATES.get(i)) else {
        bail!("回报率序号 {idx} 不在 0..6");
    };
    Ok(vec![0x06, 0x09, 0x01, code, !code, 0, 0, 0, 0])
}

/// 命令 0x07: B 取自 +0x289c(含义未知, 抓包恒为 1)。
pub fn cmd_misc07(p: &HubProfile) -> Vec<u8> {
    let b = p.u8(0x289C);
    vec![0x07, 0x08, b, !b, 0x00, 0xFF, 0, 0]
}

/// 命令 0x05: 灯光。字段含义为按偏移和取值推断。
pub fn cmd_light(p: &HubProfile) -> Vec<u8> {
    let mode = p.i32(0x288C);
    let c = p.u8(0x286C); // 取 int 的低字节
    let mut pl = vec![0u8; 15];
    pl[0] = 0x05;
    pl[1] = 0x0F;
    pl[2] = 0x01;
    pl[3] = ((mode << 4) & 0xFF) as u8;
    pl[4] = (c & 0xF0) | ((9i32 - p.u8(0x2868) as i32) & 0xFF) as u8;
    pl[5] = (c << 4) | if mode == 1 || mode == 5 { p.u8(0x2864) } else { 8 };
    let (r, g, b) = p.light_rgb();
    pl[6] = r;
    pl[7] = g;
    pl[8] = b;
    pl[9] = p.u8(0x2870);
    pl[10] = p.u8(0x2898);
    put_sum16(&mut pl, 3, 11, 11);
    pl
}

/// 命令 0x04: DPI 表。8 档低字节/高字节分开放, 8 档 RGB, 末尾 16 位和。
pub fn cmd_dpi(p: &HubProfile) -> Vec<u8> {
    let mut pl = vec![0u8; 0x38];
    pl[0] = 0x04;
    pl[1] = 0x38;
    pl[2] = 0x01;
    pl[3] = p.u8(0x2884);
    pl[4] = p.u8(0x2880);
    let mut mask = 0u8;
    for i in 0..DPI_LEVELS {
        if p.dpi_enabled(i) {
            mask |= 1 << i;
        }
    }
    pl[5] = mask;
    pl[6] = p.u8(0x2884);
    pl[7] = p.u8(0x2888);
    for i in 0..DPI_LEVELS {
        let v = p.dpi_value(i) as i64 - 1;
        if v >= 0 {
            pl[8 + i] = (v & 0xFF) as u8;
            pl[16 + i] = ((v >> 8) & 0xFF) as u8;
        }
    }
    pl[24] = ((p.i32(0x2874) as i64 + 1) & 0xFF) as u8;
    for i in 0..DPI_LEVELS {
        let (r, g, b) = p.dpi_color(i);
        pl[25 + 3 * i..28 + 3 * i].copy_from_slice(&[r, g, b]);
    }
    pl[49] = 1;
    put_sum16(&mut pl, 3, 50, 50);
    pl
}

/// 命令 0x08: 按键表, 18 个按钮 x 3 字节, 末尾 payload[57:59] 为 payload[3:57] 的 16 位和(大端)。
pub fn cmd_keymap(p: &HubProfile) -> Result<Vec<u8>> {
    let mut pl = vec![0u8; 0x3B];
    pl[0] = 0x08;
    pl[1] = 0x3B;
    pl[2] = 0x01;
    for i in 0..BUTTON_COUNT {
        let rec = p.button_record(i);
        let fid = rec[1];
        if fid == MACRO_VARIANT_ID {
            bail!("宏的\"左键按下播放\"变体(功能 id 0x18)没有抓包, 不支持");
        }
        let (b0, mut b1, mut b2) = function_wire(fid).unwrap_or((0, 0, 0));
        if fid == MACRO_FUNCTION_ID {
            if !p.macros.contains_key(&i) {
                bail!("按钮 {i} 绑定了宏但没有宏内容, 请用 set_button_macro / type: macro");
            }
            b1 = 0;
            b2 = slot_base(i).expect("绑了宏的按钮一定有槽位"); // 线上: [0x12, 0x00, 槽位]  (抓包: 12 00 1a / 12 00 15)
        }
        if fid == KEY_FUNCTION_ID {
            b1 = rec[7];
            b2 = rec[3];
        }
        pl[3 + 3 * i..6 + 3 * i].copy_from_slice(&[b0, b1, b2]);
    }
    put_sum16(&mut pl, 3, 57, 57);
    Ok(pl)
}

/// 完整应用一个方案要发的命令载荷(未分帧、无校验外壳), 顺序与 FUN_004612a0(which=0) 一致:
/// 0x0c, 0x04, 0x05, 0x06, 0x07, 0x08, 然后是宏消息(每条 111 字节, 按按钮序号)。
///
/// 蓝牙(BLE)下 HUB 直接把这些载荷写进 FEE3 特征值, 且不发 0x06(回报率), 所以 `include_polling=false`。
pub fn build_apply_payloads(p: &HubProfile, include_polling: bool) -> Result<Vec<Vec<u8>>> {
    let mut payloads = vec![cmd_begin(), cmd_dpi(p), cmd_light(p)];
    if include_polling {
        payloads.push(cmd_polling_rate(p)?);
    }
    payloads.push(cmd_misc07(p));
    payloads.push(cmd_keymap(p)?);
    for (&i, mc) in &p.macros {
        payloads.extend(encode_macro(&mc.events, slot_base(i).expect("宏只能绑在有槽位的按钮上"), mc.repeat as i64, mc.mode)?);
    }
    Ok(payloads)
}

/// 2.4G 接收器(HID)下的完整发送序列, 返回 64 字节帧列表: 普通命令一帧, 宏消息拆成两个分片帧。
///
/// 顺序与抓包里的包序一致(0c,04,05,06,07,08), 宏帧紧跟在按键表之后。
pub fn build_apply_sequence(p: &HubProfile) -> Result<Vec<Vec<u8>>> {
    let mut frames = Vec::new();
    for payload in build_apply_payloads(p, true)? {
        if payload[0] == 0x09 {
            frames.extend(message_frames(&payload)?);
        } else {
            frames.push(build_frame(&payload)?);
        }
    }
    Ok(frames)
}
