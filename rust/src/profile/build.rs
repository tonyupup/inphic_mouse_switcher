//! 从语义配置(yaml 里的一个方案)构建方案: 解析并校验 polling_rate / light / dpi / performance / buttons。
//!
//! 不认识的键、越界的值一律报错, 不悄悄忽略; 合法但不起作用的设置(如当前灯光模式用不到的参数)给出警告。

use serde_yaml::{Mapping, Value};

use crate::bail;
use crate::error::{fmt_list, Result};
use crate::profile::commands::{build_apply_payloads, build_apply_sequence};
use crate::profile::model::{
    light_mode_name, light_mode_params, HubProfile, BUTTON_COUNT, BUTTON_NAMES, DPI_LEVELS, DPI_MAX, DPI_MIN, DPI_STEP,
    LIGHT_MODES, LIGHT_MODE_NAMES, POLLING_RATES,
};
use crate::protocol::functions::{
    lookup, sorted_names, MEDIA_ACTIONS, MODIFIER_BITS, MOUSE_ACTIONS, SHORTCUT_ACTIONS,
};
use crate::protocol::keycodes::key_to_code;
use crate::protocol::macros::{parse_steps_value, start_mode, start_mode_names_sorted};
use crate::yamlutil::{as_int, fmt_keys, repr, scalar_string, to_f64, to_int, unknown_keys};

/// 宏库里展开好的一个宏: 方案构建只需要这几项。宏库本身在 config 层, 这里用 trait 避免反向依赖。
pub struct MacroSource {
    pub name: String,
    pub steps: Vec<Value>,
    pub start: String,
    pub repeat: i64,
}

pub trait MacroLibraryLookup {
    fn resolve_macro(&self, reference: &str) -> Result<MacroSource>;
}

/// 取字典里的可选子字典: 没写/null 当空; 写了别的类型报错。
fn sub_mapping<'a>(cfg: &'a Mapping, key: &str, empty: &'a Mapping) -> Result<&'a Mapping> {
    match cfg.get(key) {
        None | Some(Value::Null) => Ok(empty),
        Some(Value::Mapping(m)) => Ok(m),
        Some(other) => bail!("{key} 需要是字典, 实际 {}", repr(other)),
    }
}

fn int_of(v: &Value, what: &str) -> Result<i64> {
    match to_int(v) {
        Some(n) => Ok(n),
        None => bail!("{what} 需要是整数, 实际 {}", repr(v)),
    }
}

fn rgb_of(v: &Value, what: &str) -> Result<[u8; 3]> {
    let Value::Sequence(s) = v else {
        bail!("{what} 需要是 [r, g, b], 实际 {}", repr(v));
    };
    if s.len() != 3 {
        bail!("{what} 需要是 [r, g, b], 实际 {}", repr(v));
    }
    let mut out = [0u8; 3];
    for (k, c) in s.iter().enumerate() {
        let n = int_of(c, what)?;
        if !(0..=255).contains(&n) {
            bail!("{what} 超出 0..255: {}", repr(v));
        }
        out[k] = n as u8;
    }
    Ok(out)
}

