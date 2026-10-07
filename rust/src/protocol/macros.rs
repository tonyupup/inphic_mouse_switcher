//! 宏(快捷指令)的编码: 事件列表 -> 111 字节宏消息 -> 分片帧。
//!
//! 来自 Ghidra 对 INPHIC.exe 的 FUN_00461c90 / FUN_004606a0(0x09 分片路径)的反编译,
//! 并用两份真实抓包(macro_key_A: 按下 A/延迟 50/抬起 A;  macro_key_B: 左键、右键、B 键各按下-延迟 50-抬起)
//! 逐字节验证。验证过的部分: 键盘按键事件、鼠标左/右键事件、消息头、16 位校验、分片、按键表里的宏绑定。
//! **没有抓包验证**: 鼠标移动事件(0xF6)、延迟 >= 128ms 的扩展编码、多条消息续接(0xFE)、中/前进/后退键码(取自 GUI 代码)。
//!
//! 消息格式(M, 共 0x6f = 111 字节):
//! ```text
//! M[0] = 0x09   M[1] = 0x6f   M[2] = 槽位
//! M[3] = 启动方式(见 START_MODES; 抓包 macro_key_A/B、macro_2/3/4 验证)   M[4..6] = 恒 0
//! M[7] = 重复次数(1..255, 只对"循环次数播放"有意义)   M[8] = 事件字节数/2
//! M[9..]  = 事件, 最多 98 字节; 放不下时在末尾写 [槽位+1, 0xFE] 并续到下一个槽位
//! M[109:111] = sum(M[3:109]) 的 16 位和, 大端
//! ```
//! 事件:
//! ```text
//! 按键: [标志|延迟, 键码]   标志 0x01=按下 0x81=抬起   (延迟 < 128ms, 取低 7 位; 标志与延迟按位或!)
//!       [标志|延迟%100, 键码, 延迟//100, 0x03]            (延迟 >= 128ms)
//! 移动: [延迟, 0xF6, x_hi, x_lo, y_hi, y_lo]               (延迟 >= 128ms 时末尾再加 [延迟//100, 0x03])
//! 键码: 键盘用 HID 键码; 鼠标按键用 0xF1 左 / 0xF2 右 / 0xF3 中 / 0xF4 后退 / 0xF5 前进
//! ```
//! 分片: 111 字节拆成 59 + 52 两片, 用 `frame::build_chunk_frame`(序号 0 和 1)。

use serde_yaml::Value;

use crate::bail;
use crate::error::{fmt_list, Result};
use crate::protocol::frame::build_chunk_frame;
use crate::protocol::keycodes::key_to_code;
use crate::yamlutil::{fmt_keys, repr, scalar_string, to_int, unknown_keys};

pub const MESSAGE_LEN: usize = 0x6F;
pub const EVENT_AREA: usize = 9; // M[9] 起是事件
pub const EVENT_LIMIT: usize = 0x62; // 单条消息事件字节上限(超过就续到下一槽位)
pub const CHECKSUM_AT: usize = 109;
pub const SLOTS_PER_BUTTON: usize = 5;

/// 每个能绑宏的按钮的起始槽位(构造函数里的常量表 0x4ee940; 抓包里 btn7=26, btn6=21 与之吻合)。
/// 其余按钮(4、5、8..17)没有槽位, 不能绑宏。
pub const SLOT_BASE: &[(usize, u8)] = &[(0, 1), (1, 6), (2, 11), (3, 16), (6, 21), (7, 26)];

pub fn slot_base(button: usize) -> Option<u8> {
    SLOT_BASE.iter().find(|(b, _)| *b == button).map(|&(_, s)| s)
}

pub const MOUSE_BUTTON_CODES: &[(&str, u8)] =
    &[("left", 0xF1), ("right", 0xF2), ("middle", 0xF3), ("backward", 0xF4), ("forward", 0xF5)];
const VERIFIED_BUTTONS: &[&str] = &["left", "right"];

/// 启动方式 = GUI 里 4 个单选项的序号(ms_3_option_repeat_mode_1..4), 处理函数把序号写进 rec+0xc 即 M[3]
pub const START_MODES: &[(&str, u8)] = &[
    ("loop", 0),             // 循环次数播放          (macro_key_A / B 抓包)
    ("stop_on_any_key", 1),  // 任意键按下停止播放    (macro_2 抓包)
    ("hold", 2),             // 按住播放, 松开停止    (macro_3 抓包)
    ("stop_on_this_key", 3), // 当前按键按下停止播放  (macro_4 抓包)
];

