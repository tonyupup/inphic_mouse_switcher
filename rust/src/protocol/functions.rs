//! HUB 按键功能码表: 配置里的功能 id -> 按键表包里的 3 字节(类型, 修饰键/参数, 键码/参数)。
//!
//! 从 INPHIC.exe 的 DAT_004ceff4 开头的 64 项表(步长 20 字节: id, b0, b1, b2, 保留)原样导出,
//! 由 FUN_004620f0 在组按键表包(命令 0x08)时查表使用。id 对应 GUI 里哪个下拉项还没逐个核对,
//! 所以这里只存数值。

/// id -> (b0, b1, b2)
pub fn function_wire(id: u8) -> Option<(u8, u8, u8)> {
    FUNCTION_WIRE.iter().find(|(k, _)| *k == id).map(|&(_, v)| v)
}

pub const FUNCTION_WIRE: &[(u8, (u8, u8, u8))] = &[
    (0x00, (0x01, 0x00, 0x00)),
    (0x01, (0x02, 0x00, 0x00)),
    (0x02, (0x03, 0x00, 0x00)),
    (0x03, (0x04, 0x00, 0x00)),
    (0x04, (0x06, 0x00, 0x00)),
    (0x05, (0x05, 0x00, 0x00)),
    (0x09, (0x07, 0x00, 0x00)),
    (0x0a, (0x08, 0x00, 0x00)),
    (0x0b, (0x09, 0x00, 0x00)),
    (0x0c, (0x0a, 0x00, 0x00)),
    (0x0d, (0x0b, 0x00, 0x00)),
    (0x0e, (0x0c, 0x00, 0x00)),
    (0x06, (0x0d, 0x00, 0x00)),
    (0x07, (0x0e, 0x00, 0x00)),
    (0x08, (0x0f, 0x00, 0x00)),
    (0x10, (0x11, 0x00, 0x00)),
    (0x11, (0x12, 0x00, 0x00)),
    (0x13, (0x13, 0x00, 0x00)),
    (0x73, (0x14, 0x00, 0x00)),
    (0x15, (0x2b, 0x00, 0x00)),
    (0x16, (0x10, 0x00, 0x03)),
    (0x17, (0x29, 0x00, 0x03)),
    (0x36, (0x34, 0x00, 0x00)),
    (0x37, (0x35, 0x00, 0x00)),
    (0x38, (0x36, 0x00, 0x00)),
    (0x34, (0x3c, 0x00, 0x00)),
    (0x18, (0x3a, 0x00, 0x00)),
    (0x40, (0x15, 0x00, 0x00)),
    (0x43, (0x16, 0x00, 0x00)),
    (0x44, (0x17, 0x00, 0x00)),
    (0x41, (0x18, 0x00, 0x00)),
    (0x42, (0x19, 0x00, 0x00)),
    (0x47, (0x1a, 0x00, 0x00)),
    (0x45, (0x1b, 0x00, 0x00)),
    (0x46, (0x1c, 0x00, 0x00)),
    (0x58, (0x1d, 0x00, 0x00)),
    (0x57, (0x1e, 0x00, 0x00)),
    (0x51, (0x11, 0x03, 0x12)),
    (0x52, (0x20, 0x00, 0x00)),
    (0x53, (0x21, 0x00, 0x00)),
    (0x54, (0x22, 0x00, 0x00)),
    (0x59, (0x23, 0x00, 0x00)),
    (0x55, (0x24, 0x00, 0x00)),
    (0x50, (0x25, 0x00, 0x00)),
    (0x56, (0x26, 0x00, 0x00)),
    (0x20, (0x11, 0x01, 0x1b)),
    (0x21, (0x11, 0x01, 0x06)),
    (0x22, (0x11, 0x01, 0x19)),
    (0x23, (0x11, 0x01, 0x12)),
    (0x2f, (0x11, 0x01, 0x11)),
    (0x24, (0x11, 0x01, 0x16)),
    (0x29, (0x11, 0x01, 0x13)),
    (0x25, (0x11, 0x01, 0x09)),
    (0x26, (0x11, 0x01, 0x1c)),
    (0x27, (0x11, 0x01, 0x1d)),
    (0x30, (0x11, 0x01, 0x2e)),
    (0x31, (0x11, 0x01, 0x2d)),
    (0x28, (0x11, 0x01, 0x04)),
    (0x2b, (0x11, 0x04, 0x2b)),
    (0x2c, (0x11, 0x08, 0x07)),
    (0x2a, (0x11, 0x04, 0x3d)),
    (0x2d, (0x11, 0x08, 0x15)),
    (0x2e, (0x11, 0x08, 0x0f)),
    (0x32, (0x11, 0x0a, 0x16)),
];

// ---- 按键功能名字 -> 功能 id ----
// 来自 FUN_00457aa0(按键页 selectchanged): GUI 里第 N 个选项对应的功能 id。
// 名字取自 res/lan/EN.xml 的同序号标签; 验证: 出厂默认 中键=wheel_click(3)、后侧键=backward(5)、前侧键=forward(4)。
pub const MOUSE_ACTIONS: &[(&str, u8)] = &[
    ("off", 0), ("left_click", 1), ("right_click", 2), ("wheel_click", 3), ("forward", 4), ("backward", 5),
    ("double_click", 9), ("fire_button", 10), ("easy_aim", 0x16), ("scroll_up", 0xB), ("scroll_down", 0xC),
    ("tilt_right", 0xE), ("tilt_left", 0xD), ("dpi_cycle", 6), ("dpi_up", 7), ("dpi_down", 8),
];

