//! 协议层与方案层: 与真实 USB 抓包逐字节对比。

mod common;

use common::*;
use inphic::profile::{build_apply_sequence, cmd_keymap, cmd_light, cmd_polling_rate, from_config, HubProfile};
use inphic::protocol::frame::{build_chunk_frame, build_frame, parse_chunk_frame, parse_frame};
use inphic::protocol::keycodes::key_to_code;

#[test]
fn captured_frames_roundtrip() {
    let frames = captured_frames();
    assert!(!frames.is_empty());
    for f in frames {
        let (idx, data) = parse_chunk_frame(&f).unwrap();
        assert_eq!(build_chunk_frame(&data, idx as usize).unwrap(), f);
    }
    for (_, frames) in captured_macro_frames() {
        for f in frames {
            parse_chunk_frame(&f).unwrap();
        }
    }
}

#[test]
fn frame_rejections() {
    let mut f = build_frame(&[0x0C, 0x0A, 1, 0xFE, 1, 0xFE, 0, 0, 0, 0]).unwrap();
    f[10] ^= 1;
    assert!(parse_frame(&f).is_err());
    assert!(build_frame(&[0x0C, 0x09, 0, 0]).is_err());
    assert!(build_chunk_frame(&[0; 60], 0).is_err());
    assert!(parse_frame(&build_chunk_frame(&[1, 2], 1).unwrap()).is_err());
}

#[test]
fn profile_names() {
    let (default, cf) = profiles_5_6();
    assert_eq!(default.name(), "配置1");
    assert_eq!(cf.name(), "CF");
}

#[test]
fn default_profile_reproduces_all_six_frames() {
    let (default, _) = profiles_5_6();
    assert_eq!(build_apply_sequence(&default).unwrap(), profile_switch_frames()["default"]);
}

#[test]
fn cf_profile_reproduces_capture_when_light_mode_matches() {
    let (_, mut cf) = profiles_5_6();
    cf.set_i32(0x288C, 5); // 抓包时 CF 方案的灯光模式是 5(常亮DPI), 存档里现在是 0
    assert_eq!(build_apply_sequence(&cf).unwrap(), profile_switch_frames()["cf"]);
}

#[test]
fn cf_profile_now_differs_only_in_light_frame() {
    let (_, cf) = profiles_5_6();
    let got = build_apply_sequence(&cf).unwrap();
    let cap = &profile_switch_frames()["cf"];
    let diff: Vec<usize> = (0..6).filter(|&i| got[i] != cap[i]).collect();
    assert_eq!(diff, vec![2]);
}

#[test]
fn polling_rate_codes() {
    let (default, _) = profiles_5_6();
    for (idx, code) in [(0, 0x20u8), (1, 0x10), (2, 0x08), (3, 0x04), (4, 0x02), (5, 0x01), (6, 0x40)] {
        let mut p = default.clone();
        p.set_i32(0x2878, idx);
        let pl = cmd_polling_rate(&p).unwrap();
        assert_eq!((pl[3], pl[4]), (code, !code));
    }
}

#[test]
fn rebinding_updates_keymap_checksum() {
    let (_, mut cf) = profiles_5_6();
    cf.set_button_key(7, key_to_code("f12").unwrap(), 0).unwrap();
    let frame = build_frame(&cmd_keymap(&cf).unwrap()).unwrap();
    parse_frame(&frame).unwrap();
    assert_eq!(&frame[6 + 21..6 + 24], &[0x11, 0x00, 0x45]);
    let s: u32 = frame[6..60].iter().map(|&b| b as u32).sum();
    assert_eq!(u16::from_be_bytes([frame[60], frame[61]]) as u32, s);
    assert_eq!(cmd_light(&cf)[0], 0x05);
}

#[test]
fn unsupported_macro_bindings_are_refused() {
    let (_, cf) = profiles_5_6();
    let mut p = cf.clone();
    p.raw[0x28E4 + 0x1338 * 3 + 1] = 0x18;
    assert!(cmd_keymap(&p).is_err());
    let mut p = cf;
    p.raw[0x28E4 + 0x1338 * 3 + 1] = 0x11;
    assert!(cmd_keymap(&p).is_err());
}

