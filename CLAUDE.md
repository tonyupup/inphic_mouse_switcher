# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`inphic` is a self-made driver for the Inphic KP 8K / IN-series mouse, built from a reverse-engineered config protocol (Ghidra decompile of the official HUB + USB captures). It builds device commands directly and **switches mouse profiles (lighting, DPI, polling rate, buttons, macros) based on the foreground process**, without the official HUB or any keyboard/mouse hooks. Windows only (HID + WinRT BLE); implemented in Rust. Docs, comments and CLI output are in Chinese; keep new user-facing text in Chinese to match.

## Commands

The repo has two independent implementations sharing `config.yaml`, `macros/`, `docs/` and `fixtures/` (real USB captures) at the root:
- `rust/` — the current implementation (Cargo crate). Run from `rust/` on Windows; pass `-c ../config.yaml` or run the built exe from the repo root.
- `python/` — the original Python implementation (`uv`, `python -m inphic`), kept for reference; run from `python/` with `-c ../config.yaml`.

```powershell
cd rust
cargo run --release -- run -c ../config.yaml [-v]      # long-running foreground-process switcher; -v logs raw status bytes
cargo run --release -- show -c ../config.yaml [--payloads]              # print config summary in Chinese; --payloads = dry-run, prints bytes that would be sent (no device access)
cargo run --release -- status [--watch] [--via bluetooth]   # read-only: battery / status reports
cargo run --release -- macros list | show <group.name>      # inspect the macro library

cargo test                                            # all tests (no hardware needed, works on any OS)
cargo test --test captures                            # one test file
# python (from python/):  uv run python -m unittest discover -s tests -t .
cargo check --target x86_64-pc-windows-gnu --all-targets   # compile-check the Windows-only code from Linux/WSL
```

There is no linter/formatter configured. Hardware is abstracted behind traits (`HidBackend`/`HidDevice`, `BleSession`, `Channel`), so tests use fakes, and frame-level tests (`rust/tests/captures.rs`, `rust/tests/macros.rs`) compare **byte-for-byte against real USB captures** in `fixtures/`. HID (`hidapi`, windows-native backend) and BLE (`windows` crate WinRT) are compiled only on Windows (`cfg(windows)`).

## Architecture

Strict layering — lower layers must not import upper ones:

`rust/src/protocol/` (pure wire-format functions, no device/config access) → `rust/src/profile/` (in-memory profile model → command payloads → 64-byte frames) → `rust/src/config/` (yaml + macro library loading) → `rust/src/runtime/` (device I/O, foreground detection, switch loop) → `rust/src/cli/`.

Key flow: `config.yaml` → `config::loader` → `profile::build` (validates and builds profiles; anything unset = factory default from `profile::defaults`) → `profile::commands` turns a profile into payloads → `protocol::frame` wraps them as `[04, len+5, chunk_idx, data, sum16]` padded to 64 bytes (macros are 111-byte messages split into two chunk frames, see `protocol::macros`) → a channel writes them.

Two transport channels, selected by `device.connection` (`auto` | `dongle` | `bluetooth`) in `rust/src/runtime/switcher.rs` (`ProfileSwitcher`):
- `rust/src/runtime/channel.rs` — 2.4G dongle over HID. Windows splits interface 2 into several HID collections: Col04 (usage page 0xFF00) takes config commands (report ID 4), Col03 (usage page 0x000A) delivers device status reports (report ID 3). Opening the wrong collection silently yields no data.
- `rust/src/runtime/ble.rs` — Bluetooth via WinRT GATT service FEE0 (write on FEE3, notifications on FEE4). Uses the `windows` crate (WinRT) in a `cfg(windows)` module; blocking calls with timeouts, no async runtime. Writes wait for a per-command ack (`93 50 00 <cmd>`) and resend if missing. Bluetooth is checked first in channel priority since the dongle's HID device stays present even when the mouse is using BLE; only one link is active at a time. The official HUB/`Inphic.exe` must be closed or BLE access is denied; polling rate setting has no effect over BLE.

`ProfileSwitcher` applies the profile matching the foreground process on a background thread; failed writes are dropped and retried every 3 s, and unplug/replug re-applies the current profile. `rust/src/runtime/monitor.rs` reads status reports (battery logging, DPI-key changes).

Config errors are returned as `error::ValueError` (use the `bail!` macro) and surfaced by `cli::main` as `配置错误: ...` (exit code 2) rather than a traceback — new validation should follow that pattern. Profiles must reject bad config loudly, never silently ignore it.

## Configuration

`config.yaml` defines `profiles`, `rules` (first match wins, last must be `process: "*"`), device IDs, and `macro_files` globs (`macros/*.yaml`). Every option with a copy-paste example is documented in `docs/配置说明.md` — update it when changing config keys. Reverse-engineering notes (protocol fields, what is hardware-verified vs. only unit-tested) live in the sibling Gitea wiki checkout `../mouse_switcher.wiki/Home.md`; the README's "验证状态" section lists which features are verified on real hardware (BLE config writes to FEE3 and non-1000 polling rate / custom DPI are not).