/// 快捷键: 线上就是 `[0x11, 修饰键掩码, 键码]`, 例如 copy = Ctrl+C
pub const SHORTCUT_ACTIONS: &[(&str, u8)] = &[
    ("cut", 0x20), ("copy", 0x21), ("paste", 0x22), ("open", 0x23), ("save", 0x24), ("find", 0x25),
    ("redo", 0x26), ("undo", 0x27), ("select_all", 0x28), ("print", 0x29), ("close_window", 0x2A),
    ("swap_windows", 0x2B), ("show_desktop", 0x2C), ("run_command", 0x2D), ("lock_pc", 0x2E),
    ("screen_capture", 0x32),
];

pub const MEDIA_ACTIONS: &[(&str, u8)] = &[
    ("media_player", 0x40), ("play_pause", 0x41), ("stop", 0x42), ("previous_track", 0x43),
    ("next_track", 0x44), ("volume_up", 0x45), ("volume_down", 0x46), ("mute", 0x47),
    ("browser_home", 0x50), ("browser_favorites", 0x51), ("browser_forward", 0x52),
    ("browser_backward", 0x53), ("browser_stop", 0x54), ("browser_refresh", 0x55),
    ("browser_search", 0x56), ("email", 0x57), ("calculator", 0x58), ("my_computer", 0x59),
];

/// 按名字在动作表里找功能 id。
pub fn lookup(table: &[(&str, u8)], name: &str) -> Option<u8> {
    table.iter().find(|(n, _)| *n == name).map(|&(_, v)| v)
}

/// 动作表里的名字, 按字母序(报错信息用)。
pub fn sorted_names(table: &[(&str, u8)]) -> Vec<String> {
    let mut v: Vec<String> = table.iter().map(|(n, _)| n.to_string()).collect();
    v.sort();
    v
}

/// 按功能 id 反查动作名(依次查鼠标功能、快捷键、多媒体)。
pub fn action_name(function_id: u8) -> Option<&'static str> {
    [MOUSE_ACTIONS, SHORTCUT_ACTIONS, MEDIA_ACTIONS]
        .iter()
        .flat_map(|t| t.iter())
        .find(|(_, v)| *v == function_id)
        .map(|&(n, _)| n)
}

// 按钮记录 record[2] 的类别(GUI 用它决定显示哪一页; 线上不用, 但保持和 GUI 存档一致)
pub const CATEGORY_MOUSE: u8 = 1;
pub const CATEGORY_KEYBOARD: u8 = 4;
pub const CATEGORY_MULTIMEDIA: u8 = 5;
pub const CATEGORY_MACRO: u8 = 7;
pub const CATEGORY_SHORTCUT: u8 = 8;

/// 键盘按键的修饰键位掩码(GUI 勾选框写入 record[7]; 由快捷键码表里的 Alt+F4=[11 04 3d]、Win+D=[11 08 07] 等印证)
pub const MODIFIER_BITS: &[(&str, u8)] = &[("ctrl", 1), ("shift", 2), ("alt", 4), ("win", 8)];

pub fn category_of(function_id: u8) -> u8 {
    if MOUSE_ACTIONS.iter().any(|(_, v)| *v == function_id) {
        CATEGORY_MOUSE
    } else if SHORTCUT_ACTIONS.iter().any(|(_, v)| *v == function_id) {
        CATEGORY_SHORTCUT
    } else if MEDIA_ACTIONS.iter().any(|(_, v)| *v == function_id) {
        CATEGORY_MULTIMEDIA
    } else {
        CATEGORY_MOUSE
    }
}

/// 动作名 -> HUB 界面里的中文名(取自 res/lan/CN.xml, 三张表的名字互不重复), 用于启动时打印当前设置
pub fn action_label_cn(name: &str) -> &'static str {
    match name {
        // 鼠标功能
        "off" => "按键关闭",
        "left_click" => "左键",
        "right_click" => "右键",
        "wheel_click" => "中键",
        "forward" => "前进",
        "backward" => "后退",
        "double_click" => "双击",
        "fire_button" => "连击键",
        "easy_aim" => "DPI-SHIFT",
        "scroll_up" => "向上滚动",
        "scroll_down" => "向下滚动",
        "tilt_right" => "向右滚动",
        "tilt_left" => "向左滚动",
        "dpi_cycle" => "DPI循环",
        "dpi_up" => "DPI +",
        "dpi_down" => "DPI -",
        // 快捷方式
        "cut" => "剪切",
        "copy" => "复制",
        "paste" => "粘贴",
        "open" => "打开",
        "save" => "保存",
        "find" => "查找",
        "redo" => "重做",
        "undo" => "撤销",
        "select_all" => "全选",
        "print" => "打印",
        "close_window" => "关闭窗口",
        "swap_windows" => "切换窗口",
        "show_desktop" => "显示桌面",
        "run_command" => "运行",
        "lock_pc" => "锁定",
        "screen_capture" => "截图",
        // 多媒体
        "media_player" => "媒体播放器",
        "play_pause" => "播放/暂停",
        "stop" => "停止",
        "previous_track" => "上一曲",
        "next_track" => "下一曲",
        "volume_up" => "音量 +",
        "volume_down" => "音量 -",
        "mute" => "静音",
        "browser_home" => "浏览器主页",
        "browser_favorites" => "浏览器收藏夹",
        "browser_forward" => "浏览器前进",
        "browser_backward" => "浏览器后退",
        "browser_stop" => "浏览器停止",
        "browser_refresh" => "浏览器刷新",
        "browser_search" => "浏览器搜索",
        "email" => "Email",
        "calculator" => "计算器",
        "my_computer" => "我的电脑",
        _ => "?",
    }
}
