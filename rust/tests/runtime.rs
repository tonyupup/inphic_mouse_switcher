//! 运行时逻辑: 用假 HID / 假 GATT 会话验证重试、重连、应答重发, 不碰真实设备。

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use common::*;
use inphic::config::load_config;
use inphic::profile::from_config;
use inphic::protocol::bluetooth::{parse_address, report_from_notification};
use inphic::protocol::hex_decode;
use inphic::protocol::status::{battery_of, parse_report, Battery, StatusReport};
use inphic::runtime::ble::{BleChannel, BleOptions, BleSession, NotifyFn};
use inphic::runtime::channel::{Channel, HidBackend, HidChannel, HidDevice, HidInfo};
use inphic::runtime::monitor::BatteryTracker;
use inphic::runtime::switcher::{Parts, ProfileSwitcher};

// ---------- 假 HID ----------
#[derive(Default)]
struct FakeHid {
    present: Mutex<bool>,
    write_results: Mutex<Vec<bool>>, // false = 写失败, 依次弹出, 空了就成功
    opened: AtomicUsize,
    writes: AtomicUsize,
}

struct FakeDev(Arc<FakeHid>);

impl HidDevice for FakeDev {
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.0.writes.fetch_add(1, Ordering::SeqCst);
        let mut r = self.0.write_results.lock().unwrap();
        if !r.is_empty() && !r.remove(0) {
            bail!("fake error");
        }
        Ok(data.len())
    }

    fn read(&mut self, _: i32) -> Result<Vec<u8>> {
        Ok(vec![])
    }
}

struct Backend(Arc<FakeHid>);

impl HidBackend for Backend {
    fn enumerate(&self, _: u16, _: u16) -> Result<Vec<HidInfo>> {
        if !*self.0.present.lock().unwrap() {
            return Ok(vec![]);
        }
        Ok(vec![HidInfo { path: b"fake".to_vec(), usage_page: 0xFF00, usage: 1, interface_number: 2 }])
    }

    fn open(&self, _: &HidInfo) -> Result<Box<dyn HidDevice>> {
        self.0.opened.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeDev(self.0.clone())))
    }
}

fn fake_hid(present: bool) -> Arc<FakeHid> {
    let h = FakeHid::default();
    *h.present.lock().unwrap() = present;
    Arc::new(h)
}

fn test_config() -> Arc<inphic::config::AppConfig> {
    let mut cfg = load_config(root().join("config.yaml")).unwrap();
    cfg.packet_delay_ms = 0;
    cfg.device.bluetooth = None; // 真实配置里写了蓝牙地址; 测试不碰真实蓝牙
    Arc::new(cfg)
}

