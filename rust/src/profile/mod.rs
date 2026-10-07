//! 方案: 模型(model)、命令构造(commands)、从配置构建(build)、中文描述(describe)。

pub mod build;
pub mod commands;
pub mod defaults;
pub mod describe;
pub mod model;

pub use build::{apply_config, from_config, set_button_action, ConfiguredProfile, MacroLibraryLookup};
pub use commands::{
    build_apply_payloads, build_apply_sequence, cmd_begin, cmd_dpi, cmd_keymap, cmd_light, cmd_misc07, cmd_polling_rate,
};
pub use describe::{describe_function, describe_profile, describe_status_change};
pub use model::{load_pro_data, HubProfile, BUTTON_COUNT, BUTTON_NAMES, PROFILE_SIZE};
