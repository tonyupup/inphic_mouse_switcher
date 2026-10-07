//! 配置: yaml 加载(loader)与宏库(macro_library)。

pub mod describe;
pub mod loader;
pub mod macro_library;

pub use describe::describe_config;
pub use loader::{load_config, read_device_section, AppConfig, Connection, DeviceConfig, Rule};
pub use macro_library::{build_library, MacroDef, MacroLibrary};