#[test]
fn config_reproduces_real_cf_profile() {
    let (default, cf) = profiles_5_6();
    let cfg = yaml(
        r#"{polling_rate: 1000, light: {mode: off},
            buttons: {middle_click: {type: key, key: "1"}, side_back: {type: key, key: "6"}, side_front: {type: key, key: "5"}}}"#,
    );
    assert_eq!(from_config("cf", &cfg, None).unwrap().build_packets().unwrap(), build_apply_sequence(&cf).unwrap());
    assert_eq!(
        from_config("d", &serde_yaml::Value::Null, None).unwrap().build_packets().unwrap(),
        build_apply_sequence(&default).unwrap()
    );
    let _ = HubProfile::factory_default();
}

#[test]
fn bad_config_is_rejected_not_ignored() {
    let all8 = "[800, 1600, 3200, 6500, 13000, 26000, 500, 500]";
    let bad = [
        "{polling_rat: 1000}".to_string(),
        "{polling_rate: 999}".into(),
        "{light: {mode: disco}}".into(),
        "{light: {rgb: [0, 256, 0]}}".into(),
        "{light: {colour: 1}}".into(),
        "{light: {brightness: 0}}".into(),
        "{light: {brightness: 9}}".into(),
        "{light: {speed: 9}}".into(),
        "{dpi: {levels: [1, 2, 3]}}".into(),
        "{dpi: {levels: [801, 1600, 3200, 6500, 13000, 26000, 500, 500]}}".into(),
        "{dpi: {levels: [800, 1600, 3200, 6500, 13000, 26050, 500, 500]}}".into(),
        "{dpi: {levels: [0, 1600, 3200, 6500, 13000, 26000, 500, 500]}}".into(),
        "{dpi: {current: 0}}".into(),
        "{dpi: {current: 9}}".into(),
        "{dpi: {current: 7}}".into(),
        "{dpi: {enabled: [true, false, true, true, true, true, false, false]}}".into(),
        "{dpi: {levels: []}}".into(),
        "{dpi: {levels: [800, 800, 800, 800, 800, 800, 800, 800, 800]}}".into(),
        "{dpi: {current: 1700}}".into(),
        "{dpi: {colors: [[0, 0, 300], [0, 0, 300], [0, 0, 300], [0, 0, 300], [0, 0, 300], [0, 0, 300], [0, 0, 300], [0, 0, 300]]}}".into(),
        "{dpi: {step: 1}}".into(),
        "{performance: {sleep: 1}}".into(),
        "{performance: {sleep_minutes: 0.3}}".into(),
        "{performance: {sleep_minutes: 31}}".into(),
        "{performance: {deep_sleep_minutes: 0}}".into(),
        "{performance: {deep_sleep_minutes: 61}}".into(),
        "{performance: {deep_sleep_minutes: 2.5}}".into(),
        "{performance: {debounce_ms: 3}}".into(),
        "{performance: {debounce_ms: 52}}".into(),
        "{performance: {ripple_control: \"yes\"}}".into(),
        "{buttons: {nonexistent: {type: key, key: a}}}".into(),
        "{buttons: {middle_click: {type: macro}}}".into(),
        "{buttons: {99: {type: key, key: a}}}".into(),
    ];
    let _ = all8;
    for b in bad {
        assert!(from_config("x", &yaml(&b), None).is_err(), "应该报错: {b}");
    }
}

#[test]
fn light_mode_name_and_index_are_equal() {
    let a = from_config("a", &yaml("{light: {mode: breathing}}"), None).unwrap().build_packets().unwrap();
    let b = from_config("b", &yaml("{light: {mode: 2}}"), None).unwrap().build_packets().unwrap();
    assert_eq!(a, b);
}

#[test]
fn dpi_shorthand_and_partial_levels() {
    let p = from_config("a", &yaml("{dpi: 1600}"), None).unwrap().profile;
    assert_eq!((p.dpi(0), p.dpi_enabled(0), p.dpi_enabled(1), p.dpi_current()), (1600, true, false, 0));
    let p = from_config("a", &yaml("{dpi: {levels: [800, 1600, 3200], current: 1600}}"), None).unwrap().profile;
    assert_eq!(p.dpi_current(), 1);
}