/// 配置里一个按钮的 action 字典 -> 写入记录。type: key / mouse / shortcut / media / function / macro。
pub fn set_button_action(
    profile: &mut HubProfile,
    i: usize,
    action: &Value,
    label: &str,
    library: Option<&dyn MacroLibraryLookup>,
) -> Result<()> {
    let label = if label.is_empty() { i.to_string() } else { label.to_string() };
    let Value::Mapping(action) = action else {
        bail!("按钮 {label} 的绑定需要是字典, 实际 {}", repr(action));
    };
    let allowed = ["type", "key", "modifiers", "action", "id", "steps", "repeat", "default_delay_ms", "start", "ref"];
    let extra = unknown_keys(action, &allowed);
    if !extra.is_empty() {
        bail!("按钮 {label} 里有不认识的键: {}", fmt_keys(&extra));
    }
    let kind = action.get("type");
    let kind_str = match kind {
        Some(Value::String(s)) => s.as_str(),
        _ => "",
    };
    match kind_str {
        "key" => {
            let mods: Vec<Value> = match action.get("modifiers") {
                None | Some(Value::Null) => vec![],
                Some(Value::Sequence(s)) => s.clone(),
                Some(other) => bail!("按钮 {label} 的 modifiers 需要是列表, 实际 {}", repr(other)),
            };
            let mut mask = 0u8;
            let mut bad = Vec::new();
            for m in &mods {
                match m {
                    Value::String(s) if MODIFIER_BITS.iter().any(|(n, _)| n == s) => {
                        mask |= MODIFIER_BITS.iter().find(|(n, _)| n == s).unwrap().1;
                    }
                    other => bad.push(repr(other)),
                }
            }
            if !bad.is_empty() {
                let mut names: Vec<&str> = MODIFIER_BITS.iter().map(|(n, _)| *n).collect();
                names.sort();
                bail!("按钮 {label} 的修饰键 [{}] 不认识, 可选: {}", bad.join(", "), fmt_list(&names));
            }
            let Some(key) = action.get("key") else {
                bail!("按钮 {label} 的 key 绑定缺少 key");
            };
            let Some(key) = scalar_string(key) else {
                bail!("按钮 {label} 的 key 需要是键名字符串, 实际 {}", repr(key));
            };
            profile.set_button_key(i, key_to_code(&key)?, mask)?;
        }
        "mouse" | "shortcut" | "media" => {
            let table = match kind_str {
                "mouse" => MOUSE_ACTIONS,
                "shortcut" => SHORTCUT_ACTIONS,
                _ => MEDIA_ACTIONS,
            };
            let name = action.get("action");
            let id = match name {
                Some(Value::String(n)) => lookup(table, n),
                _ => None,
            };
            let Some(id) = id else {
                let shown = name.map(repr).unwrap_or_else(|| "None".into());
                bail!("按钮 {label} 的 {kind_str} 动作 {shown} 不认识, 可选: {}", fmt_list(&sorted_names(table)));
            };
            profile.set_button_function(i, id as i64)?;
        }
        "function" => {
            let Some(id) = action.get("id") else {
                bail!("按钮 {label} 的 function 绑定缺少 id");
            };
            profile.set_button_function(i, int_of(id, &format!("按钮 {label} 的 id"))?)?;
        }
        "macro" => {
            let reference = action.get("ref");
            if reference.is_some() && action.contains_key("steps") {
                bail!("按钮 {label}: ref 和 steps 只能写一个");
            }
            let (steps, mut start, mut repeat, name): (Value, Value, i64, String);
            if let Some(reference) = reference {
                let Some(library) = library else {
                    bail!("按钮 {label}: 用了 ref {} 但没有加载宏库, 请在配置里写 macro_files", repr(reference));
                };
                let Value::String(reference) = reference else {
                    bail!("按钮 {label}: ref 需要是字符串, 实际 {}", repr(reference));
                };
                let mdef = library.resolve_macro(reference)?;
                steps = Value::Sequence(mdef.steps);
                start = Value::String(mdef.start);
                repeat = mdef.repeat;
                name = mdef.name;
            } else {
                steps = action.get("steps").cloned().unwrap_or(Value::Null);
                start = Value::String("loop".into());
                repeat = 1;
                name = String::new();
            }
            if let Some(s) = action.get("start") {
                start = s.clone(); // 绑定处可以覆盖宏自己的设置
            }
            if let Some(r) = action.get("repeat") {
                repeat = int_of(r, &format!("按钮 {label} 的 repeat"))?;
            }
            let start_name = match &start {
                Value::String(s) if start_mode(s).is_some() => s.clone(),
                other => bail!(
                    "按钮 {label} 的宏启动方式 {} 不认识, 可选: {}",
                    repr(other),
                    fmt_list(&start_mode_names_sorted())
                ),
            };
            if start_name != "loop" && repeat != 1 {
                log::warn!("按钮 {label}: repeat 只对 start: loop 有意义, 其他启动方式会被忽略");
            }
            let default_delay = match action.get("default_delay_ms") {
                None => 50,
                Some(v) => int_of(v, &format!("按钮 {label} 的 default_delay_ms"))?,
            };
            let events = parse_steps_value(Some(&steps), default_delay)?;
            profile.set_button_macro(i, events, repeat, start_mode(&start_name).unwrap(), &name)?;
        }
        _ => {
            let shown = kind.map(repr).unwrap_or_else(|| "None".into());
            bail!("按钮 {label} 的 type {shown} 不支持, 可选: key / mouse / shortcut / media / function / macro");
        }
    }
    Ok(())
}

