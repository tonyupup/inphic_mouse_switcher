"""读取 config.yaml: 设备标识、宏库、各方案、按前台程序切换的规则。"""

from __future__ import annotations

from pathlib import Path

import yaml

from inphic.config.macro_library import MacroLibrary, build_library
from inphic.profile import ConfiguredProfile, from_config

REQUIRED_SECTIONS = ("device", "profiles", "rules")


class AppConfig:
    def __init__(self, raw: dict, profiles: dict[str, ConfiguredProfile], macro_library=None):
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


def load_config(config_path="config.yaml") -> AppConfig:
    config_path = Path(config_path)
    try:
        raw = yaml.safe_load(config_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise ValueError(f"找不到配置文件 {config_path}") from None
    except yaml.YAMLError as e:
        raise ValueError(f"{config_path}: yaml 解析失败: {e}") from None
    if not isinstance(raw, dict):
        raise ValueError(f"{config_path}: 顶层需要是字典")
    missing = [k for k in REQUIRED_SECTIONS if not raw.get(k)]
    if missing:
        raise ValueError(f"{config_path}: 缺少必需的段 {missing}")

    # 宏库: macro_files 是文件名/通配符列表(相对配置文件所在目录), 宏按"分组.名字"独立管理, 按钮用 ref 引用
    library: MacroLibrary | None = (
        build_library(raw["macro_files"], config_path.parent) if raw.get("macro_files") else None
    )
    profiles = {name: from_config(name, pdef or {}, library) for name, pdef in raw["profiles"].items()}

    for rule in raw["rules"]:
        if rule.get("profile") not in profiles:
            raise ValueError(
                f"规则 {rule!r} 引用了不存在的方案 {rule.get('profile')!r}, 已有的方案: {sorted(profiles)}"
            )
    return AppConfig(raw, profiles, library)
