//! 测试共用的夹具加载。夹具都来自真实 USB 抓包(见 tests/fixtures)。
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

use inphic::profile::{HubProfile, PROFILE_SIZE};
use inphic::protocol::hex_decode;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf() // 仓库根目录(config.yaml、macros/、fixtures/ 与 Python 版共用)
}

fn fixture(name: &str) -> PathBuf {
    root().join("fixtures").join(name)
}

/// usb / profile_switch / key_remap 三份抓包里全部不重复的 OUT 帧。
pub fn captured_frames() -> Vec<Vec<u8>> {
    std::fs::read_to_string(fixture("captured_frames.txt"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| hex_decode(l.trim()))
        .collect()
}

/// 宏抓包的 4 帧(开始包、按键表、宏分片 0、宏分片 1), 按标签分: A、B、M2、M3、M4。
pub fn captured_macro_frames() -> BTreeMap<String, Vec<Vec<u8>>> {
    let mut out: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
    for line in std::fs::read_to_string(fixture("captured_macro_frames.txt")).unwrap().lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (tag, hex) = line.split_once(' ').unwrap();
        out.entry(tag.to_string()).or_default().push(hex_decode(hex));
    }
    out
}

/// profile_switch 抓包里 cf / default 两个方案各 6 个命令帧。
pub fn profile_switch_frames() -> BTreeMap<String, Vec<Vec<u8>>> {
    let raw: BTreeMap<String, Vec<String>> =
        serde_yaml::from_str(&std::fs::read_to_string(fixture("profile_switch_frames.yaml")).unwrap()).unwrap();
    raw.into_iter().map(|(k, v)| (k, v.iter().map(|h| hex_decode(h)).collect())).collect()
}

/// HUB 存档里的方案 5("配置1")和方案 6("CF")。
pub fn profiles_5_6() -> (HubProfile, HubProfile) {
    let mut blob = Vec::new();
    flate2::read::ZlibDecoder::new(std::fs::File::open(fixture("profiles_5_6.bin.z")).unwrap())
        .read_to_end(&mut blob)
        .unwrap();
    let mk = |b: &[u8]| HubProfile { raw: b.to_vec(), macros: Default::default() };
    (mk(&blob[..PROFILE_SIZE]), mk(&blob[PROFILE_SIZE..]))
}

pub fn yaml(text: &str) -> serde_yaml::Value {
    serde_yaml::from_str(text).unwrap()
}