/// 把 config.yaml 里一个方案的语义配置套到本方案上; 不认识的键直接报错, 不悄悄忽略。
pub fn apply_config(profile: &mut HubProfile, cfg: &Mapping, library: Option<&dyn MacroLibraryLookup>) -> Result<()> {
    let known = ["polling_rate", "light", "dpi", "buttons", "performance"];
    let unknown = unknown_keys(cfg, &known);
    if !unknown.is_empty() {
        bail!("方案里有不认识的配置项: {}, 可选: {}", fmt_keys(&unknown), fmt_list(&{
            let mut k = known.to_vec();
            k.sort();
            k
        }));
    }
    let empty = Mapping::new();

    if let Some(v) = cfg.get("polling_rate") {
        let hz = int_of(v, "polling_rate")?;
        let Some(idx) = POLLING_RATES.iter().position(|&(h, _)| h as i64 == hz) else {
            let mut hzs: Vec<u32> = POLLING_RATES.iter().map(|&(h, _)| h).collect();
            hzs.sort();
            bail!("回报率 {hz} 不支持, 可选: {hzs:?}");
        };
        profile.set_i32(0x2878, idx as i64);
    }

    let light = sub_mapping(cfg, "light", &empty)?;
    let extra = unknown_keys(light, &["mode", "rgb", "brightness", "speed"]);
    if !extra.is_empty() {
        bail!("light 里有不认识的键: {}, 可选: mode, rgb, brightness, speed", fmt_keys(&extra));
    }
    if let Some(m) = light.get("mode") {
        let mode = match m {
            Value::String(s) => match LIGHT_MODE_NAMES.iter().find(|(n, _)| n == s) {
                Some(&(_, v)) => v,
                None => {
                    let mut names: Vec<&str> = LIGHT_MODE_NAMES.iter().map(|(n, _)| *n).collect();
                    names.sort();
                    bail!("灯光模式 {} 不认识, 可选: {}", repr(m), fmt_list(&names));
                }
            },
            other => match as_int(other) {
                Some(n) => n,
                None => bail!("灯光模式 {} 不认识, 需要名字或 0..6 的序号", repr(other)),
            },
        };
        if !(0..LIGHT_MODES.len() as i64).contains(&mode) {
            bail!("灯光模式序号 {mode} 不在 0..6");
        }
        profile.set_i32(0x288C, mode);
    }
    if let Some(v) = light.get("rgb") {
        let rgb = rgb_of(v, "light.rgb")?;
        profile.raw[0x28A0..0x28A3].copy_from_slice(&rgb);
    }
    for (key, off) in [("brightness", 0x2864usize), ("speed", 0x2868usize)] {
        if let Some(v) = light.get(key) {
            let n = int_of(v, &format!("light.{key}"))?;
            if !(1..=8).contains(&n) {
                bail!("light.{key} = {n} 超出 GUI 滑条范围 1..8");
            }
            profile.set_i32(off, n);
        }
    }
    // 配了当前模式用不到的参数: 合法但不起作用, 警告而不是悄悄忽略
    let used = light_mode_params(profile.light_mode() as i64);
    for (key, param) in [("rgb", "color"), ("brightness", "brightness"), ("speed", "speed")] {
        if light.contains_key(key) && !used.contains(&param) {
            log::warn!("灯光模式 '{}' 不使用 {key}, 该设置不会起作用", light_mode_name(profile.light_mode() as i64));
        }
    }

    apply_dpi(profile, cfg)?;
    apply_performance(profile, cfg, &empty)?;

    if let Some(b) = cfg.get("buttons") {
        let buttons = match b {
            Value::Null => &empty,
            Value::Mapping(m) => m,
            other => bail!("buttons 需要是字典, 实际 {}", repr(other)),
        };
        for (name, action) in buttons {
            let label = scalar_string(name).unwrap_or_else(|| repr(name));
            let idx = match name {
                Value::String(s) => BUTTON_NAMES.iter().find(|(n, _)| n == s).map(|&(_, i)| i as i64),
                other => as_int(other),
            };
            let Some(idx) = idx.filter(|i| (0..BUTTON_COUNT as i64).contains(i)) else {
                let mut names: Vec<&str> = BUTTON_NAMES.iter().map(|(n, _)| *n).collect();
                names.sort();
                bail!(
                    "按钮 {} 不认识, 可用名字 {} 或序号 0..{}",
                    repr(name),
                    fmt_list(&names),
                    BUTTON_COUNT - 1
                );
            };
            set_button_action(profile, idx as usize, action, &label, library)?;
        }
    }
    Ok(())
}

