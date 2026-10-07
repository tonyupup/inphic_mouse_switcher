//! 读取 config.yaml: 设备标识、宏库、各方案、按前台程序切换的规则。

use std::path::Path;

use indexmap::IndexMap;
use serde_yaml::{Mapping, Value};

use crate::bail;
use crate::config::macro_library::{build_library, read_yaml, MacroLibrary};
use crate::error::Result;
use crate::profile::{from_config, ConfiguredProfile};
use crate::protocol::bluetooth::parse_address;
use crate::yamlutil::{as_int, repr, scalar_string};

const REQUIRED_SECTIONS: [&str; 3] = ["device", "profiles", "rules"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    /// 蓝牙连着用蓝牙, 否则用接收器
    Auto,
    Dongle,
    Bluetooth,
}

impl Connection {
    pub fn as_str(self) -> &'static str {
        match self {
            Connection::Auto => "auto",
            Connection::Dongle => "dongle",
            Connection::Bluetooth => "bluetooth",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BluetoothConfig {
    /// 原样写在配置里的地址(显示用)
    pub address: String,
    pub address_value: u64,
    pub packet_delay_ms: u64,
}

#[derive(Debug, Clone)]
pub struct DeviceConfig {
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
    /// device.bluetooth 段(没写 = 不用蓝牙)
    pub bluetooth: Option<BluetoothConfig>,
    pub connection: Connection,
}

#[derive(Debug, Clone)]
pub struct Rule {
    /// 进程名(原样), 或 `*`
    pub process: String,
    pub profile: String,
}

pub struct AppConfig {
    pub poll_interval_ms: u64,
    pub packet_delay_ms: u64,
    pub device: DeviceConfig,
    pub rules: Vec<Rule>,
    pub profiles: IndexMap<String, ConfiguredProfile>,
    pub macro_library: Option<MacroLibrary>,
}

impl AppConfig {
    /// 前台进程名(小写) -> 方案名: 第一个匹配的规则生效, 没有匹配的走 `*`。
    pub fn resolve_profile_name(&self, process_name: Option<&str>) -> Option<&str> {
        if let Some(name) = process_name.filter(|n| !n.is_empty()) {
            for rule in &self.rules {
                let proc = rule.process.to_lowercase();
                if proc != "*" && proc == name {
                    return Some(&rule.profile);
                }
            }
        }
        self.rules.iter().find(|r| r.process == "*").map(|r| r.profile.as_str())
    }
}

fn int_field(map: &Mapping, key: &str, wher: &str) -> Result<Option<i64>> {
    match map.get(key) {
        None => Ok(None),
        Some(v) => match as_int(v) {
            Some(n) => Ok(Some(n)),
            None => bail!("{wher}: {key} 需要是整数, 实际 {}", repr(v)),
        },
    }
}

fn u16_field(map: &Mapping, key: &str, default: Option<u16>, wher: &str) -> Result<u16> {
    match int_field(map, key, wher)? {
        Some(n) => match u16::try_from(n) {
            Ok(v) => Ok(v),
            Err(_) => bail!("{wher}: {key} = {n} 超出 0..65535"),
        },
        None => match default {
            Some(d) => Ok(d),
            None => bail!("{wher}: device 里缺少 {key}"),
        },
    }
}

impl DeviceConfig {
    /// 校验并解析 `device` 段。`require_usage` 为 false 时(status 只读)usage_page/usage 缺省取接口 2 的配置集合。
    pub fn from_value(dev: &Value, config_path: &Path) -> Result<DeviceConfig> {
        let wher = config_path.display().to_string();
        let Value::Mapping(dev) = dev else {
            bail!("{wher}: device 需要是字典");
        };
        let connection = match dev.get("connection") {
            None => Connection::Auto,
            Some(Value::String(s)) if s == "auto" => Connection::Auto,
            Some(Value::String(s)) if s == "dongle" => Connection::Dongle,
            Some(Value::String(s)) if s == "bluetooth" => Connection::Bluetooth,
            Some(other) => bail!("{wher}: device.connection 是 {}, 可选: auto, dongle, bluetooth", repr(other)),
        };
        let bluetooth = match dev.get("bluetooth") {
            None | Some(Value::Null) => {
                if connection == Connection::Bluetooth {
                    bail!("{wher}: device.connection 是 bluetooth, 但没有 device.bluetooth.address");
                }
                None
            }
            Some(Value::Mapping(bt)) => {
                let addr = bt.get("address").and_then(scalar_string).filter(|a| !a.is_empty());
                let Some(addr) = addr else {
                    bail!("{wher}: device.bluetooth 需要写 address（如 78:87:11:75:9D:56）");
                };
                let address_value = parse_address(&addr)?;
                let packet_delay_ms = match int_field(bt, "packet_delay_ms", &wher)? {
                    Some(n) if n >= 0 => n as u64,
                    Some(n) => bail!("{wher}: device.bluetooth.packet_delay_ms = {n} 不能是负数"),
                    None => 200,
                };
                Some(BluetoothConfig { address: addr, address_value, packet_delay_ms })
            }
            Some(_) => bail!("{wher}: device.bluetooth 需要写 address（如 78:87:11:75:9D:56）"),
        };
        Ok(DeviceConfig {
            vendor_id: u16_field(dev, "vendor_id", None, &wher)?,
            product_id: u16_field(dev, "product_id", None, &wher)?,
            usage_page: u16_field(dev, "usage_page", Some(0xFF00), &wher)?,
            usage: u16_field(dev, "usage", Some(1), &wher)?,
            bluetooth,
            connection,
        })
    }
}

/// 只读取并校验 device 段(status 子命令用, 不加载方案和宏库)。
pub fn read_device_section(config_path: impl AsRef<Path>) -> Result<DeviceConfig> {
    let path = config_path.as_ref();
    if !path.exists() {
        bail!("{} 不存在, 或里面没有 device 段", path.display());
    }
    let raw = read_yaml(path)?;
    match raw.get("device") {
        Some(dev) => DeviceConfig::from_value(dev, path),
        None => bail!("{} 不存在, 或里面没有 device 段", path.display()),
    }
}

fn ms_field(raw: &Mapping, key: &str, default: u64, wher: &str) -> Result<u64> {
    match int_field(raw, key, wher)? {
        Some(n) if n >= 0 => Ok(n as u64),
        Some(n) => bail!("{wher}: {key} = {n} 不能是负数"),
        None => Ok(default),
    }
}

pub fn load_config(config_path: impl AsRef<Path>) -> Result<AppConfig> {
    let config_path = config_path.as_ref();
    let wher = config_path.display().to_string();
    if !config_path.exists() {
        bail!("找不到配置文件 {wher}");
    }
    let raw = read_yaml(config_path)?;
    let Value::Mapping(raw) = raw else {
        bail!("{wher}: 顶层需要是字典");
    };
    let truthy = |v: Option<&Value>| match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Sequence(s)) => !s.is_empty(),
        Some(Value::Mapping(m)) => !m.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(_) => true,
    };
    let missing: Vec<&str> = REQUIRED_SECTIONS.iter().copied().filter(|k| !truthy(raw.get(*k))).collect();
    if !missing.is_empty() {
        bail!("{wher}: 缺少必需的段 {}", crate::error::fmt_list(&missing));
    }

