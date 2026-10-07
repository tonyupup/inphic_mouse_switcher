//! 配置/参数错误。对应 Python 版的 `ValueError`: 由 `cli::main` 显示成 `配置错误: ...`(退出码 2), 不打印回溯。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueError(pub String);

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValueError {}

pub type Result<T> = std::result::Result<T, ValueError>;

/// `bail!("...{}", x)`: 直接返回一个 `ValueError`。
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::error::ValueError(format!($($arg)*)))
    };
}

/// 把字符串列表格式化成 `['a', 'b']`, 与 Python 版报错信息保持一致。
pub fn fmt_list<S: AsRef<str>>(items: &[S]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format!("'{}'", s.as_ref())).collect();
    format!("[{}]", inner.join(", "))
}
