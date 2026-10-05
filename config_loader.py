"""把 config.yaml (人读) + templates.yaml (机器管理的原始字节) 拼成 Profile 对象。"""

from pathlib import Path

import yaml

from hub_profile import from_config
from macro_library import build_library
from mouse_protocol import ButtonTable, KeyAction, MouseFunction, Profile


def _parse_action(d: dict):
    kind = d.get("type")
    if kind == "key":
        return KeyAction(d["key"])
    if kind == "function":
        return MouseFunction(d["id"])
    raise ValueError(f"配置里有不认识的 action type: {d!r}，只支持 key / function")


class AppConfig:
    def __init__(self, raw: dict, profiles: dict[str, Profile], macro_library=None):
        self.raw = raw
        self.profiles = profiles
        self.macro_library = macro_library

    @property
    def poll_interval_ms(self) -> int:
        return self.raw.get("poll_interval_ms", 150)

    @property
    def packet_delay_ms(self) -> int:
        return self.raw.get("packet_delay_ms", 520)

    @property
    def device(self) -> dict:
        return self.raw["device"]

    @property
    def rules(self) -> list[dict]:
        return self.raw["rules"]

    def resolve_profile_name(self, process_name: str | None) -> str | None:
        if process_name:
            for rule in self.rules:
                proc = rule["process"].lower()
                if proc != "*" and proc == process_name:
                    return rule["profile"]
        for rule in self.rules:
            if rule["process"] == "*":
                return rule["profile"]
        return None


def load_config(config_path="config.yaml", templates_path=None) -> AppConfig:
    config_path = Path(config_path)
    with open(config_path, "r", encoding="utf-8") as f:
        raw = yaml.safe_load(f)

    templates_path = Path(templates_path) if templates_path else config_path.parent / "templates.yaml"
    templates = None  # 只有用到 `template:` 的旧式方案才需要; 语义配置的方案不依赖抓包模板

    # 宏库: macro_files 是文件名/通配符列表(相对配置文件所在目录), 宏按"分组.名字"独立管理, 按钮用 ref 引用
    library = build_library(raw.get("macro_files"), config_path.parent) if raw.get("macro_files") else None

    profiles = {}
    for name, pdef in raw["profiles"].items():
        if "template" not in pdef:
            profiles[name] = from_config(name, pdef or {}, library)
            continue
        if templates is None:
            with open(templates_path, "r", encoding="utf-8") as f:
                templates = yaml.safe_load(f)
        template_name = pdef["template"]
        if template_name not in templates:
            raise ValueError(f"profile '{name}' 引用的模板 '{template_name}' 在 templates.yaml 里不存在")
        buttons = None
        if "buttons" in pdef:
            bindings = {btn: _parse_action(action) for btn, action in pdef["buttons"].items()}
            buttons = ButtonTable(bindings)
        profiles[name] = Profile(name, templates[template_name], buttons)

    return AppConfig(raw, profiles, library)