fn apply_dpi(profile: &mut HubProfile, cfg: &Mapping) -> Result<()> {
    let dpi_val = cfg.get("dpi").cloned().unwrap_or(Value::Null);
    let shorthand;
    let dpi: Mapping = match &dpi_val {
        Value::Null => Mapping::new(),
        Value::Mapping(m) => m.clone(),
        Value::Number(_) if as_int(&dpi_val).is_some() => {
            // 简写: dpi: 1600 = 只用这一个 DPI
            let mut m = Mapping::new();
            m.insert("levels".into(), Value::Sequence(vec![dpi_val.clone()]));
            m.insert("current".into(), dpi_val.clone());
            shorthand = m;
            shorthand
        }
        other => bail!("dpi 需要是数字(如 1600)或字典, 实际 {}", repr(other)),
    };
    let extra = unknown_keys(&dpi, &["levels", "enabled", "current", "colors"]);
    if !extra.is_empty() {
        bail!("dpi 里有不认识的键: {}, 可选: levels, enabled, current, colors", fmt_keys(&extra));
    }
    let seq_of = |key: &str| -> Option<Vec<Value>> {
        match dpi.get(key) {
            Some(Value::Sequence(s)) => Some(s.clone()),
            Some(_) => Some(vec![]), // 类型不对: 当成长度 0, 由后面的范围检查报错
            None => None,
        }
    };
    let levels = seq_of("levels");
    let mut enabled = seq_of("enabled");
    if let Some(levels) = &levels {
        if !(1..=DPI_LEVELS).contains(&levels.len()) {
            bail!("dpi.levels 需要 1..{DPI_LEVELS} 个值(没写到的档位不启用)");
        }
        for (i, v) in levels.iter().enumerate() {
            let ok = as_int(v).filter(|n| (DPI_MIN..=DPI_MAX).contains(n) && n % DPI_STEP == 0);
            let Some(n) = ok else {
                bail!("dpi 档位 {} 不合法: 需要 {DPI_MIN}..{DPI_MAX} 之间 {DPI_STEP} 的整数倍", repr(v));
            };
            profile.set_i32(0x2804 + 4 * i, n / DPI_STEP);
        }
        if enabled.is_none() && levels.len() < DPI_LEVELS {
            enabled = Some(vec![Value::Bool(true); levels.len()]);
        }
    }
    if let Some(enabled) = &enabled {
        if !(1..=DPI_LEVELS).contains(&enabled.len()) {
            bail!("dpi.enabled 需要 1..{DPI_LEVELS} 个布尔值(没写到的档位不启用)");
        }
        let mut flags = vec![false; DPI_LEVELS];
        for (i, v) in enabled.iter().enumerate() {
            let Value::Bool(b) = v else {
                bail!("dpi.enabled 里的 {} 需要是 true / false", repr(v));
            };
            flags[i] = *b;
        }
        for (i, on) in flags.iter().enumerate() {
            profile.set_i32(0x2844 + 4 * i, *on as i64);
        }
    }
    if let Some(cur) = dpi.get("current") {
        let mut cur = int_of(cur, "dpi.current")?;
        if cur > DPI_LEVELS as i64 {
            // 大于 8 的当 DPI 值: 找到启用的、值相同的那一档
            let hit = (0..DPI_LEVELS).find(|&i| profile.dpi_enabled(i) && profile.dpi(i) == cur);
            let Some(hit) = hit else {
                bail!("dpi.current = {cur}: 启用的档位里没有这个 DPI 值(档位号请写 1..{DPI_LEVELS})");
            };
            cur = hit as i64 + 1;
        }
        if !(1..=DPI_LEVELS as i64).contains(&cur) {
            bail!("dpi.current = {cur} 超出 1..{DPI_LEVELS}");
        }
        profile.set_i32(0x2874, cur - 1);
    } else if matches!(&levels, Some(l) if l.len() < DPI_LEVELS) {
        profile.set_i32(0x2874, 0); // 只写了几档又没指定当前档: 用第 1 档
    }
    if let Some(colors) = dpi.get("colors") {
        let list = match colors {
            Value::Sequence(s) if (1..=DPI_LEVELS).contains(&s.len()) => s,
            _ => bail!("dpi.colors 需要 1..{DPI_LEVELS} 个 [r, g, b]"),
        };
        for (i, rgb) in list.iter().enumerate() {
            let c = rgb_of(rgb, &format!("dpi.colors[{i}]"))?;
            profile.raw[0x28A4 + 4 * i..0x28A7 + 4 * i].copy_from_slice(&c);
        }
    }
    let cur = profile.dpi_current();
    let cur_enabled = usize::try_from(cur).ok().filter(|c| *c < DPI_LEVELS).is_some_and(|c| profile.dpi_enabled(c));
    if !dpi.is_empty() && !cur_enabled {
        // GUI 不允许取消勾选当前档位, 这里同样拒绝, 否则鼠标会停在一个被禁用的档位上
        bail!("当前档位 {} 被禁用: 请在 dpi.enabled 里启用它, 或改 dpi.current", cur + 1);
    }
    Ok(())
}