pub fn start_mode(name: &str) -> Option<u8> {
    START_MODES.iter().find(|(n, _)| *n == name).map(|&(_, v)| v)
}

pub fn start_mode_name(mode: u8) -> Option<&'static str> {
    START_MODES.iter().find(|(_, v)| *v == mode).map(|&(n, _)| n)
}

pub fn start_mode_names_sorted() -> Vec<String> {
    let mut v: Vec<String> = START_MODES.iter().map(|(n, _)| n.to_string()).collect();
    v.sort();
    v
}

/// HUB 语言包 select_macro_4/5/6/8_text
pub fn start_mode_label_cn(name: &str) -> &'static str {
    match name {
        "loop" => "循环次数播放",
        "stop_on_any_key" => "任意键按下停止播放",
        "hold" => "按住播放，松开停止",
        "stop_on_this_key" => "当前按键按下停止播放",
        _ => "?",
    }
}

pub const MOVE_OPCODE: u8 = 0xF6;
pub const EXT_DELAY_OPCODE: u8 = 0x03;
pub const CONTINUE_OPCODE: u8 = 0xFE;
pub const FLAG_DOWN: u8 = 0x01;
pub const FLAG_UP: u8 = 0x81;

pub const MAX_DELAY_MS: i64 = 25599; // 延迟//100 要放进一个字节

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Macro {
    /// 一个要上传的宏: 事件、重复次数(仅 loop 模式有意义)、启动方式。
    pub events: Vec<MacroEvent>,
    pub repeat: u8,
    pub mode: u8,
    /// 宏库里的 "分组.名字", 只用于日志
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Down,
    Up,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacroEvent {
    pub kind: EventKind,
    /// 键码(move 不用)
    pub code: u8,
    pub delay_ms: i64,
    pub x: i64,
    pub y: i64,
}

impl MacroEvent {
    pub fn key(kind: EventKind, code: u8, delay_ms: i64) -> Self {
        MacroEvent { kind, code, delay_ms, x: 0, y: 0 }
    }
}

fn check_delay(kind: EventKind, d: i64) -> Result<()> {
    if !(0..=MAX_DELAY_MS).contains(&d) {
        bail!("延迟 {d} 不合法: 需要 0..{MAX_DELAY_MS} 的整数毫秒");
    }
    if kind == EventKind::Move {
        return Ok(());
    }
    // 按键事件的第一个字节是 标志|延迟, 延迟的最低位会被标志位占掉, 所以奇数会被悄悄改成偶数
    let low = if d < 0x80 { d } else { d % 100 };
    if low % 2 != 0 {
        bail!(
            "按键延迟 {d} ms 不合法: 编码里延迟最低位与标志位重合, 小于 128 的值需要是偶数, \
             大于等于 128 的值其对 100 取余的部分需要是偶数"
        );
    }
    Ok(())
}

/// 返回 (事件字节, 计数单位)。计数单位 = 字节数/2, 累加后写进 M[8]。
pub fn encode_event(ev: &MacroEvent) -> Result<(Vec<u8>, usize)> {
    check_delay(ev.kind, ev.delay_ms)?;
    let d = ev.delay_ms;
    if ev.kind == EventKind::Move {
        for v in [ev.x, ev.y] {
            if !(-32768..=32767).contains(&v) {
                bail!("鼠标移动坐标 {v} 超出 16 位范围");
            }
        }
        let mut xy = (ev.x as u16).to_be_bytes().to_vec();
        xy.extend_from_slice(&(ev.y as u16).to_be_bytes());
        if d < 0x80 {
            let mut out = vec![if d == 0 { 1 } else { d as u8 }, MOVE_OPCODE];
            out.extend(xy);
            return Ok((out, 3));
        }
        let low = d % 100;
        let mut out = vec![if low == 0 { 1 } else { low as u8 }, MOVE_OPCODE];
        out.extend(xy);
        out.extend([(d / 100) as u8, EXT_DELAY_OPCODE]);
        return Ok((out, 4));
    }
    let flag = if ev.kind == EventKind::Down { FLAG_DOWN } else { FLAG_UP };
    if d < 0x80 {
        return Ok((vec![flag | if d == 0 { 1 } else { d as u8 }, ev.code], 1));
    }
    let low = d % 100;
    Ok((vec![flag | if low == 0 { 1 } else { low as u8 }, ev.code, (d / 100) as u8, EXT_DELAY_OPCODE], 2))
}

/// 事件列表 -> 一条或多条 111 字节宏消息(最多 SLOTS_PER_BUTTON 条)。
pub fn encode_macro(events: &[MacroEvent], first_slot: u8, repeat: i64, mode: u8) -> Result<Vec<Vec<u8>>> {
    if events.is_empty() {
        bail!("宏至少需要一个事件");
    }
    if !(1..=255).contains(&repeat) {
        bail!("重复次数 {repeat} 超出 1..255");
    }
    if start_mode_name(mode).is_none() {
        bail!("启动方式 {mode} 不合法, 可选 {:?}", START_MODES);
    }
    let mut messages: Vec<Vec<u8>> = Vec::new();
    let (mut slot, mut start) = (first_slot, 0usize);
    loop {
        if messages.len() >= SLOTS_PER_BUTTON {
            bail!("宏太长: 一个按钮最多 {SLOTS_PER_BUTTON} 条消息(每条约 {EVENT_LIMIT} 字节事件)");
        }
        let mut m = vec![0u8; MESSAGE_LEN + 16]; // 多留一点, 溢出的事件字节先写进来再清掉
        m[0] = 0x09;
        m[1] = MESSAGE_LEN as u8;
        m[2] = slot;
        if start == 0 {
            // 属性只在宏的第一条消息里
            m[3] = mode;
            m[7] = repeat as u8;
        }
        let (mut pos, mut count) = (0usize, 0usize);
        let (mut i, mut more, mut next_start) = (start, false, 0usize);
        while i < events.len() {
            let (data, units) = encode_event(&events[i])?;
            let new_pos = pos + data.len();
            m[EVENT_AREA + pos..EVENT_AREA + new_pos].copy_from_slice(&data);
            count += units;
            if new_pos > EVENT_LIMIT {
                // 放不下: 清掉这个事件, 在它的位置写续接标记 [slot+1, 0xFE](计数里占 1 个单位)
                m[EVENT_AREA + pos..EVENT_AREA + new_pos].fill(0);
                count -= units - 1;
                m[EVENT_AREA + pos] = slot + 1;
                m[EVENT_AREA + pos + 1] = CONTINUE_OPCODE;
                more = true;
                next_start = i;
                break;
            }
            pos = new_pos;
            i += 1;
        }
        m[8] = count as u8;
        let s = crate::protocol::frame::sum16(&m[3..CHECKSUM_AT]);
        m[CHECKSUM_AT..CHECKSUM_AT + 2].copy_from_slice(&s.to_be_bytes());
        m.truncate(MESSAGE_LEN);
        messages.push(m);
        if !more {
            return Ok(messages);
        }
        slot += 1;
        start = next_start;
    }
}

/// 111 字节宏消息 -> 2 个 64 字节帧(59 + 52 字节, 分片序号 0 和 1)。
pub fn message_frames(message: &[u8]) -> Result<Vec<Vec<u8>>> {
    if message.len() != MESSAGE_LEN {
        bail!("宏消息需要 {MESSAGE_LEN} 字节, 实际 {}", message.len());
    }
    Ok(vec![build_chunk_frame(&message[..59], 0)?, build_chunk_frame(&message[59..], 1)?])
}

pub fn macro_frames(events: &[MacroEvent], first_slot: u8, repeat: i64, mode: u8) -> Result<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    for m in encode_macro(events, first_slot, repeat, mode)? {
        out.extend(message_frames(&m)?);
    }
    Ok(out)
}

