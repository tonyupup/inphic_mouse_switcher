//! 方案模型: HUB 配置存档(Data/ms_N/pro.data)里一个方案的内存结构, 以及字段读写、按钮绑定。
//!
//! 不含命令构造(见 commands.rs)和 yaml 配置解析(见 build.rs)。
//!
//! pro.data 布局: 4 字节头 + 7 个方案, 每个方案 PROFILE_SIZE 字节(4 + 7*0x1f624 == 899840)。
//! 本机的"配置1"(默认)是第 5 个, "CF"是第 6 个; 其余槽位全 0。
//!
//! 下面所有 `+0xNNNN` 都是方案结构体内的偏移, 来自 Ghidra 对 INPHIC.exe(MS_3 页面)的反编译;
//! 字段含义里"推断"二字表示是按偏移和取值猜的名字, 没有逐个核对过 GUI 控件。

use std::collections::BTreeMap;
use std::path::Path;

use crate::bail;
use crate::error::Result;
use crate::profile::defaults::DEFAULT_FIELDS;
use crate::protocol::functions::{
    action_name, category_of, function_wire, CATEGORY_KEYBOARD, CATEGORY_MACRO, MODIFIER_BITS,
};
use crate::protocol::hex_decode;
use crate::protocol::keycodes::code_to_key;
use crate::protocol::macros::{macro_frames, slot_base, start_mode_name, Macro, MacroEvent, SLOT_BASE};

pub const HEADER_SIZE: usize = 4;
pub const PROFILE_SIZE: usize = 0x1F624;
pub const PROFILE_COUNT: usize = 7;

pub const DPI_LEVELS: usize = 8;
/// GUI 里显示的 DPI = 存档值 * 50, 输入限制 50..26000 (FUN_004561c0: 值/0x32 存回, 滑条 1..520)
pub const DPI_STEP: i64 = 50;
pub const DPI_MIN: i64 = 50;
pub const DPI_MAX: i64 = 26000;
pub const BUTTON_COUNT: usize = 18;
pub const BUTTON_STRIDE: usize = 0x1338;
pub const BUTTON_BASE: usize = 0x28E4;

/// 灯光模式(GUI 下拉序号, 来自 res/lan/CN.xml 的 ms_lightmodeN)
pub const LIGHT_MODES: &[&str] = &["关闭", "常亮", "呼吸", "霓虹", "循环呼吸", "常亮DPI", "呼吸DPI"];

/// 每种灯光模式在 GUI 里会显示哪些参数控件(FUN_004333b0: 先全部隐藏, 再按模式显示)。
/// brightness = 亮度滑条(1..8, 方案 +0x2864), speed = 速度滑条(1..8, +0x2868), color = RGB(+0x28a0)。
/// 与 cmd_light 的组包一致: 只有模式 1、5 才会把亮度写进灯光包, 其余模式亮度位固定为 8。
/// 模式 8..11 本机界面里隐藏, 这里不收录。
pub fn light_mode_params(mode: i64) -> &'static [&'static str] {
    match mode {
        0 => &[],                          // 关闭
        1 => &["brightness", "color"],     // 常亮
        2 => &["speed", "color"],          // 呼吸
        3 => &["speed"],                   // 霓虹
        4 => &["speed"],                   // 循环呼吸
        5 => &["brightness"],              // 常亮DPI (颜色跟随当前 DPI 档位的颜色)
        6 => &["speed"],                   // 呼吸DPI (颜色跟随当前 DPI 档位的颜色)
        _ => &[],
    }
}

pub fn light_mode_name(mode: i64) -> String {
    usize::try_from(mode).ok().and_then(|m| LIGHT_MODES.get(m)).map(|s| s.to_string()).unwrap_or_else(|| mode.to_string())
}

/// 回报率: GUI 序号 -> (Hz, 线上码)。线上码 C 以 C, ~C 两个字节发出
pub const POLLING_RATES: &[(u32, u8)] =
    &[(125, 0x20), (250, 0x10), (500, 0x08), (1000, 0x04), (2000, 0x02), (4000, 0x01), (8000, 0x40)];

/// 配置文件里的按钮名 -> 按钮序号。只有这几个的物理对应关系是实测确认过的; 其余用数字序号 0..17
pub const BUTTON_NAMES: &[(&str, usize)] = &[("middle_click", 2), ("dpi_key", 3), ("side_back", 6), ("side_front", 7)];

/// 这只鼠标实际有的 6 个可配置按钮(GUI 按键页只显示这 6 个热点)及其中文名;
/// 0/1 从出厂默认功能推断, 3 是鼠标顶部的 DPI 键(真机确认: 改成输入 a 后按它就输出 a)
pub const BUTTON_LABELS_CN: &[(usize, &str)] =
    &[(0, "左键"), (1, "右键"), (2, "中键"), (3, "DPI 键"), (6, "后退侧键"), (7, "前进侧键")];

