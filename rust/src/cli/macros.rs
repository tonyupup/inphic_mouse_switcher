//! macros 子命令: 查看宏库(只读, 不碰设备)。show 会展开 use/锚点, 打印最终步骤、事件数、占几个槽位、大约播放多久。

use anyhow::Result;

use crate::bail;
use crate::config::{load_config, MacroLibrary};
use crate::protocol::macros::{encode_macro, parse_steps, start_mode, EVENT_LIMIT};
use crate::yamlutil::repr;

fn library(config_path: &str) -> crate::error::Result<MacroLibrary> {
    match load_config(config_path)?.macro_library {
        Some(lib) => Ok(lib),
        None => bail!("{config_path} 里没有 macro_files, 没有宏库可看"),
    }
}

fn pad(s: &str, width: usize) -> String {
    format!("{s}{}", " ".repeat(width.saturating_sub(s.chars().count())))
}

pub fn macros_list(config_path: &str) -> Result<u8> {
    let lib = library(config_path)?;
    for (group, names) in lib.groups() {
        println!("[{group}]");
        for n in names {
            let d = lib.resolve(&format!("{group}.{n}"))?;
            println!(
                "  {} {} {:>3} 步  {}",
                pad(&format!("{group}.{n}"), 24),
                pad(&d.start, 17),
                d.steps.len(),
                d.description
            );
        }
    }
    Ok(0)
}

pub fn macros_show(config_path: &str, reference: &str) -> Result<u8> {
    let d = library(config_path)?.resolve(reference)?;
    let events = parse_steps(&d.steps, 50)?;
    let msgs = encode_macro(&events, 1, d.repeat, start_mode(&d.start).unwrap())?;
    println!("{}  ({})", d.name, d.source);
    if !d.description.is_empty() {
        println!("  说明: {}", d.description);
    }
    println!("  启动方式: {}   循环次数: {}", d.start, d.repeat);
    println!(
        "  展开后 {} 步 -> {} 个事件, 占 {}/5 个槽位 (单条最多约 {EVENT_LIMIT} 字节事件)",
        d.steps.len(),
        events.len(),
        msgs.len()
    );
    println!("  一遍大约 {} ms", events.iter().map(|e| e.delay_ms).sum::<i64>());
    for (i, s) in d.steps.iter().enumerate() {
        println!("  {:>3}. {}", i + 1, repr(s));
    }
    Ok(0)
}