// ---------------- 配置里的 steps -> 事件 ----------------

/// `'a'` / `'left_shift'` -> 键盘 HID 键码; `'mouse_left'` 等 -> 0xF1.. 鼠标按键码。
fn resolve_target(spec: &str) -> Result<u8> {
    if let Some(name) = spec.strip_prefix("mouse_") {
        let Some(&(_, code)) = MOUSE_BUTTON_CODES.iter().find(|(n, _)| *n == name) else {
            let names: Vec<String> = MOUSE_BUTTON_CODES.iter().map(|(n, _)| format!("mouse_{n}")).collect();
            bail!("鼠标按键 {spec:?} 不认识, 可选: {}", names.join(", "));
        };
        if !VERIFIED_BUTTONS.contains(&name) {
            log::warn!("宏里的 {spec} 键码 {code:#x} 取自 GUI 代码, 没有抓包验证");
        }
        return Ok(code);
    }
    key_to_code(spec)
}

/// `steps` 里每一项恰好有一个动作键:  down / up / tap / move,  可选 delay_ms。
///
/// ```yaml
/// - { tap: "a", delay_ms: 50 }          # 按下再抬起, 两个事件各带这个延迟(和 HUB 的"默认延迟"一致)
/// - { down: "left_shift" }  /  { up: "left_shift" }
/// - { tap: "mouse_left" }                # 鼠标按键
/// - { move: [10, -5], delay_ms: 20 }     # 相对移动(没有抓包验证)
/// ```
pub fn parse_steps_value(steps: Option<&Value>, default_delay_ms: i64) -> Result<Vec<MacroEvent>> {
    match steps {
        Some(Value::Sequence(s)) if !s.is_empty() => parse_steps(s, default_delay_ms),
        _ => bail!("macro.steps 需要是非空列表"),
    }
}