fn wait_idle(s: &ProfileSwitcher) {
    for _ in 0..500 {
        if !s.is_applying() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("apply 没有结束");
}

fn dongle_switcher(hid: &Arc<FakeHid>) -> ProfileSwitcher {
    let ch: Arc<dyn Channel> = Arc::new(HidChannel::new(Arc::new(Backend(hid.clone())), 0x1D57, 0xFA65, 0xFF00, 1));
    ProfileSwitcher::build(test_config(), Box::new(|| None), |_| {
        Ok(Parts { channels: vec![("dongle", ch)], monitor: None })
    })
    .unwrap()
}

#[test]
fn success_marks_applied_and_does_not_repeat() {
    let hid = fake_hid(true);
    let s = dongle_switcher(&hid);
    s.set_desired(Some("default"));
    s.maybe_apply();
    wait_idle(&s);
    assert_eq!(s.applied().as_deref(), Some("default"));
    let n = hid.writes.load(Ordering::SeqCst);
    assert_eq!(n, 6);
    s.maybe_apply();
    wait_idle(&s);
    assert_eq!(hid.writes.load(Ordering::SeqCst), n);
}

#[test]
fn failed_write_drops_handle_and_retries_later() {
    let hid = fake_hid(true);
    *hid.write_results.lock().unwrap() = vec![false];
    let s = dongle_switcher(&hid);
    s.set_desired(Some("cf"));
    s.maybe_apply();
    wait_idle(&s);
    assert_eq!(s.applied(), None); // 失败不能算已应用
    s.maybe_apply();
    wait_idle(&s);
    assert_eq!(hid.opened.load(Ordering::SeqCst), 1); // 还在退避期, 没有再试
    s.with_state(|st| st.retry_at = None);
    s.maybe_apply();
    wait_idle(&s);
    assert_eq!(s.applied().as_deref(), Some("cf"));
    assert_eq!(hid.opened.load(Ordering::SeqCst), 2); // 失效句柄被丢弃, 重新打开
}

#[test]
fn replug_triggers_reapply() {
    let hid = fake_hid(true);
    let s = dongle_switcher(&hid);
    s.set_desired(Some("default"));
    s.maybe_apply();
    wait_idle(&s);
    let n = hid.writes.load(Ordering::SeqCst);
    let t0 = Instant::now();
    *hid.present.lock().unwrap() = false;
    s.poll_presence(t0);
    assert!(!s.was_present());
    *hid.present.lock().unwrap() = true;
    s.poll_presence(t0 + Duration::from_secs(10));
    assert_eq!(s.applied(), None);
    s.maybe_apply();
    wait_idle(&s);
    assert!(hid.writes.load(Ordering::SeqCst) > n);
    assert_eq!(s.applied().as_deref(), Some("default"));
}

#[test]
fn profile_change_and_unknown_profile() {
    let hid = fake_hid(true);
    let s = dongle_switcher(&hid);
    s.set_desired(Some("default"));
    s.maybe_apply();
    wait_idle(&s);
    s.set_desired(Some("cf"));
    s.maybe_apply();
    wait_idle(&s);
    assert_eq!(s.applied().as_deref(), Some("cf"));
    s.set_desired(Some("nope"));
    s.maybe_apply();
    wait_idle(&s);
    assert_eq!(s.desired(), None);
}

// ---------- 蓝牙优先 / 切换 ----------
struct FakeBleChannel {
    present: Mutex<bool>,
    sent: Mutex<Vec<String>>,
    released: AtomicUsize,
}

impl Channel for FakeBleChannel {
    fn kind(&self) -> &'static str {
        "bluetooth"
    }
    fn label(&self) -> &'static str {
        "蓝牙"
    }
    fn supports_polling_rate(&self) -> bool {
        false
    }
    fn is_present(&self) -> bool {
        *self.present.lock().unwrap()
    }
    fn send_profile(&self, p: &inphic::profile::ConfiguredProfile, _: Option<Duration>) -> Result<()> {
        self.sent.lock().unwrap().push(p.name.clone());
        Ok(())
    }
    fn release(&self) {
        self.released.fetch_add(1, Ordering::SeqCst);
    }
    fn close(&self) {}
}

fn bt_switcher(hid: &Arc<FakeHid>) -> (ProfileSwitcher, Arc<FakeBleChannel>) {
    let ble = Arc::new(FakeBleChannel { present: Mutex::new(true), sent: Default::default(), released: Default::default() });
    let dongle: Arc<dyn Channel> = Arc::new(HidChannel::new(Arc::new(Backend(hid.clone())), 0x1D57, 0xFA65, 0xFF00, 1));
    let b = ble.clone();
    let s = ProfileSwitcher::build(test_config(), Box::new(|| None), move |_| {
        Ok(Parts { channels: vec![("bluetooth", b as Arc<dyn Channel>), ("dongle", dongle)], monitor: None })
    })
    .unwrap();
    (s, ble)
}

#[test]
fn bluetooth_used_when_dongle_absent() {
    let hid = fake_hid(false);
    let (s, ble) = bt_switcher(&hid);
    s.set_desired(Some("cf"));
    s.poll_presence(Instant::now());
    s.maybe_apply();
    wait_idle(&s);
    assert_eq!(s.kind(), Some("bluetooth"));
    assert_eq!(*ble.sent.lock().unwrap(), vec!["cf"]);
    assert_eq!(hid.writes.load(Ordering::SeqCst), 0);
}

#[test]
fn bluetooth_preferred_and_switching_reapplies() {
    let hid = fake_hid(true); // 接收器一直插着, 但鼠标此刻连着蓝牙 → 用蓝牙
    let (s, ble) = bt_switcher(&hid);
    s.set_desired(Some("default"));
    let t0 = Instant::now();
    s.poll_presence(t0);
    assert_eq!(s.kind(), Some("bluetooth"));
    s.maybe_apply();
    wait_idle(&s);
    assert_eq!(hid.writes.load(Ordering::SeqCst), 0);
    *ble.present.lock().unwrap() = false; // 鼠标切回 2.4G → 放掉蓝牙, 重新应用
    s.poll_presence(t0 + Duration::from_secs(10));
    assert_eq!(s.kind(), Some("dongle"));
    assert_eq!(ble.released.load(Ordering::SeqCst), 1);
    assert_eq!(s.applied(), None);
    s.maybe_apply();
    wait_idle(&s);
    assert!(hid.writes.load(Ordering::SeqCst) > 0);
    assert_eq!(s.applied().as_deref(), Some("default"));
}

