//! 极简日志: 全部写 stderr, 格式 `2026-10-07 12:00:00,123 [INFO] 消息`(与 Python 版 logging 一致)。

use log::{Level, LevelFilter, Log, Metadata, Record};

struct Logger {
    level: LevelFilter,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let name = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => "WARNING",
            Level::Info => "INFO",
            Level::Debug | Level::Trace => "DEBUG",
        };
        eprintln!("{} [{name}] {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f"), record.args());
    }

    fn flush(&self) {}
}

/// `verbose` 只对 `run` 有意义: 打开 DEBUG。其余子命令只显示警告和错误。
pub fn init(verbose: bool) {
    let level = if verbose { LevelFilter::Debug } else { LevelFilter::Info };
    let level = if std::env::args().nth(1).as_deref() == Some("run") { level } else { LevelFilter::Warn };
    let logger: &'static Logger = Box::leak(Box::new(Logger { level }));
    let _ = log::set_logger(logger);
    log::set_max_level(level);
}