/// 灯光模式名(配置里可写名字或序号)
pub const LIGHT_MODE_NAMES: &[(&str, i64)] = &[
    ("off", 0), ("static", 1), ("breathing", 2), ("neon", 3), ("spectrum_cycle", 4),
    ("dpi_static", 5), ("dpi_breathing", 6),
];

/// 功能 id: 模拟一次键盘按键(线上类型 0x11)
pub const KEY_FUNCTION_ID: u8 = 0x10;
/// 功能 id: 宏(线上类型 0x12, 参数是槽位); 抓包验证
pub const MACRO_FUNCTION_ID: u8 = 0x11;
/// "左键按下播放"变体(线上类型 0x3a): 没有抓包, 不支持
pub const MACRO_VARIANT_ID: u8 = 0x18;
pub const MACRO_FUNCTION_IDS: [u8; 2] = [MACRO_FUNCTION_ID, MACRO_VARIANT_ID];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubProfile {
    pub raw: Vec<u8>,
    /// 按钮序号 -> Macro(事件, 重复次数, 启动方式)。宏内容在 GUI 里存在别处(macro.data), 这里只保存要上传的那份
    pub macros: BTreeMap<usize, Macro>,
}

impl HubProfile {
    // --- 低层读写 ---
    pub fn u8(&self, off: usize) -> u8 {
        self.raw[off]
    }

    pub fn i32(&self, off: usize) -> i32 {
        i32::from_le_bytes(self.raw[off..off + 4].try_into().unwrap())
    }

    pub fn set_i32(&mut self, off: usize, v: i64) {
        self.raw[off..off + 4].copy_from_slice(&(v as i32).to_le_bytes());
    }

