//! 宏编码、宏库、与宏抓包逐字节对比。

mod common;

use common::*;
use inphic::config::{MacroLibrary};
use inphic::profile::{build_apply_sequence, from_config, HubProfile};
use inphic::protocol::macros::{encode_macro, macro_frames, parse_steps_value, MESSAGE_LEN};
use std::path::Path;

const STEPS_A: &str = "[{tap: a, delay_ms: 50}]";
const STEPS_B: &str = "[{tap: mouse_left}, {tap: mouse_right}, {tap: b}]";

fn events(steps: &str) -> Vec<inphic::protocol::macros::MacroEvent> {
    parse_steps_value(Some(&yaml(steps)), 50).unwrap()
}

#[test]
fn macro_frames_match_captures() {
    let cap = captured_macro_frames();
    assert_eq!(macro_frames(&events(STEPS_A), 26, 1, 0).unwrap(), cap["A"][2..4]);
    assert_eq!(macro_frames(&events(STEPS_B), 21, 1, 0).unwrap(), cap["B"][2..4]);
}

#[test]
fn whole_sequence_matches_captures() {
    let cap = captured_macro_frames();
    for (tag, button, steps, mode) in [("A", 7, STEPS_A, 0u8), ("B", 6, STEPS_B, 0), ("M2", 7, STEPS_A, 1), ("M3", 7, STEPS_A, 2), ("M4", 7, STEPS_A, 3)] {
        let mut p = HubProfile::factory_default();
        p.set_button_macro(button, events(steps), 1, mode, "").unwrap();
        let seq = build_apply_sequence(&p).unwrap();
        assert_eq!((&seq[0], &seq[5], &seq[6..]), (&cap[tag][0], &cap[tag][1], &cap[tag][2..4]), "{tag}");
    }
}

#[test]
fn via_config_matches_capture() {
    let cap = captured_macro_frames();
    for (tag, start) in [("A", "loop"), ("M2", "stop_on_any_key"), ("M3", "hold"), ("M4", "stop_on_this_key")] {
        let cfg = yaml(&format!("{{buttons: {{side_front: {{type: macro, steps: {STEPS_A}, start: {start}}}}}}}"));
        let seq = from_config("t", &cfg, None).unwrap().build_packets().unwrap();
        assert_eq!(&seq[6..], &cap[tag][2..4], "{tag}");
    }
}

#[test]
fn encoding_rules() {
    // 按键事件
    let e = events("[{tap: a, delay_ms: 50}]");
    assert_eq!(inphic::protocol::macros::encode_event(&e[0]).unwrap().0, vec![0x01 | 50, 0x04]);
    assert_eq!(inphic::protocol::macros::encode_event(&e[1]).unwrap().0, vec![0x81 | 50, 0x04]);
    // 奇数延迟被拒绝, 不悄悄改
    assert!(parse_steps_value(Some(&yaml("[{tap: a, delay_ms: 51}]")), 50).is_err());
    // 扩展延迟
    let e = events("[{down: a, delay_ms: 300}]");
    assert_eq!(inphic::protocol::macros::encode_event(&e[0]).unwrap(), (vec![0x01 | 1, 0x04, 3, 0x03], 2));
    // 消息头与校验
    let m = &encode_macro(&events(STEPS_A), 26, 1, 0).unwrap()[0];
    assert_eq!((m.len(), m[0], m[1], m[2]), (MESSAGE_LEN, 9, 0x6F, 26));
    let s: u32 = m[3..109].iter().map(|&b| b as u32).sum();
    assert_eq!(u16::from_be_bytes([m[109], m[110]]) as u32, s);
}

#[test]
fn long_macro_continues_and_mode_only_in_first_message() {
    let steps: Vec<String> = (0..30).map(|_| "{tap: a, delay_ms: 50}".to_string()).collect();
    let msgs = encode_macro(&events(&format!("[{}]", steps.join(","))), 21, 1, 2).unwrap();
    assert!(msgs.len() >= 2);
    assert_eq!((msgs[0][3], msgs[1][3]), (2, 0));
    assert_eq!(msgs[1][2], 22);
    // 太长
    let many: Vec<String> = (0..400).map(|_| "{tap: a, delay_ms: 50}".to_string()).collect();
    assert!(encode_macro(&events(&format!("[{}]", many.join(","))), 1, 1, 0).is_err());
}

