//! 把一个方案用中文描述出来(启动时打印当前生效的设置)。纯文本生成, 不碰设备。
//!
//! 描述的是**实际生效值**: 配置里没写的项会显示出厂默认, 这样看到的就是鼠标最终会被设成什么样。

use crate::profile::model::{
    light_mode_name, light_mode_params, HubProfile, BUTTON_LABELS_CN, DPI_LEVELS, KEY_FUNCTION_ID, MACRO_FUNCTION_ID,
    MACRO_VARIANT_ID, POLLING_RATES,
};
use crate::protocol::functions::{action_label_cn, action_name, MODIFIER_BITS};
use crate::protocol::keycodes::code_to_key;
use crate::protocol::macros::{start_mode_label_cn, start_mode_name};
use crate::protocol::status::{
    StatusReport, KIND_COMMAND_ACK, KIND_DPI_STAGE, KIND_LEVEL, KIND_LIGHT_MODE, KIND_POLLING_RATE, KIND_PROFILE_SLOT,
};

fn modifier_label(name: &str) -> &'static str {
    match name {
        "ctrl" => "Ctrl",
        "shift" => "Shift",
        "alt" => "Alt",
        "win" => "Win",
        _ => "?",
    }
}

fn on_off(v: i32) -> &'static str {
    if v != 0 {
        "开"
    } else {
        "关"
    }
}