#[test]
fn nothing_present_marks_disconnected() {
    let hid = fake_hid(false);
    let (s, ble) = bt_switcher(&hid);
    *ble.present.lock().unwrap() = false;
    s.poll_presence(Instant::now());
    assert!(!s.was_present());
}

// ---------- 假 GATT 会话 ----------
#[derive(Clone, Copy, PartialEq)]
enum Ack {
    Ok,
    None,
    Err,
}

struct FakeSession {
    ack: Ack,
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
    closed: Arc<Mutex<bool>>,
    connected: Arc<Mutex<bool>>,
    fail_on: Option<usize>,
    skip_ack_for_macro: bool,
    notify: NotifyFn,
}

impl BleSession for FakeSession {
    fn write(&mut self, payload: &[u8]) -> Result<()> {
        let mut w = self.writes.lock().unwrap();
        if self.fail_on == Some(w.len()) {
            bail!("write failed");
        }
        w.push(payload.to_vec());
        drop(w);
        if self.skip_ack_for_macro && payload[0] == 0x09 {
            return Ok(());
        }
        match self.ack {
            Ack::Ok => (self.notify)(vec![0x93, 0x50, 0x00, payload[0]]),
            Ack::Err => (self.notify)(vec![0x93, 0x50, 0x01, payload[0]]),
            Ack::None => {}
        }
        Ok(())
    }
    fn connected(&self) -> bool {
        *self.connected.lock().unwrap()
    }
    fn close(&mut self) {
        *self.closed.lock().unwrap() = true;
    }
}

struct Handles {
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
    closed: Arc<Mutex<bool>>,
    connected: Arc<Mutex<bool>>,
    notify: Arc<Mutex<Option<NotifyFn>>>,
    opened: Arc<AtomicUsize>,
}

fn ble_channel(acks: Vec<(Ack, Option<usize>, bool)>, mut opts: BleOptions) -> (BleChannel, Vec<Handles>) {
    let handles: Vec<Handles> = acks
        .iter()
        .map(|_| Handles {
            writes: Default::default(),
            closed: Default::default(),
            connected: Arc::new(Mutex::new(true)),
            notify: Default::default(),
            opened: Default::default(),
        })
        .collect();
    let plan: Arc<Mutex<Vec<(Ack, Option<usize>, bool, Arc<Mutex<Vec<Vec<u8>>>>, Arc<Mutex<bool>>, Arc<Mutex<bool>>, Arc<Mutex<Option<NotifyFn>>>)>>> =
        Arc::new(Mutex::new(
            acks.iter()
                .zip(&handles)
                .map(|(a, h)| (a.0, a.1, a.2, h.writes.clone(), h.closed.clone(), h.connected.clone(), h.notify.clone()))
                .collect(),
        ));
    opts.packet_delay = Duration::ZERO;
    opts.ack_timeout = Duration::from_millis(50);
    opts.open_session = Some(Box::new(move |_, notify| {
        let mut p = plan.lock().unwrap();
        let (ack, fail_on, skip, writes, closed, connected, slot) = p.remove(0);
        *slot.lock().unwrap() = Some(notify.clone());
        Ok(Box::new(FakeSession { ack, writes, closed, connected, fail_on, skip_ack_for_macro: skip, notify }))
    }));
    (BleChannel::new("78:87:11:75:9D:56", opts).unwrap(), handles)
}

fn profile() -> inphic::profile::ConfiguredProfile {
    from_config("t", &yaml("{polling_rate: 1000, light: {mode: spectrum_cycle}}"), None).unwrap()
}

#[test]
fn ble_sends_raw_payloads_without_polling() {
    let (ch, h) = ble_channel(vec![(Ack::Ok, None, false)], BleOptions::default());
    ch.send_profile(&profile(), None).unwrap();
    let sent = h[0].writes.lock().unwrap().clone();
    assert_eq!(sent.iter().map(|p| p[0]).collect::<Vec<_>>(), vec![0x0C, 0x04, 0x05, 0x07, 0x08]);
    assert_eq!(sent, profile().build_payloads(false).unwrap());
    assert!(sent.iter().all(|p| p.len() == p[1] as usize));
}