    // 宏库: macro_files 是文件名/通配符列表(相对配置文件所在目录), 宏按"分组.名字"独立管理, 按钮用 ref 引用
    let library: Option<MacroLibrary> = if truthy(raw.get("macro_files")) {
        let patterns = match &raw["macro_files"] {
            Value::Sequence(s) => s.iter().map(|v| scalar_string(v).ok_or(v)).collect::<std::result::Result<Vec<_>, _>>(),
            other => Err(other),
        };
        let patterns = match patterns {
            Ok(p) => p,
            Err(v) => bail!("{wher}: macro_files 需要是文件名/通配符的列表, 实际 {}", repr(v)),
        };
        let base = config_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        Some(build_library(&patterns, base)?)
    } else {
        None
    };

    let Value::Mapping(profile_defs) = &raw["profiles"] else {
        bail!("{wher}: profiles 需要是 方案名: 配置 的字典");
    };
    let mut profiles = IndexMap::new();
    for (name, pdef) in profile_defs {
        let Some(name) = scalar_string(name) else {
            bail!("{wher}: 方案名 {} 不合法", repr(name));
        };
        let cp = from_config(&name, pdef, library.as_ref().map(|l| l as &dyn crate::profile::MacroLibraryLookup))?;
        profiles.insert(name, cp);
    }

    let device = DeviceConfig::from_value(&raw["device"], config_path)?;

    let Value::Sequence(rule_defs) = &raw["rules"] else {
        bail!("{wher}: rules 需要是列表");
    };
    let mut rules = Vec::new();
    for rule in rule_defs {
        let Value::Mapping(m) = rule else {
            bail!("{wher}: 规则需要是字典, 实际 {}", repr(rule));
        };
        let profile = m.get("profile").and_then(scalar_string);
        let exists = profile.as_ref().is_some_and(|p| profiles.contains_key(p));
        if !exists {
            let mut have: Vec<&str> = profiles.keys().map(|s| s.as_str()).collect();
            have.sort();
            bail!(
                "规则 {} 引用了不存在的方案 {}, 已有的方案: {}",
                repr(rule),
                m.get("profile").map(repr).unwrap_or_else(|| "None".into()),
                crate::error::fmt_list(&have)
            );
        }
        let Some(process) = m.get("process").and_then(scalar_string) else {
            bail!("{wher}: 规则 {} 缺少 process", repr(rule));
        };
        rules.push(Rule { process, profile: profile.unwrap() });
    }
    Ok(AppConfig {
        poll_interval_ms: ms_field(&raw, "poll_interval_ms", 150, &wher)?,
        packet_delay_ms: ms_field(&raw, "packet_delay_ms", 520, &wher)?,
        device,
        rules,
        profiles,
        macro_library: library,
    })
}