pub fn parse_steps(steps: &[Value], default_delay_ms: i64) -> Result<Vec<MacroEvent>> {
    if steps.is_empty() {
        bail!("macro.steps 需要是非空列表");
    }
    let mut events: Vec<MacroEvent> = Vec::new();
    let mut bad_delay: Vec<Option<String>> = Vec::new(); // 与 events 平行: 不是整数的延迟, 稍后统一报错
    for (n, step) in steps.iter().enumerate() {
        let n = n + 1;
        let Value::Mapping(step) = step else {
            bail!("steps[{n}] 需要是字典");
        };
        let extra = unknown_keys(step, &["down", "up", "tap", "move", "delay_ms"]);
        if !extra.is_empty() {
            bail!("steps[{n}] 里有不认识的键: {}", fmt_keys(&extra));
        }
        let actions: Vec<&str> = ["down", "up", "tap", "move"].into_iter().filter(|k| step.contains_key(*k)).collect();
        if actions.len() != 1 {
            bail!("steps[{n}] 需要恰好一个动作 (down / up / tap / move), 实际: {}", fmt_list(&actions));
        }
        let action = actions[0];
        let (delay, bad) = match step.get("delay_ms") {
            None => (default_delay_ms, None),
            Some(v) => match crate::yamlutil::as_int(v) {
                Some(d) => (d, None),
                None => (-1, Some(repr(v))),
            },
        };
        let pushed = |events: &mut Vec<MacroEvent>, bad_delay: &mut Vec<Option<String>>, ev: MacroEvent| {
            events.push(ev);
            bad_delay.push(bad.clone());
        };
        if action == "move" {
            let xy = match &step["move"] {
                Value::Sequence(s) if s.len() == 2 => (to_int(&s[0]), to_int(&s[1])),
                _ => bail!("steps[{n}].move 需要 [x, y]"),
            };
            let (Some(x), Some(y)) = xy else {
                bail!("steps[{n}].move 需要 [x, y]");
            };
            pushed(&mut events, &mut bad_delay, MacroEvent { kind: EventKind::Move, code: 0, delay_ms: delay, x, y });
            continue;
        }
        let Some(target) = scalar_string(&step[action]) else {
            bail!("steps[{n}].{action} 需要是键名字符串, 实际 {}", repr(&step[action]));
        };
        let code = resolve_target(&target)?;
        if action == "down" || action == "tap" {
            pushed(&mut events, &mut bad_delay, MacroEvent::key(EventKind::Down, code, delay));
        }
        if action == "up" || action == "tap" {
            pushed(&mut events, &mut bad_delay, MacroEvent::key(EventKind::Up, code, delay));
        }
    }
    for (n, ev) in events.iter().enumerate() {
        // 配置错误在解析时就报, 不等到编码
        if let Some(r) = &bad_delay[n] {
            bail!("steps 展开后第 {} 个事件: 延迟 {r} 不合法: 需要 0..{MAX_DELAY_MS} 的整数毫秒", n + 1);
        }
        if let Err(e) = check_delay(ev.kind, ev.delay_ms) {
            bail!("steps 展开后第 {} 个事件: {e}", n + 1);
        }
    }
    Ok(events)
}