fn apply_performance(profile: &mut HubProfile, cfg: &Mapping, empty: &Mapping) -> Result<()> {
    let perf = sub_mapping(cfg, "performance", empty)?;
    let keys = ["sleep_minutes", "deep_sleep_minutes", "debounce_ms", "ripple_control", "angle_snap", "motion_sync"];
    let extra = unknown_keys(perf, &keys);
    if !extra.is_empty() {
        bail!(
            "performance 里有不认识的键: {}, 可选: sleep_minutes, deep_sleep_minutes, debounce_ms, ripple_control, angle_snap, motion_sync",
            fmt_keys(&extra)
        );
    }
    if let Some(v) = perf.get("sleep_minutes") {
        // 存档值 = 分钟 / 0.5 (GUI 滑条 1..60 -> 0.5..30 分钟)
        let minutes = to_f64(v).unwrap_or(f64::NAN);
        let x = minutes * 2.0;
        if x.is_nan() || x != x.trunc() || !(1.0..=60.0).contains(&x) {
            bail!("performance.sleep_minutes = {} 不合法: 0.5..30 之间 0.5 的整数倍", repr(v));
        }
        profile.set_i32(0x2870, x as i64);
    }
    if let Some(v) = perf.get("deep_sleep_minutes") {
        let Some(n) = as_int(v).filter(|n| (1..=60).contains(n)) else {
            bail!("performance.deep_sleep_minutes = {} 不合法: 1..60 的整数", repr(v));
        };
        profile.set_i32(0x286C, n);
    }
    if let Some(v) = perf.get("debounce_ms") {
        // 存档值 = 毫秒 / 2 (GUI 滑条 2..25 -> 4..50 ms)
        let Some(n) = as_int(v).filter(|n| n % 2 == 0 && (4..=50).contains(n)) else {
            bail!("performance.debounce_ms = {} 不合法: 4..50 之间的偶数", repr(v));
        };
        profile.set_i32(0x2898, n / 2);
    }
    for (key, off) in [("ripple_control", 0x2880usize), ("angle_snap", 0x2884), ("motion_sync", 0x2888)] {
        if let Some(v) = perf.get(key) {
            let Value::Bool(b) = v else {
                bail!("performance.{key} 需要 true / false");
            };
            profile.set_i32(off, *b as i64);
        }
    }
    Ok(())
}

/// 配置里的一个具名方案: 给运行时用的最小接口(build_packets / build_payloads / current_buttons)。
#[derive(Debug, Clone)]
pub struct ConfiguredProfile {
    pub name: String,
    pub profile: HubProfile,
}

impl ConfiguredProfile {
    /// 2.4G(HID)下发送的 64 字节帧。
    pub fn build_packets(&self) -> Result<Vec<Vec<u8>>> {
        build_apply_sequence(&self.profile)
    }

    /// 蓝牙(BLE)下写进 FEE3 的原始载荷; 蓝牙不支持改回报率, 调用方传 `include_polling=false`。
    pub fn build_payloads(&self, include_polling: bool) -> Result<Vec<Vec<u8>>> {
        build_apply_payloads(&self.profile, include_polling)
    }

    pub fn current_buttons(&self) -> Vec<(String, String)> {
        BUTTON_NAMES.iter().map(|&(name, idx)| (name.to_string(), self.profile.describe_button(idx))).collect()
    }
}

pub fn from_config(name: &str, cfg: &Value, library: Option<&dyn MacroLibraryLookup>) -> Result<ConfiguredProfile> {
    let empty = Mapping::new();
    let cfg = match cfg {
        Value::Null => &empty,
        Value::Mapping(m) => m,
        other => bail!("方案「{name}」需要是字典, 实际 {}", repr(other)),
    };
    let mut p = HubProfile::factory_default();
    apply_config(&mut p, cfg, library)?;
    Ok(ConfiguredProfile { name: name.to_string(), profile: p })
}