    pub fn name(&self) -> String {
        let units: Vec<u16> = self.raw[..64].chunks(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
        String::from_utf16_lossy(&units).split('\0').next().unwrap_or("").to_string()
    }

    // --- 语义字段(名字为推断) ---
    /// DPI 档位: 存的是 DPI/50 (16 -> 800), 设备收到的是 值-1
    pub fn dpi_value(&self, i: usize) -> i32 {
        self.i32(0x2804 + 4 * i)
    }

    /// 第 i 档(0 起算)的实际 DPI。
    pub fn dpi(&self, i: usize) -> i64 {
        self.dpi_value(i) as i64 * DPI_STEP
    }

    /// 当前选中的档位(0 起算), 对应 GUI 里点选的那一档。
    pub fn dpi_current(&self) -> i32 {
        self.i32(0x2874)
    }

    pub fn dpi_enabled(&self, i: usize) -> bool {
        self.i32(0x2844 + 4 * i) == 1
    }

    pub fn dpi_color(&self, i: usize) -> (u8, u8, u8) {
        let o = 0x28A4 + 4 * i;
        (self.raw[o], self.raw[o + 1], self.raw[o + 2])
    }

    pub fn light_mode(&self) -> i32 {
        self.i32(0x288C)
    }

    pub fn polling_rate_index(&self) -> i32 {
        self.i32(0x2878)
    }

    pub fn sleep_minutes(&self) -> f64 {
        self.i32(0x2870) as f64 * 0.5
    }

    pub fn deep_sleep_minutes(&self) -> i32 {
        self.i32(0x286C)
    }

    pub fn debounce_ms(&self) -> i32 {
        self.i32(0x2898) * 2
    }

    pub fn light_rgb(&self) -> (u8, u8, u8) {
        (self.raw[0x28A0], self.raw[0x28A1], self.raw[0x28A2])
    }

    // --- 按键 ---
    pub fn button_record(&self, i: usize) -> &[u8] {
        let o = BUTTON_BASE + BUTTON_STRIDE * i;
        &self.raw[o..o + 8]
    }

    pub fn button_function_id(&self, i: usize) -> u8 {
        self.button_record(i)[1]
    }

    /// 把按钮 i 绑定为键盘按键(HID 键码) + 可选修饰键掩码(ctrl=1 shift=2 alt=4 win=8)。
    pub fn set_button_key(&mut self, i: usize, hid_code: u8, modifier: u8) -> Result<()> {
        if modifier > 0x0F {
            bail!("修饰键掩码 {modifier:#x} 超出 0..0xf");
        }
        let o = BUTTON_BASE + BUTTON_STRIDE * i;
        self.macros.remove(&i);
        self.raw[o + 1] = KEY_FUNCTION_ID;
        self.raw[o + 2] = CATEGORY_KEYBOARD;
        self.raw[o + 3] = hid_code;
        self.raw[o + 7] = modifier;
        Ok(())
    }

    /// 把按钮 i 绑定为表里的功能 id(鼠标功能/快捷键/多媒体等)。
    pub fn set_button_function(&mut self, i: usize, function_id: i64) -> Result<()> {
        let id = u8::try_from(function_id).ok().filter(|id| function_wire(*id).is_some());
        let Some(id) = id else {
            bail!("未知功能 id {function_id:#x}");
        };
        if MACRO_FUNCTION_IDS.contains(&id) {
            bail!("宏请用 type: macro 配置, 不能直接指定宏的功能 id");
        }
        let o = BUTTON_BASE + BUTTON_STRIDE * i;
        self.macros.remove(&i);
        self.raw[o + 1] = id;
        self.raw[o + 2] = category_of(id);
        self.raw[o + 3] = 0;
        self.raw[o + 7] = 0;
        Ok(())
    }

    /// 把按钮 i 绑定为宏。只有按钮 0、1、2、3、6、7 有宏槽位(见 `protocol::macros::SLOT_BASE`)。
    pub fn set_button_macro(&mut self, i: usize, events: Vec<MacroEvent>, repeat: i64, mode: u8, name: &str) -> Result<()> {
        let Some(slot) = slot_base(i) else {
            let slots: Vec<usize> = SLOT_BASE.iter().map(|&(b, _)| b).collect();
            bail!("按钮 {i} 没有宏槽位, 只有 {slots:?} 可以绑宏");
        };
        macro_frames(&events, slot, repeat, mode)?; // 先试编码一遍, 尽早报错(太长、延迟不合法等)
        let o = BUTTON_BASE + BUTTON_STRIDE * i;
        self.raw[o + 1] = MACRO_FUNCTION_ID;
        self.raw[o + 2] = CATEGORY_MACRO;
        self.raw[o + 3] = 0;
        self.raw[o + 7] = 0;
        self.macros.insert(i, Macro { events, repeat: repeat as u8, mode, name: name.to_string() });
        Ok(())
    }

    /// 按钮当前绑定的可读描述, 用于日志。
    pub fn describe_button(&self, i: usize) -> String {
        let rec = self.button_record(i);
        let fid = rec[1];
        if fid == KEY_FUNCTION_ID {
            let mut parts: Vec<String> =
                MODIFIER_BITS.iter().filter(|(_, bit)| rec[7] & bit != 0).map(|(n, _)| n.to_string()).collect();
            parts.push(code_to_key(rec[3]));
            return parts.join("+");
        }
        if fid == MACRO_FUNCTION_ID {
            if let Some(mc) = self.macros.get(&i) {
                let mode = start_mode_name(mc.mode).unwrap_or("?");
                let tag = if mc.name.is_empty() { String::new() } else { format!("{}: ", mc.name) };
                let tail = if mc.mode == 0 { format!(", 重复 {} 次)", mc.repeat) } else { ")".into() };
                return format!("macro({tag}{} 个事件, {mode}{tail}", mc.events.len());
            }
        }
        if MACRO_FUNCTION_IDS.contains(&fid) {
            return format!("macro({fid:#x}, 内容未定义)");
        }
        match action_name(fid) {
            Some(name) => name.to_string(),
            None => format!("function {fid:#x}"),
        }
    }

    /// 工厂默认方案: 全 0 的结构体 + `DEFAULT_FIELDS`。
    pub fn factory_default() -> HubProfile {
        let mut raw = vec![0u8; PROFILE_SIZE];
        for &(off, hex) in DEFAULT_FIELDS {
            let data = hex_decode(hex);
            raw[off..off + data.len()].copy_from_slice(&data);
        }
        HubProfile { raw, macros: BTreeMap::new() }
    }
}

pub fn load_pro_data(path: impl AsRef<Path>) -> Result<Vec<HubProfile>> {
    let data = match std::fs::read(path.as_ref()) {
        Ok(d) => d,
        Err(e) => bail!("读取 {} 失败: {e}", path.as_ref().display()),
    };
    let expected = HEADER_SIZE + PROFILE_COUNT * PROFILE_SIZE;
    if data.len() != expected {
        bail!("pro.data 大小 {} != 预期 {expected}, 格式可能随版本变了", data.len());
    }
    Ok((0..PROFILE_COUNT)
        .map(|k| HubProfile {
            raw: data[HEADER_SIZE + k * PROFILE_SIZE..HEADER_SIZE + (k + 1) * PROFILE_SIZE].to_vec(),
            macros: BTreeMap::new(),
        })
        .collect())
}
