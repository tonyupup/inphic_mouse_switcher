//! 命令行入口:
//!
//! ```text
//! inphic run     [-c config.yaml] [-v]                按前台程序切换方案(要常驻运行); 启动时先打印当前设置
//! inphic show    [-c config.yaml] [--payloads]        只打印当前设置(中文), 不连接鼠标; --payloads 另打印将发送的字节(干跑)
//! inphic status  [-c config.yaml] [--watch] [--via bluetooth]   读电量等状态(只读)
//! inphic macros list|show <分组.名字> [-c config.yaml]   查看宏库(只读)
//! ```

use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use crate::error::ValueError;

pub mod logger;
pub mod macros;
pub mod run;
pub mod show;
pub mod status;

pub const DEFAULT_CONFIG: &str = "config.yaml";

/// 直接把这句话打到 stderr 并以 1 退出, 不加"错误:"前缀(对应 Python 版的 `SystemExit(msg)`)。
#[derive(Debug)]
pub struct Fatal(pub String);

impl std::fmt::Display for Fatal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Fatal {}

#[derive(Parser)]
#[command(name = "inphic", version, about = "Inphic KP 8K 鼠标自制驱动")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// 按前台程序自动切换鼠标方案
    Run {
        /// 配置文件
        #[arg(short, long, default_value = DEFAULT_CONFIG)]
        config: String,
        /// 日志里连每条状态报告的原始字节也打印(DEBUG)
        #[arg(short, long)]
        verbose: bool,
    },
    /// 用中文打印当前设置, 不连接鼠标
    Show {
        #[arg(short, long, default_value = DEFAULT_CONFIG)]
        config: String,
        /// 另外打印每个方案将要发送的字节(干跑, 不发送)
        #[arg(long)]
        payloads: bool,
    },
    /// 读取鼠标状态(电量等), 只读
    Status {
        #[arg(short, long, default_value = DEFAULT_CONFIG)]
        config: String,
        /// 从哪条连接读: 默认有接收器读接收器, 否则读蓝牙
        #[arg(long, value_enum, default_value_t = Via::Auto)]
        via: Via,
        /// 持续打印全部状态报告(观察 DPI 键等变化)
        #[arg(long)]
        watch: bool,
    },
    /// 查看宏库, 只读
    Macros {
        #[command(subcommand)]
        command: MacrosCommand,
    },
}

#[derive(Subcommand)]
pub enum MacrosCommand {
    /// 列出所有分组和宏
    List {
        #[arg(short, long, default_value = DEFAULT_CONFIG)]
        config: String,
    },
    /// 展开一个宏: 步骤、槽位、时长
    Show {
        /// 分组.名字, 如 fps.rapid_fire
        reference: String,
        #[arg(short, long, default_value = DEFAULT_CONFIG)]
        config: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Via {
    Auto,
    Dongle,
    Bluetooth,
}

pub fn main() -> ExitCode {
    let cli = Cli::parse();
    let verbose = matches!(cli.command, Command::Run { verbose: true, .. });
    logger::init(verbose);
    let result = match cli.command {
        Command::Run { config, .. } => run::run(&config),
        Command::Show { config, payloads } => show::show(&config, payloads),
        Command::Status { config, watch, via } => status::status(&config, watch, via),
        Command::Macros { command } => match command {
            MacrosCommand::List { config } => macros::macros_list(&config),
            MacrosCommand::Show { reference, config } => macros::macros_show(&config, &reference),
        },
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) if e.downcast_ref::<Fatal>().is_some() => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
        Err(e) => match e.downcast_ref::<ValueError>() {
            // 配置/宏库里的错误: 显示成一句人话, 不要回溯
            Some(v) => {
                eprintln!("配置错误: {v}");
                ExitCode::from(2)
            }
            None => {
                eprintln!("错误: {e:#}");
                ExitCode::from(1)
            }
        },
    }
}