#[test]
fn macro_binding_rules() {
    let mut p = HubProfile::factory_default();
    assert!(p.set_button_macro(4, events(STEPS_A), 1, 0, "").is_err()); // 没有槽位
    assert!(p.set_button_macro(7, events(STEPS_A), 0, 0, "").is_err());
    assert!(p.set_button_macro(7, events(STEPS_A), 256, 0, "").is_err());
    assert!(p.set_button_macro(7, events(STEPS_A), 1, 4, "").is_err());
    p.set_button_macro(7, events(STEPS_A), 1, 0, "").unwrap();
    p.set_button_key(7, 4, 0).unwrap();
    assert!(p.macros.is_empty()); // 改绑清掉宏
    assert!(p.set_button_function(7, 0x11).is_err()); // 不能直接指定宏的功能 id
    for bad in ["[]", "[{tap: a, down: a}]", "[{tap: a, foo: 1}]", "[{tap: nokey}]", "[{tap: mouse_nope}]", "[1]", "[{move: [1]}]"] {
        assert!(parse_steps_value(Some(&yaml(bad)), 50).is_err(), "{bad}");
    }
    assert!(parse_steps_value(None, 50).is_err());
}

fn lib_from(files: &[(&str, &str)]) -> Result<MacroLibrary, inphic::error::ValueError> {
    let mut lib = MacroLibrary::new();
    for (name, text) in files {
        let v = inphic::config::macro_library::parse_yaml(text, Path::new(name))?;
        lib.load_value(v, Path::new(name))?;
    }
    lib.validate_all()?;
    Ok(lib)
}

#[test]
fn library_basics() {
    let lib = lib_from(&[
        ("fps.yaml", "macros:\n  a: [{tap: a}]\n  b:\n    start: hold\n    default_delay_ms: 20\n    steps: [{use: a, times: 2}, {tap: b}]\n"),
        ("common.yaml", "group: common\nmacros:\n  c: [{tap: c}]\n  d: [{use: fps.a}]\n"),
    ])
    .unwrap();
    assert_eq!(lib.names(), vec!["common.c", "common.d", "fps.a", "fps.b"]);
    let b = lib.resolve("fps.b").unwrap();
    assert_eq!((b.start.as_str(), b.steps.len()), ("hold", 3));
    // use 的宏用它自己的默认延迟(50), b 自己的步骤用 20
    assert_eq!(b.steps[0]["delay_ms"], yaml("50"));
    assert_eq!(b.steps[2]["delay_ms"], yaml("20"));
    assert_eq!(lib.resolve("c").unwrap().name, "common.c"); // 唯一短名
}

#[test]
fn library_errors() {
    let dup = lib_from(&[("x.yaml", "macros: {a: [{tap: a}]}"), ("x.yaml", "macros: {a: [{tap: a}]}")]);
    assert!(dup.unwrap_err().0.contains("重复定义"));
    let cyc = lib_from(&[("g.yaml", "macros: {a: [{use: b}], b: [{use: a}]}")]);
    assert!(cyc.unwrap_err().0.contains("循环引用"));
    let amb = lib_from(&[("g.yaml", "macros: {a: [{tap: a}]}"), ("h.yaml", "macros: {a: [{tap: a}]}")]).unwrap();
    assert!(amb.resolve("a").unwrap_err().0.contains("歧义"));
    let near = lib_from(&[("g.yaml", "macros: {rapid_fire: [{tap: a}]}")]).unwrap();
    assert!(near.resolve("g.rapid_fir").unwrap_err().0.contains("你是不是想写"));
    for bad in ["macros: {a: 5}", "macros: {a: {steps: []}}", "macros: {a: {steps: [{tap: a}], start: x}}", "macros: {a: {steps: [{tap: a}], repeat: 0}}", "macros: {a: {steps: [{tap: a}], bogus: 1}}", "bogus: 1", "macros: {'a.b': [{tap: a}]}"] {
        assert!(lib_from(&[("g.yaml", bad)]).is_err(), "{bad}");
    }
}

#[test]
fn library_yaml_anchors_and_flatten() {
    let lib = lib_from(&[(
        "fps.yaml",
        "x-click: &click\n  - {down: mouse_left, delay_ms: 20}\n  - {up: mouse_left, delay_ms: 20}\nmacros:\n  t: [*click, *click, {tap: r}]\n",
    )])
    .unwrap();
    assert_eq!(lib.resolve("fps.t").unwrap().steps.len(), 5);
}

#[test]
fn real_macro_files_load() {
    let lib = inphic::config::build_library(&["macros/*.yaml".to_string()], &root()).unwrap();
    assert_eq!(lib.names().len(), 8);
    assert!(inphic::config::build_library(&["nope/*.yaml".to_string()], &root()).is_err());
}

#[test]
fn real_config_loads_and_resolves_rules() {
    let cfg = inphic::config::load_config(root().join("config.yaml")).unwrap();
    assert_eq!(cfg.resolve_profile_name(Some("crossfire.exe")), Some("cf"));
    assert_eq!(cfg.resolve_profile_name(Some("notepad.exe")), Some("default"));
    assert_eq!(cfg.resolve_profile_name(None), Some("default"));
    assert_eq!(cfg.device.vendor_id, 0x1D57);
    assert_eq!(cfg.packet_delay_ms, 520);
}