/// Python 的 `f"{v:g}"`: 整数值不带小数点, 如 `0.5` / `3`。
fn num(v: f64) -> String {
    if v == v.trunc() {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// 按钮 i 当前绑定的中文描述, 如"按键 Ctrl+C"、"复制"、"宏 fps.rapid_fire（按住播放，松开停止，2 个事件）"。
pub fn describe_function(p: &HubProfile, i: usize) -> String {
    let rec = p.button_record(i);
    let fid = rec[1];
    if fid == KEY_FUNCTION_ID {
        let mut parts: Vec<String> = MODIFIER_BITS
            .iter()
            .filter(|(_, bit)| rec[7] & bit != 0)
            .map(|(n, _)| modifier_label(n).to_string())
            .collect();
        parts.push(code_to_key(rec[3]).to_uppercase().replace('_', " "));
        return format!("按键 {}", parts.join("+"));
    }
    if fid == MACRO_FUNCTION_ID {
        if let Some(mc) = p.macros.get(&i) {
            let mode_name = start_mode_name(mc.mode).unwrap_or("loop");
            let extra = if mc.mode == 0 { format!("，循环 {} 次", mc.repeat) } else { String::new() };
            let name = if mc.name.is_empty() { "(内联)" } else { mc.name.as_str() };
            return format!("宏 {name}（{}{extra}，{} 个事件）", start_mode_label_cn(mode_name), mc.events.len());
        }
    }
    if fid == MACRO_FUNCTION_ID || fid == MACRO_VARIANT_ID {
        return format!("宏（功能 {fid:#04x}，内容未定义）");
    }
    match action_name(fid) {
        Some(name) => action_label_cn(name).to_string(),
        None => format!("功能 {fid:#04x}（出厂默认）"),
    }
}

pub fn describe_light(p: &HubProfile) -> String {
    let mode = p.light_mode() as i64;
    let mut parts = vec![light_mode_name(mode)];
    let used = light_mode_params(mode);
    if used.contains(&"brightness") {
        parts.push(format!("亮度 {}", p.i32(0x2864)));
    }
    if used.contains(&"speed") {
        parts.push(format!("速度 {}", p.i32(0x2868)));
    }
    if used.contains(&"color") {
        let (r, g, b) = p.light_rgb();
        parts.push(format!("颜色 RGB({r}, {g}, {b})"));
    }
    if mode == 5 || mode == 6 {
        parts.push("颜色跟随当前 DPI 档".into());
    }
    parts.join(" · ")
}

fn dpi_at(p: &HubProfile, cur: i32) -> i64 {
    usize::try_from(cur).ok().filter(|c| *c < DPI_LEVELS).map(|c| p.dpi(c)).unwrap_or(0)
}

pub fn describe_dpi(p: &HubProfile) -> String {
    let cur = p.dpi_current();
    let stages: Vec<String> = (0..DPI_LEVELS)
        .filter(|&i| p.dpi_enabled(i))
        .map(|i| format!("{}档 {}{}", i + 1, p.dpi(i), if i as i32 == cur { "（当前）" } else { "" }))
        .collect();
    let off = DPI_LEVELS - stages.len();
    let tail = if off > 0 { format!(" · 另有 {off} 档未启用") } else { String::new() };
    format!("当前第 {} 档 = {} · {}{tail}", cur + 1, dpi_at(p, cur), stages.join(" / "))
}

/// 鼠标主动上报的非电量状态 -> 一句中文; 不认识的类型返回 None。
///
/// DPI 档位(类型 0x10, 数值 = 档位+1)来自 HUB 的 FUN_004600f0; 没有抓包/真机验证过。
/// 回报率(0x20)、灯光模式(0x70)、0x60、0x80 的数值含义没核实, 只报原始值; 0x50 按 HUB 的用法解释为命令应答。
pub fn describe_status_change(report: &StatusReport, p: Option<&HubProfile>) -> Option<String> {
    let value = report.value();
    match report.kind {
        KIND_DPI_STAGE => {
            let stage = value as i64 - 1;
            if !(0..DPI_LEVELS as i64).contains(&stage) {
                return Some(format!("收到 DPI 档位上报, 但数值 {value} 超出 1..{DPI_LEVELS}"));
            }
            let stage = stage as usize;
            Some(match p {
                None => format!("DPI 档位切换到第 {} 档", stage + 1),
                Some(p) => {
                    let note = if p.dpi_enabled(stage) { "" } else { "（该档在当前方案里未启用）" };
                    format!("DPI 档位切换到第 {} 档 = {} DPI{note}", stage + 1, p.dpi(stage))
                }
            })
        }
        KIND_POLLING_RATE => Some(format!("鼠标上报回报率变化（原始值 {value:#06x}）")),
        KIND_COMMAND_ACK => {
            let ok = if report.a == 0 { "成功".to_string() } else { format!("失败（结果 {:#04x}）", report.a) };
            Some(format!("鼠标应答命令 {:#04x}: {ok}", report.b)) // 真机确认: 93 50 00 <命令字>, 每条写入各回一个
        }
        KIND_LEVEL => Some(format!("鼠标上报 0x60 档位值 {value}（HUB 存 1..10 的值, 含义未确认）")),
        KIND_PROFILE_SLOT => Some(format!("鼠标上报 0x80 方案槽 {value}（HUB 收到 1..5 会存档, 含义未确认）")),
        KIND_LIGHT_MODE => Some(format!("鼠标上报灯光模式变化（原始值 {value:#06x}）")),
        _ => None,
    }
}

pub fn describe_performance(p: &HubProfile) -> String {
    [
        format!("休眠 {} 分钟", num(p.sleep_minutes())),
        format!("深度睡眠 {} 分钟", p.deep_sleep_minutes()),
        format!("按键响应 {} 毫秒", p.debounce_ms()),
        format!("波纹控制 {}", on_off(p.i32(0x2880))),
        format!("直线修正 {}", on_off(p.i32(0x2884))),
        format!("移动同步 {}", on_off(p.i32(0x2888))),
    ]
    .join(" · ")
}

/// 一个方案的全部生效设置, 每项一行(按钮一个一行); 与出厂默认不同的按钮标 ★。
pub fn describe_profile(p: &HubProfile) -> Vec<String> {
    let factory = HubProfile::factory_default();
    let hz = usize::try_from(p.polling_rate_index()).ok().and_then(|i| POLLING_RATES.get(i)).map(|r| r.0).unwrap_or(0);
    let mut lines = vec![
        format!("回报率  {hz} Hz"),
        format!("灯光    {}", describe_light(p)),
        format!("DPI     {}", describe_dpi(p)),
        format!("性能    {}", describe_performance(p)),
    ];
    for (n, &(i, label)) in BUTTON_LABELS_CN.iter().enumerate() {
        let changed = p.macros.contains_key(&i) || p.button_record(i) != factory.button_record(i);
        let head = if n == 0 { "按钮    " } else { "        " };
        lines.push(format!("{head}{label} → {}{}", describe_function(p, i), if changed { "  ★" } else { "" }));
    }
    lines
}
