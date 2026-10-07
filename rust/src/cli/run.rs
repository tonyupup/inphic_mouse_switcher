//! run 子命令: 常驻运行, 按前台程序切换方案。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;

use crate::config::{describe_config, load_config};
use crate::runtime::switcher::ProfileSwitcher;

pub fn run(config_path: &str) -> Result<u8> {
    let config = load_config(config_path)?; // 先加载配置: 配置错误在碰设备之前就报
    println!("{}", describe_config(&config, Path::new(config_path))); // 启动时先把当前生效的设置完整打印出来
    let switcher = ProfileSwitcher::new(Arc::new(config))?;

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))?;
    }
    switcher.run(&stop);
    log::info!("退出");
    switcher.close();
    Ok(0)
}
