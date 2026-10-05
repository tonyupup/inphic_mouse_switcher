"""方案: 模型(model)、命令构造(commands)、从配置构建(build)。"""

from inphic.profile.build import ConfiguredProfile, apply_config, from_config, set_button_action
from inphic.profile.commands import (
    build_apply_sequence, cmd_begin, cmd_dpi, cmd_keymap, cmd_light, cmd_misc07, cmd_polling_rate,
)
from inphic.profile.describe import describe_function, describe_profile
from inphic.profile.model import (
    BUTTON_COUNT, BUTTON_NAMES, LIGHT_MODE_PARAMS, LIGHT_MODES, PROFILE_SIZE, HubProfile, load_pro_data,
)

__all__ = [
    "BUTTON_COUNT", "BUTTON_NAMES", "ConfiguredProfile", "HubProfile", "LIGHT_MODES", "LIGHT_MODE_PARAMS",
    "PROFILE_SIZE", "apply_config", "build_apply_sequence", "cmd_begin", "cmd_dpi", "cmd_keymap", "cmd_light",
    "cmd_misc07", "cmd_polling_rate", "describe_function", "describe_profile", "from_config", "load_pro_data",
    "set_button_action",
]
