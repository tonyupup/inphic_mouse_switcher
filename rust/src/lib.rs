//! inphic: Inphic KP 8K / IN 系列鼠标的自制驱动。
//!
//! 严格分层, 下层不能引用上层:
//! `protocol`(线上格式纯函数) → `profile`(方案模型/命令/帧) → `config`(yaml + 宏库) → `runtime`(设备 I/O、前台检测、切换循环) → `cli`。

pub mod cli;
pub mod config;
pub mod error;
pub mod profile;
pub mod protocol;
pub mod runtime;
pub mod yamlutil;