#[test]
fn ble_missing_ack_retries_then_fails() {
    let (ch, h) = ble_channel(vec![(Ack::None, None, false)], BleOptions::default());
    let err = ch.send_profile(&profile(), None).unwrap_err().to_string();
    assert!(err.contains("没有收到应答"), "{err}");
    let w = h[0].writes.lock().unwrap();
    assert_eq!(w.len(), 3); // 1 次 + 重发 2 次, 都是第一条命令 0x0c
    assert!(w.iter().all(|p| p[0] == 0x0C));
}

#[test]
fn ble_nonzero_ack_is_rejection() {
    let (ch, _) = ble_channel(vec![(Ack::Err, None, false)], BleOptions::default());
    assert!(ch.send_profile(&profile(), None).unwrap_err().to_string().contains("拒绝"));
}

#[test]
fn ble_macro_without_ack_is_only_a_warning() {
    let p = from_config(
        "m",
        &yaml("{buttons: {side_front: {type: macro, start: loop, steps: [{tap: a}]}}}"),
        None,
    )
    .unwrap();
    let (ch, h) = ble_channel(vec![(Ack::Ok, None, true)], BleOptions::default());
    ch.send_profile(&p, None).unwrap();
    let cmds: Vec<u8> = h[0].writes.lock().unwrap().iter().map(|w| w[0]).collect();
    assert_eq!(cmds, vec![0x0C, 0x04, 0x05, 0x07, 0x08, 0x09]);
}

#[test]
fn ble_failed_write_drops_session_and_reopens() {
    let (ch, h) = ble_channel(vec![(Ack::Ok, Some(2), false), (Ack::Ok, None, false)], BleOptions::default());
    assert!(ch.send_profile(&profile(), None).is_err());
    assert!(*h[0].closed.lock().unwrap());
    ch.send_profile(&profile(), None).unwrap();
    assert_eq!(h[1].writes.lock().unwrap().len(), 5);
}

#[test]
fn ble_notifications_reach_listener_and_raw() {
    let got: Arc<Mutex<Vec<StatusReport>>> = Default::default();
    let raws: Arc<Mutex<Vec<Vec<u8>>>> = Default::default();
    let (g, r) = (got.clone(), raws.clone());
    let opts = BleOptions {
        listener: Some(Arc::new(move |x| g.lock().unwrap().push(x))),
        on_raw: Some(Arc::new(move |x| r.lock().unwrap().push(x))),
        ..Default::default()
    };
    let (ch, h) = ble_channel(vec![(Ack::Ok, None, false)], opts);
    ch.connect().unwrap();
    let notify = h[0].notify.lock().unwrap().clone().unwrap();
    notify(hex_decode("93400142"));
    notify(vec![1, 2]);
    notify(hex_decode("93400142ff"));
    let got = got.lock().unwrap();
    assert_eq!(got[0], StatusReport::new(0x40, 1, 0x42));
    assert_eq!(got[1].raw, hex_decode("93400142ff")); // 多出来的字节也在 raw 里
    assert_eq!(*raws.lock().unwrap(), vec![vec![1u8, 2]]);
}

#[test]
fn ble_is_present_follows_session_and_release_notifies() {
    let gone = Arc::new(AtomicUsize::new(0));
    let g = gone.clone();
    let opts = BleOptions { on_disconnect: Some(Arc::new(move || { g.fetch_add(1, Ordering::SeqCst); })), ..Default::default() };
    let (ch, h) = ble_channel(vec![(Ack::Ok, None, false)], opts);
    ch.connect().unwrap();
    assert!(ch.is_present());
    *h[0].connected.lock().unwrap() = false;
    assert!(!ch.is_present());
    ch.release();
    assert!(*h[0].closed.lock().unwrap());
    assert_eq!(gone.load(Ordering::SeqCst), 1);
    let _ = &h[0].opened;
}

