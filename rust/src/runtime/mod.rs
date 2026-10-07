//! 运行时: 与设备和系统打交道(HID 通道、蓝牙通道、前台检测、状态监听、切换循环)。
//!
//! 与硬件相关的实现只在 Windows 上编译(`hidapi` 的 windows-native 后端、WinRT); 其余逻辑
//! (重试/重连、应答重发、电量日志等)都通过 trait 注入设备, 在任何平台上都能用假设备测试。

pub mod ble;
pub mod channel;
pub mod foreground;
pub mod monitor;
pub mod switcher;