// ---------- 协议小件 ----------
#[test]
fn bluetooth_address_and_notifications() {
    assert_eq!(parse_address("78:87:11:75:9D:56").unwrap(), 0x788711759D56);
    assert_eq!(parse_address("788711759d56").unwrap(), 0x788711759D56);
    assert!(parse_address("78:87").is_err());
    assert!(parse_address("zz:87:11:75:9D:56").is_err());
    let r = report_from_notification(&hex_decode("93400142")).unwrap();
    assert_eq!(battery_of(Some(&r)), Some(Battery { percent: 0x42, state: 1 }));
    assert!(report_from_notification(&[1, 2]).is_none());
}

#[test]
fn status_report_parsing() {
    let r = parse_report(&hex_decode("03 93 40 01 45")).unwrap();
    assert_eq!(battery_of(Some(&r)).unwrap().percent, 69);
    assert!(parse_report(&hex_decode("04 06 c0 01 00 cb")).is_none());
    assert!(parse_report(&hex_decode("03 93 40 01")).is_none());
    assert!(battery_of(parse_report(&hex_decode("03 93 40 01 65")).as_ref()).is_none()); // 101%
    assert_eq!(parse_report(&hex_decode("03 93 10 02 01")).unwrap().value(), 0x0102);
}

// ---------- 电量日志 ----------
fn bat(p: u8, s: u8) -> Battery {
    Battery { percent: p, state: s }
}

#[test]
fn battery_tracker() {
    let mut t = BatteryTracker::new();
    assert_eq!(t.update(bat(80, 1)).len(), 1); // 第一个读数
    assert!(t.update(bat(80, 1)).is_empty()); // 不变不写
    assert_eq!(t.update(bat(79, 1))[0].1, "电量 80% → 79%");
    assert_eq!(t.update(bat(79, 3)).len(), 1); // 状态变化
    let out = t.update(bat(20, 3));
    assert!(out.iter().any(|(l, _)| *l == log::Level::Warn)); // 20% 警告一次
    assert_eq!(t.update(bat(19, 3)).iter().filter(|(l, _)| *l == log::Level::Warn).count(), 0);
    assert_eq!(t.update(bat(10, 3)).iter().filter(|(l, _)| *l == log::Level::Warn).count(), 1);
    t.update(bat(50, 3)); // 充电回升
    assert_eq!(t.update(bat(20, 3)).iter().filter(|(l, _)| *l == log::Level::Warn).count(), 1);
    t.reset();
    assert!(t.update(bat(20, 3)).iter().any(|(_, s)| s.starts_with("电量 20%")));
    let mut t = BatteryTracker::new();
    assert_eq!(t.update(bat(5, 1)).iter().filter(|(l, _)| *l == log::Level::Warn).count(), 2); // 一上来就低
}

// ---------- 配置热重载 ----------
#[test]
fn config_file_change_reloads_and_reapplies() {
    use std::sync::atomic::AtomicBool;
    let dir = std::env::temp_dir().join(format!("inphic-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("macros")).unwrap();
    for f in ["common.yaml", "fps.yaml"] {
        std::fs::copy(root().join("macros").join(f), dir.join("macros").join(f)).unwrap();
    }
    let text = std::fs::read_to_string(root().join("config.yaml")).unwrap().replace("packet_delay_ms: 520", "packet_delay_ms: 0");
    let path = dir.join("config.yaml");
    std::fs::write(&path, &text).unwrap();

    let hid = fake_hid(true);
    let ch: Arc<dyn Channel> = Arc::new(HidChannel::new(Arc::new(Backend(hid.clone())), 0x1D57, 0xFA65, 0xFF00, 1));
    let s = ProfileSwitcher::build(
        Arc::new(load_config(&path).unwrap()),
        Box::new(|| Some("crossfire.exe".to_string())),
        |_| Ok(Parts { channels: vec![("dongle", ch)], monitor: None }),
    )
    .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let t = {
        let (s, stop, path) = (s.clone(), stop.clone(), path.clone());
        std::thread::spawn(move || s.run_with_reload(&stop, Some(&path)))
    };
    let wait = |want: &str| {
        for _ in 0..300 {
            if s.applied().as_deref() == Some(want) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    };
    assert!(wait("cf"));
    // crossfire.exe 改成走 default 方案
    std::fs::write(&path, text.replace("profile: \"cf\"", "profile: \"default\"")).unwrap();
    assert!(wait("default"), "改配置后应自动重新加载并应用");
    // 写坏配置: 报错但继续用旧配置
    std::fs::write(&path, "rules: [").unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(s.applied().as_deref(), Some("default"));
    stop.store(true, Ordering::SeqCst);
    t.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
