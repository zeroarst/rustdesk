# Configurable Modifier Key Remapping — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user freely choose which peer modifier each of their four local modifiers (Ctrl, Win/Cmd, Alt/Option, Shift) sends, configured once globally per target OS.

**Architecture:** A new pure-function module, `src/modifier_remap.rs`, holds an immutable `ModifierMap` parsed from a single JSON local-config option. It is applied in *local* key space inside `event_to_key_events()` — before any peer scancode translation — plus two smaller sites for held-modifier state and mouse-event modifiers. The existing hardcoded `swap_modifier_key()` and its `allow_swap_key` option are deleted and migrated forward.

**Tech Stack:** Rust (`rdev` for key enums and scancode tables, `serde_json` for the config value, `lazy_static` for the cache), Flutter/Dart for the configuration dialog, `flutter_rust_bridge` for the existing `mainGetLocalOption` / `mainSetLocalOption` bindings.

**Spec:** `docs/superpowers/specs/2026-08-14-modifier-key-remapping-design.md`

## Global Constraints

- Config option key is exactly `modifier-remap`. Migration marker key is exactly `modifier-remap-migrated`.
- Canonical slot names, in storage and in code, are exactly `ctrl`, `meta`, `alt`, `shift`. Lowercase, no other spellings.
- OS keys in the JSON are the lowercase, whitespace-stripped platform names already defined at `src/keyboard.rs:27`: `windows`, `linux`, `macos`, `android`.
- Any parse failure, missing OS entry, missing slot, or unrecognised slot value MUST fall back to identity (no remap). Never error, never panic.
- `rdev::Key::AltGr` is never remapped and is never produced as a remap target.
- A `rdev::Key::ControlLeft` whose `position_code >> 8 == 0xE0` is the synthetic Control that Windows injects ahead of AltGr. It is never remapped.
- The remap must run **after** `update_modifiers_state()` and the `TO_RELEASE` bookkeeping in `event_to_key_events()`, so both stay in local key space.
- Builds and tests run on the Windows side via interop, never WSL-native. Env preamble (from `D:\dev\setup\build-full.ps1`) is required because WSL interop does not see freshly-set Windows env vars.
- Do not commit unless the user explicitly asks. Each task's "Commit" step means `git add` the listed files and then **stop and ask**.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `src/modifier_remap.rs` (new) | The `ModifierMap` value type, JSON parsing, the cache, and the one-shot migration. All pure except the config read/write. |
| `src/lib.rs` (modify) | Declare `mod modifier_remap;`. |
| `src/keyboard.rs` (modify) | Apply the map to the `rdev::Event` and to the held-modifier tuple. |
| `src/ui_session_interface.rs` (modify) | Delete `swap_modifier_key()`; convert `swap_modifier_mouse()` to use the map. |
| `src/client.rs` (modify) | Remove the `allow_swap_key` toggle handlers. |
| `src/common.rs` (modify) | Call the migration from `global_init()`. |
| `src/flutter_ffi.rs` (modify) | Invalidate the cache and release held keys when the option is written. |
| `src/ui/header.tis` (modify) | Remove the dead Sciter checkbox. |
| `flutter/lib/common/widgets/modifier_remap_dialog.dart` (new) | Storage helpers, display labels, and the configuration dialog. |
| `flutter/lib/common/widgets/toolbar.dart` (modify) | Remove the `allow_swap_key` checkbox. |
| `flutter/lib/desktop/widgets/remote_toolbar.dart` (modify) | Add the "Modifier keys…" menu entry. |
| `src/lang/template.rs` (modify) | Register the new UI strings for translators. |

---

## Reference: verified APIs

These were checked against the actual sources. Do not guess alternatives.

**`rdev` modifier key variants** (`rdev::Key`): `ControlLeft`, `ControlRight`, `MetaLeft`, `MetaRight`, `Alt`, `AltGr`, `ShiftLeft`, `ShiftRight`.

There is **no** `AltLeft`/`AltRight`. `Key::Alt` *is* the left Alt and `Key::AltGr` *is* the physical right Alt. This is why `Slot::Alt` has no right-hand target.

**`rdev` code conversions** (all return `Option<u32>`, all re-exported at crate root):
`win_scancode_from_key`, `win_code_from_key`, `linux_keycode_from_key`, `macos_keycode_from_key`, `usb_hid_keycode_from_key`.

**`rdev::Event` fields** (all `u32`): `platform_code`, `position_code`, `usb_hid`, plus `time`, `unicode`, `event_type`, and a platform-gated `extra_data`.

**Config:** `hbb_common::config::LocalConfig::get_option(&str) -> String` and `LocalConfig::set_option(String, String)`. `hbb_common::config::PeerConfig::peers(None) -> Vec<(String, SystemTime, PeerConfig)>`.

**`serde_json`** is re-exported as `hbb_common::serde_json`.

---

## Task 1: The `ModifierMap` engine

Pure value type plus its tests. No config reads, no callers wired up yet. This task is entirely self-contained and its tests are the safety net for everything after it.

**Files:**
- Create: `src/modifier_remap.rs`
- Modify: `src/lib.rs:1` (add the module declaration)
- Test: `src/modifier_remap.rs` (inline `#[cfg(test)] mod tests`, matching the convention in `src/common.rs` and `src/client.rs`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub const OPTION_MODIFIER_REMAP: &str = "modifier-remap";`
  - `pub const OPTION_MODIFIER_REMAP_MIGRATED: &str = "modifier-remap-migrated";`
  - `pub enum Slot { Ctrl, Meta, Alt, Shift }` — `Copy`, with `Slot::ALL: [Slot; 4]`, `Slot::from_name(&str) -> Option<Slot>`, `Slot::name(&self) -> &'static str`
  - `pub struct ModifierMap` — `Copy`, `Default` (= identity)
  - `ModifierMap::identity() -> ModifierMap`
  - `ModifierMap::is_identity(&self) -> bool`
  - `ModifierMap::set(&mut self, from: Slot, to: Slot)`
  - `ModifierMap::map_key(&self, key: rdev::Key, position_code: u32) -> rdev::Key`
  - `ModifierMap::map_control_key(&self, ck: ControlKey) -> ControlKey`
  - `ModifierMap::map_state(&self, alt: bool, ctrl: bool, shift: bool, command: bool) -> (bool, bool, bool, bool)`
  - `ModifierMap::to_json_value(&self) -> serde_json::Value`
  - `pub fn parse_all(raw: &str) -> HashMap<String, ModifierMap>`

- [x] **Step 1: Write the failing tests**

Create `src/modifier_remap.rs` containing **only** the test module for now, so the first run fails on missing items rather than on a syntax error:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rdev::Key;

    /// The mapping the whole feature exists for: a Windows keyboard driving a
    /// Mac, matching physical key position (Mac reads Ctrl · Option · Cmd).
    fn mac_positional() -> ModifierMap {
        let mut m = ModifierMap::identity();
        m.set(Slot::Ctrl, Slot::Ctrl);
        m.set(Slot::Meta, Slot::Alt);
        m.set(Slot::Alt, Slot::Meta);
        m
    }

    fn swap_ctrl_cmd() -> ModifierMap {
        let mut m = ModifierMap::identity();
        m.set(Slot::Ctrl, Slot::Meta);
        m.set(Slot::Meta, Slot::Ctrl);
        m
    }

    #[test]
    fn identity_changes_nothing() {
        let m = ModifierMap::identity();
        assert!(m.is_identity());
        assert_eq!(m.map_key(Key::ControlLeft, 0x1D), Key::ControlLeft);
        assert_eq!(m.map_key(Key::MetaLeft, 0xE05B), Key::MetaLeft);
        assert_eq!(m.map_control_key(ControlKey::Control), ControlKey::Control);
        assert_eq!(m.map_state(true, true, true, true), (true, true, true, true));
    }

    #[test]
    fn mac_positional_maps_keys() {
        let m = mac_positional();
        assert!(!m.is_identity());
        assert_eq!(m.map_key(Key::ControlLeft, 0x1D), Key::ControlLeft);
        assert_eq!(m.map_key(Key::MetaLeft, 0xE05B), Key::Alt);
        assert_eq!(m.map_key(Key::Alt, 0x38), Key::MetaLeft);
        assert_eq!(m.map_key(Key::ShiftLeft, 0x2A), Key::ShiftLeft);
    }

    #[test]
    fn side_is_preserved() {
        let m = swap_ctrl_cmd();
        assert_eq!(m.map_key(Key::ControlLeft, 0x1D), Key::MetaLeft);
        assert_eq!(m.map_key(Key::ControlRight, 0xE01D), Key::MetaRight);
        assert_eq!(m.map_key(Key::MetaLeft, 0xE05B), Key::ControlLeft);
        assert_eq!(m.map_key(Key::MetaRight, 0xE05C), Key::ControlRight);
    }

    /// rdev has no right-hand Alt (AltGr occupies that slot and must never be
    /// produced), so a right-hand source targeting Alt collapses to Key::Alt.
    #[test]
    fn right_side_targeting_alt_collapses_to_left_alt() {
        let mut m = ModifierMap::identity();
        m.set(Slot::Ctrl, Slot::Alt);
        assert_eq!(m.map_key(Key::ControlLeft, 0x1D), Key::Alt);
        assert_eq!(m.map_key(Key::ControlRight, 0xE01D), Key::Alt);
        assert_ne!(m.map_key(Key::ControlRight, 0xE01D), Key::AltGr);
    }

    #[test]
    fn altgr_is_never_remapped() {
        let mut m = mac_positional();
        m.set(Slot::Ctrl, Slot::Meta);
        // The physical right Alt.
        assert_eq!(m.map_key(Key::AltGr, 0xE038), Key::AltGr);
        // The synthetic ControlLeft Windows injects ahead of AltGr: extended
        // scancode prefix 0xE0 on a key rdev reports as ControlLeft.
        assert_eq!(m.map_key(Key::ControlLeft, 0xE01D), Key::ControlLeft);
        // A genuine left Control has no 0xE0 prefix and IS remapped.
        assert_eq!(m.map_key(Key::ControlLeft, 0x001D), Key::MetaLeft);
    }

    #[test]
    fn non_modifier_keys_pass_through() {
        let m = mac_positional();
        assert_eq!(m.map_key(Key::KeyA, 0x1E), Key::KeyA);
        assert_eq!(m.map_key(Key::Tab, 0x0F), Key::Tab);
    }

    #[test]
    fn control_keys_map_with_side() {
        let m = mac_positional();
        assert_eq!(m.map_control_key(ControlKey::Meta), ControlKey::Alt);
        assert_eq!(m.map_control_key(ControlKey::RWin), ControlKey::Alt);
        assert_eq!(m.map_control_key(ControlKey::Alt), ControlKey::Meta);
        assert_eq!(m.map_control_key(ControlKey::Control), ControlKey::Control);
        assert_eq!(m.map_control_key(ControlKey::Shift), ControlKey::Shift);
        // RAlt is AltGr.
        assert_eq!(m.map_control_key(ControlKey::RAlt), ControlKey::RAlt);
        // Anything that is not a modifier is untouched.
        assert_eq!(m.map_control_key(ControlKey::Return), ControlKey::Return);
    }

    #[test]
    fn control_key_side_preserved_on_swap() {
        let m = swap_ctrl_cmd();
        assert_eq!(m.map_control_key(ControlKey::Control), ControlKey::Meta);
        assert_eq!(m.map_control_key(ControlKey::RControl), ControlKey::RWin);
        assert_eq!(m.map_control_key(ControlKey::Meta), ControlKey::Control);
        assert_eq!(m.map_control_key(ControlKey::RWin), ControlKey::RControl);
    }

    /// Argument and return order is (alt, ctrl, shift, command), matching
    /// `keyboard::client::get_modifiers_state`.
    #[test]
    fn held_modifier_state_is_remapped() {
        let m = mac_positional();
        // Holding local Alt must report `command`, not `alt`.
        assert_eq!(m.map_state(true, false, false, false), (false, false, false, true));
        // Holding local Win/Meta must report `alt`.
        assert_eq!(m.map_state(false, false, false, true), (true, false, false, false));
        // Ctrl maps to itself.
        assert_eq!(m.map_state(false, true, false, false), (false, true, false, false));
        // Shift is untouched.
        assert_eq!(m.map_state(false, false, true, false), (false, false, true, false));
    }

    /// Two sources may legitimately share one target; the result is a union.
    #[test]
    fn held_state_unions_colliding_targets() {
        let mut m = ModifierMap::identity();
        m.set(Slot::Ctrl, Slot::Meta);
        m.set(Slot::Meta, Slot::Meta);
        assert_eq!(m.map_state(false, true, false, false), (false, false, false, true));
        assert_eq!(m.map_state(false, false, false, true), (false, false, false, true));
        assert_eq!(m.map_state(false, true, false, true), (false, false, false, true));
    }

    #[test]
    fn parses_a_full_table() {
        let raw = r#"{"macos":{"ctrl":"ctrl","meta":"alt","alt":"meta","shift":"shift"}}"#;
        let all = parse_all(raw);
        assert_eq!(all.len(), 1);
        assert_eq!(*all.get("macos").unwrap(), mac_positional());
    }

    #[test]
    fn parses_a_partial_table_as_identity_for_missing_slots() {
        let raw = r#"{"macos":{"meta":"alt","alt":"meta"}}"#;
        let all = parse_all(raw);
        assert_eq!(*all.get("macos").unwrap(), mac_positional());
    }

    #[test]
    fn bad_input_falls_back_to_identity() {
        for raw in [
            "",
            "not json",
            "[]",
            "null",
            r#"{"macos": "nonsense"}"#,
            r#"{"macos": {"ctrl": "banana"}}"#,
            r#"{"macos": {"banana": "ctrl"}}"#,
            r#"{"macos": {"ctrl": 7}}"#,
        ] {
            let all = parse_all(raw);
            let m = all.get("macos").copied().unwrap_or_default();
            assert!(m.is_identity(), "expected identity for input {raw:?}");
        }
    }

    #[test]
    fn os_keys_are_normalised_to_lowercase() {
        let raw = r#"{"MacOS":{"alt":"meta"}}"#;
        let all = parse_all(raw);
        assert!(all.contains_key("macos"));
    }

    #[test]
    fn json_round_trips() {
        let m = mac_positional();
        let v = m.to_json_value();
        let raw = format!(r#"{{"macos":{}}}"#, v);
        assert_eq!(*parse_all(&raw).get("macos").unwrap(), m);
    }
}
```

- [x] **Step 2: Run the tests to verify they fail**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:RUSTUP_HOME='D:\dev\rustup'; $env:CARGO_HOME='D:\dev\cargo'; $env:VCPKG_ROOT='D:\dev\vcpkg'; $env:LIBCLANG_PATH='D:\dev\LLVM\bin'; $env:PATH='D:\dev\cargo\bin;D:\dev\LLVM\bin;'+$env:PATH; cd D:\Projects\rustdesk; cargo test --lib --features hwcodec,vram,flutter modifier_remap -- --nocapture"
```

Expected: compile error — `cannot find type ModifierMap in this scope`, and similar for `Slot` and `parse_all`.

Note: the first `cargo test` link is the slow one — measured at **3m09s** on this machine, because it links a second binary against every dependency. Subsequent runs are ~30s. Use the debug profile shown above, **not** `--release` — the release profile sets `lto = true` and `codegen-units = 1`, which makes linking dramatically worse.

To confirm RED without paying even that, run `cargo check --lib --tests --features hwcodec,vram,flutter` instead: it reports the missing symbols in about 20s without linking. Use the full `cargo test` for GREEN.

- [x] **Step 3: Write the implementation**

Prepend to `src/modifier_remap.rs`, above the test module:

```rust
//! Configurable modifier key remapping.
//!
//! Rewrites the four modifier keys (Ctrl, Meta, Alt, Shift) in *local* key
//! space, before any peer scancode translation happens. See
//! `docs/superpowers/specs/2026-08-14-modifier-key-remapping-design.md`.

use hbb_common::message_proto::ControlKey;
use hbb_common::serde_json;
use rdev::Key;
use std::collections::HashMap;

pub const OPTION_MODIFIER_REMAP: &str = "modifier-remap";
pub const OPTION_MODIFIER_REMAP_MIGRATED: &str = "modifier-remap-migrated";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Ctrl,
    Meta,
    Alt,
    Shift,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Left,
    Right,
}

impl Slot {
    pub const ALL: [Slot; 4] = [Slot::Ctrl, Slot::Meta, Slot::Alt, Slot::Shift];

    pub fn from_name(s: &str) -> Option<Slot> {
        match s {
            "ctrl" => Some(Slot::Ctrl),
            "meta" => Some(Slot::Meta),
            "alt" => Some(Slot::Alt),
            "shift" => Some(Slot::Shift),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Slot::Ctrl => "ctrl",
            Slot::Meta => "meta",
            Slot::Alt => "alt",
            Slot::Shift => "shift",
        }
    }

    fn index(&self) -> usize {
        match self {
            Slot::Ctrl => 0,
            Slot::Meta => 1,
            Slot::Alt => 2,
            Slot::Shift => 3,
        }
    }
}

/// Which peer modifier each local modifier sends. Indexed by `Slot::index()`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ModifierMap {
    targets: [Slot; 4],
}

impl Default for ModifierMap {
    fn default() -> Self {
        Self::identity()
    }
}

/// rdev reports the physical right Alt as `Key::AltGr`, so it is absent here
/// on purpose: it must never be remapped.
fn split_key(key: Key) -> Option<(Slot, Side)> {
    match key {
        Key::ControlLeft => Some((Slot::Ctrl, Side::Left)),
        Key::ControlRight => Some((Slot::Ctrl, Side::Right)),
        Key::MetaLeft => Some((Slot::Meta, Side::Left)),
        Key::MetaRight => Some((Slot::Meta, Side::Right)),
        Key::Alt => Some((Slot::Alt, Side::Left)),
        Key::ShiftLeft => Some((Slot::Shift, Side::Left)),
        Key::ShiftRight => Some((Slot::Shift, Side::Right)),
        _ => None,
    }
}

/// `Key::AltGr` is never produced: there is no right-hand Alt target, so a
/// right-hand source aimed at Alt collapses onto the single `Key::Alt`.
fn join_key(slot: Slot, side: Side) -> Key {
    match (slot, side) {
        (Slot::Ctrl, Side::Left) => Key::ControlLeft,
        (Slot::Ctrl, Side::Right) => Key::ControlRight,
        (Slot::Meta, Side::Left) => Key::MetaLeft,
        (Slot::Meta, Side::Right) => Key::MetaRight,
        (Slot::Alt, _) => Key::Alt,
        (Slot::Shift, Side::Left) => Key::ShiftLeft,
        (Slot::Shift, Side::Right) => Key::ShiftRight,
    }
}

fn split_control_key(ck: ControlKey) -> Option<(Slot, Side)> {
    match ck {
        ControlKey::Control => Some((Slot::Ctrl, Side::Left)),
        ControlKey::RControl => Some((Slot::Ctrl, Side::Right)),
        ControlKey::Meta => Some((Slot::Meta, Side::Left)),
        ControlKey::RWin => Some((Slot::Meta, Side::Right)),
        ControlKey::Alt => Some((Slot::Alt, Side::Left)),
        ControlKey::Shift => Some((Slot::Shift, Side::Left)),
        ControlKey::RShift => Some((Slot::Shift, Side::Right)),
        // ControlKey::RAlt is AltGr.
        _ => None,
    }
}

fn join_control_key(slot: Slot, side: Side) -> ControlKey {
    match (slot, side) {
        (Slot::Ctrl, Side::Left) => ControlKey::Control,
        (Slot::Ctrl, Side::Right) => ControlKey::RControl,
        (Slot::Meta, Side::Left) => ControlKey::Meta,
        (Slot::Meta, Side::Right) => ControlKey::RWin,
        (Slot::Alt, _) => ControlKey::Alt,
        (Slot::Shift, Side::Left) => ControlKey::Shift,
        (Slot::Shift, Side::Right) => ControlKey::RShift,
    }
}

/// Windows injects a synthetic left Control ahead of AltGr, distinguishable
/// only by the extended-scancode prefix. Remapping it breaks accented
/// characters. The legacy path uses the same test at `src/keyboard.rs:1067`.
fn is_altgr_synthetic_ctrl(key: Key, position_code: u32) -> bool {
    key == Key::ControlLeft && (position_code >> 8) == 0xE0
}

impl ModifierMap {
    pub fn identity() -> Self {
        ModifierMap { targets: Slot::ALL }
    }

    pub fn is_identity(&self) -> bool {
        self.targets == Slot::ALL
    }

    pub fn set(&mut self, from: Slot, to: Slot) {
        self.targets[from.index()] = to;
    }

    fn target(&self, from: Slot) -> Slot {
        self.targets[from.index()]
    }

    /// `position_code` is the raw local scancode, needed only for the AltGr test.
    pub fn map_key(&self, key: Key, position_code: u32) -> Key {
        if self.is_identity() || is_altgr_synthetic_ctrl(key, position_code) {
            return key;
        }
        match split_key(key) {
            Some((slot, side)) => join_key(self.target(slot), side),
            None => key,
        }
    }

    pub fn map_control_key(&self, ck: ControlKey) -> ControlKey {
        if self.is_identity() {
            return ck;
        }
        match split_control_key(ck) {
            Some((slot, side)) => join_control_key(self.target(slot), side),
            None => ck,
        }
    }

    /// Argument and return order is (alt, ctrl, shift, command), matching
    /// `keyboard::client::get_modifiers_state`. Colliding targets union.
    pub fn map_state(
        &self,
        alt: bool,
        ctrl: bool,
        shift: bool,
        command: bool,
    ) -> (bool, bool, bool, bool) {
        if self.is_identity() {
            return (alt, ctrl, shift, command);
        }
        let mut held = [false; 4];
        held[Slot::Ctrl.index()] = ctrl;
        held[Slot::Meta.index()] = command;
        held[Slot::Alt.index()] = alt;
        held[Slot::Shift.index()] = shift;

        let mut out = [false; 4];
        for from in Slot::ALL {
            if held[from.index()] {
                out[self.target(from).index()] = true;
            }
        }
        (
            out[Slot::Alt.index()],
            out[Slot::Ctrl.index()],
            out[Slot::Shift.index()],
            out[Slot::Meta.index()],
        )
    }

    pub fn to_json_value(&self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        for from in Slot::ALL {
            obj.insert(
                from.name().to_string(),
                serde_json::Value::String(self.target(from).name().to_string()),
            );
        }
        serde_json::Value::Object(obj)
    }
}

/// Parses the whole `modifier-remap` option. Never fails: anything unparseable
/// simply yields fewer entries, and a missing entry means identity.
pub fn parse_all(raw: &str) -> HashMap<String, ModifierMap> {
    let mut out = HashMap::new();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return out;
    };
    let Some(root) = value.as_object() else {
        return out;
    };
    for (os, table) in root {
        let Some(table) = table.as_object() else {
            continue;
        };
        let mut map = ModifierMap::identity();
        for from in Slot::ALL {
            if let Some(to) = table
                .get(from.name())
                .and_then(|v| v.as_str())
                .and_then(Slot::from_name)
            {
                map.set(from, to);
            }
        }
        out.insert(os.to_lowercase(), map);
    }
    out
}
```

Then add the module declaration. In `src/lib.rs`, immediately after line 1 (`mod keyboard;`):

```rust
mod modifier_remap;
```

- [x] **Step 4: Run the tests to verify they pass**

Same command as Step 2. Expected: `test result: ok. 15 passed; 0 failed`.

If `ControlKey::KeyA` style variants are missing, check that `hbb_common::message_proto::ControlKey` is the right import — it is the same enum `src/keyboard.rs` gets from `use hbb_common::message_proto::*;`.

- [x] **Step 5: Stage and ask**

```bash
git add src/modifier_remap.rs src/lib.rs
```

Then stop and ask the user whether to commit. Suggested message: `feat: add ModifierMap engine for configurable modifier remapping`.

---

## Task 2: Config loading, cache, and invalidation

Wires the engine to the stored option with a cache, because this sits on the keystroke hot path and must not parse JSON per key.

**Files:**
- Modify: `src/modifier_remap.rs`
- Modify: `src/flutter_ffi.rs:1223` (`main_set_local_option`)
- Test: `src/modifier_remap.rs` (extend the existing test module)

**Interfaces:**
- Consumes: `parse_all()`, `ModifierMap`, `OPTION_MODIFIER_REMAP` from Task 1.
- Produces:
  - `pub fn for_peer(peer_platform_lower: &str) -> ModifierMap`
  - `pub fn invalidate_cache()`
  - `pub fn write_map_for(os_lower: &str, map: ModifierMap)`

- [x] **Step 1: Write the failing tests**

Append inside the `mod tests` block in `src/modifier_remap.rs`:

`LocalConfig` reads and writes the developer's **real** `%APPDATA%\RustDesk`
config, so these tests must restore whatever they found, and must not assert
that a real OS key is unset — the developer may legitimately have configured
one.

```rust
    /// `LocalConfig` reads and writes the real on-disk config, so any test that
    /// touches it must put back exactly what it found.
    fn with_saved_option<T>(f: impl FnOnce() -> T) -> T {
        let original = LocalConfig::get_option(OPTION_MODIFIER_REMAP);
        let result = f();
        LocalConfig::set_option(OPTION_MODIFIER_REMAP.to_string(), original);
        invalidate_cache();
        result
    }

    #[test]
    fn unknown_os_is_always_identity() {
        assert!(for_peer("definitely-not-an-os").is_identity());
        assert!(for_peer("").is_identity());
    }

    #[test]
    fn write_then_read_round_trips_through_the_cache() {
        with_saved_option(|| {
            LocalConfig::set_option(OPTION_MODIFIER_REMAP.to_string(), String::new());
            invalidate_cache();

            write_map_for("macos", mac_positional());
            assert_eq!(for_peer("macos"), mac_positional());
            // Other OSes are unaffected.
            assert!(for_peer("windows").is_identity());

            // Writing identity removes the entry rather than storing a no-op table.
            write_map_for("macos", ModifierMap::identity());
            assert!(for_peer("macos").is_identity());
            assert!(LocalConfig::get_option(OPTION_MODIFIER_REMAP).is_empty());
        });
    }
```

- [x] **Step 2: Run tests to verify they fail**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:RUSTUP_HOME='D:\dev\rustup'; $env:CARGO_HOME='D:\dev\cargo'; $env:VCPKG_ROOT='D:\dev\vcpkg'; $env:LIBCLANG_PATH='D:\dev\LLVM\bin'; $env:PATH='D:\dev\cargo\bin;D:\dev\LLVM\bin;'+$env:PATH; cd D:\Projects\rustdesk; cargo test --lib --features hwcodec,vram,flutter modifier_remap -- --nocapture --test-threads=1"
```

Expected: `cannot find function for_peer in this scope`.

Note `--test-threads=1`: the two cache tests share process-global config state and must not interleave.

- [x] **Step 3: Write the implementation**

Add to the imports at the top of `src/modifier_remap.rs`:

```rust
use hbb_common::config::LocalConfig;
use std::sync::RwLock;
```

Add after `parse_all()`:

```rust
lazy_static::lazy_static! {
    /// `None` means "not loaded yet". Parsed once, then reused for every
    /// keystroke; invalidated whenever the option is written.
    static ref CACHE: RwLock<Option<HashMap<String, ModifierMap>>> = RwLock::new(None);
}

/// `peer_platform_lower` must already be lowercased and whitespace-stripped,
/// which is what `keyboard::event_to_key_events` hands out ("Mac OS" -> "macos").
pub fn for_peer(peer_platform_lower: &str) -> ModifierMap {
    if let Some(maps) = CACHE.read().unwrap().as_ref() {
        return maps.get(peer_platform_lower).copied().unwrap_or_default();
    }
    let maps = parse_all(&LocalConfig::get_option(OPTION_MODIFIER_REMAP));
    let found = maps.get(peer_platform_lower).copied().unwrap_or_default();
    *CACHE.write().unwrap() = Some(maps);
    found
}

pub fn invalidate_cache() {
    *CACHE.write().unwrap() = None;
}

/// Merges one OS entry into the stored option. An identity map removes the
/// entry entirely, so an untouched config stays `{}` and costs nothing to parse.
pub fn write_map_for(os_lower: &str, map: ModifierMap) {
    let mut maps = parse_all(&LocalConfig::get_option(OPTION_MODIFIER_REMAP));
    if map.is_identity() {
        maps.remove(os_lower);
    } else {
        maps.insert(os_lower.to_string(), map);
    }
    let raw = if maps.is_empty() {
        String::new()
    } else {
        let mut root = serde_json::Map::new();
        for (os, m) in maps.iter() {
            root.insert(os.clone(), m.to_json_value());
        }
        serde_json::Value::Object(root).to_string()
    };
    LocalConfig::set_option(OPTION_MODIFIER_REMAP.to_string(), raw);
    invalidate_cache();
}
```

Now hook invalidation for writes that come from Dart. In `src/flutter_ffi.rs`, replace the opening of `main_set_local_option`:

```rust
pub fn main_set_local_option(key: String, value: String) {
    let is_texture_render_key = key.eq(config::keys::OPTION_TEXTURE_RENDER);
    let is_d3d_render_key = key.eq(config::keys::OPTION_ALLOW_D3D_RENDER);
```

with:

```rust
pub fn main_set_local_option(key: String, value: String) {
    let is_texture_render_key = key.eq(config::keys::OPTION_TEXTURE_RENDER);
    let is_d3d_render_key = key.eq(config::keys::OPTION_ALLOW_D3D_RENDER);
    let is_modifier_remap_key = key.eq(crate::modifier_remap::OPTION_MODIFIER_REMAP);
    if is_modifier_remap_key {
        // Release anything currently held *under the old mapping*, before the
        // new one takes effect. Otherwise a change mid-keypress would send the
        // press of one target and the release of another, stranding a modifier
        // down on the peer.
        crate::keyboard::release_remote_keys("map");
    }
```

and immediately after the existing `set_local_option(key, value.clone());` line, add:

```rust
    if is_modifier_remap_key {
        crate::modifier_remap::invalidate_cache();
    }
```

Finally, `src/lib.rs` currently declares the module privately. Make it visible to `flutter_ffi`, which is a sibling module, by leaving it as `mod modifier_remap;` — sibling access via `crate::modifier_remap::` already works. No change needed.

- [x] **Step 4: Run tests to verify they pass**

Same command as Step 2. Expected: `test result: ok. 17 passed; 0 failed`.

- [x] **Step 5: Verify the crate still builds**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:RUSTUP_HOME='D:\dev\rustup'; $env:CARGO_HOME='D:\dev\cargo'; $env:VCPKG_ROOT='D:\dev\vcpkg'; $env:LIBCLANG_PATH='D:\dev\LLVM\bin'; $env:PATH='D:\dev\cargo\bin;D:\dev\LLVM\bin;'+$env:PATH; cd D:\Projects\rustdesk; cargo check --features hwcodec,vram,flutter"
```

Expected: `Finished`, no errors. Roughly one minute.

- [x] **Step 6: Stage and ask**

```bash
git add src/modifier_remap.rs src/flutter_ffi.rs
```

Then stop and ask. Suggested message: `feat: load and cache the modifier remap config`.

---

## Task 3: Apply the map in the keyboard pipeline

The core hook. After this task the feature actually works end to end, driven by a hand-written config value.

**Files:**
- Modify: `src/keyboard.rs:348` (`get_modifiers_state`), `src/keyboard.rs:940` (`event_to_key_events`), `src/keyboard.rs:1018` (`legacy_keyboard_mode`), `src/keyboard.rs:1236` and `src/keyboard.rs:1286` (the two `get_modifiers_state` call sites)
- Test: `src/keyboard.rs` (new inline `#[cfg(test)] mod remap_tests`)

**Interfaces:**
- Consumes: `crate::modifier_remap::{for_peer, ModifierMap}` from Task 2.
- Produces: `fn remap_event(remap: &ModifierMap, event: &Event) -> Event` (private to `keyboard.rs`); `get_modifiers_state` gains a leading `remap: &ModifierMap` parameter; `legacy_keyboard_mode` gains a leading `peer: &str` parameter.

- [x] **Step 1: Write the failing test**

Append at the end of `src/keyboard.rs`:

```rust
#[cfg(test)]
mod remap_tests {
    use super::*;
    use crate::modifier_remap::{ModifierMap, Slot};

    fn mac_positional() -> ModifierMap {
        let mut m = ModifierMap::identity();
        m.set(Slot::Meta, Slot::Alt);
        m.set(Slot::Alt, Slot::Meta);
        m
    }

    fn key_event(key: Key, position_code: u32, platform_code: u32) -> Event {
        Event {
            time: std::time::SystemTime::now(),
            unicode: None,
            event_type: EventType::KeyPress(key),
            platform_code,
            position_code,
            usb_hid: 0,
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            extra_data: 0,
        }
    }

    fn pressed_key(event: &Event) -> Option<Key> {
        match event.event_type {
            EventType::KeyPress(k) => Some(k),
            EventType::KeyRelease(k) => Some(k),
            _ => None,
        }
    }

    #[test]
    fn identity_map_returns_the_event_untouched() {
        let e = key_event(Key::MetaLeft, 0xE05B, 0x5B);
        let out = remap_event(&ModifierMap::identity(), &e);
        assert_eq!(pressed_key(&out), Some(Key::MetaLeft));
        assert_eq!(out.position_code, e.position_code);
        assert_eq!(out.platform_code, e.platform_code);
    }

    #[test]
    fn remapped_event_carries_the_new_key_and_recomputed_codes() {
        let e = key_event(Key::MetaLeft, 0xE05B, 0x5B);
        let out = remap_event(&mac_positional(), &e);
        assert_eq!(pressed_key(&out), Some(Key::Alt));
        // Codes must be recomputed for the LOCAL platform, so downstream
        // per-peer translation tables see a coherent Alt event.
        #[cfg(target_os = "windows")]
        {
            assert_eq!(
                out.position_code,
                rdev::win_scancode_from_key(Key::Alt).unwrap()
            );
            assert_eq!(out.platform_code, rdev::win_code_from_key(Key::Alt).unwrap());
        }
        assert_ne!(out.position_code, e.position_code);
    }

    #[test]
    fn release_is_remapped_the_same_way_as_press() {
        let mut e = key_event(Key::Alt, 0x38, 0x12);
        e.event_type = EventType::KeyRelease(Key::Alt);
        let out = remap_event(&mac_positional(), &e);
        assert!(matches!(out.event_type, EventType::KeyRelease(Key::MetaLeft)));
    }

    #[test]
    fn altgr_survives_the_pipeline() {
        let mut m = mac_positional();
        m.set(Slot::Ctrl, Slot::Meta);
        // Physical right Alt.
        let e = key_event(Key::AltGr, 0xE038, 0xA5);
        assert_eq!(pressed_key(&remap_event(&m, &e)), Some(Key::AltGr));
        // The synthetic ControlLeft Windows injects ahead of it.
        let e = key_event(Key::ControlLeft, 0xE01D, 0x11);
        assert_eq!(pressed_key(&remap_event(&m, &e)), Some(Key::ControlLeft));
    }

    #[test]
    fn non_key_events_pass_through() {
        let mut e = key_event(Key::Alt, 0x38, 0x12);
        e.event_type = EventType::Wheel { delta_x: 0, delta_y: 1 };
        let out = remap_event(&mac_positional(), &e);
        assert!(matches!(out.event_type, EventType::Wheel { .. }));
    }
}
```

- [x] **Step 2: Run the test to verify it fails**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:RUSTUP_HOME='D:\dev\rustup'; $env:CARGO_HOME='D:\dev\cargo'; $env:VCPKG_ROOT='D:\dev\vcpkg'; $env:LIBCLANG_PATH='D:\dev\LLVM\bin'; $env:PATH='D:\dev\cargo\bin;D:\dev\LLVM\bin;'+$env:PATH; cd D:\Projects\rustdesk; cargo test --lib --features hwcodec,vram,flutter remap_tests -- --nocapture"
```

Expected: `cannot find function remap_event in this scope`.

- [x] **Step 3: Add `remap_event`**

Add to `src/keyboard.rs`, immediately above `pub fn event_to_key_events`:

```rust
/// Rewrites a modifier key in *local* key space, recomputing the platform
/// codes so the downstream per-peer translation tables see a coherent event.
fn remap_event(remap: &crate::modifier_remap::ModifierMap, event: &Event) -> Event {
    let (key, is_press) = match event.event_type {
        EventType::KeyPress(k) => (k, true),
        EventType::KeyRelease(k) => (k, false),
        _ => return event.clone(),
    };
    let mapped = remap.map_key(key, event.position_code as _);
    if mapped == key {
        return event.clone();
    }

    let mut e = event.clone();
    e.event_type = if is_press {
        EventType::KeyPress(mapped)
    } else {
        EventType::KeyRelease(mapped)
    };
    // Modifiers never carry text; a stale unicode payload would confuse
    // translate mode.
    e.unicode = None;
    e.usb_hid = rdev::usb_hid_keycode_from_key(mapped).unwrap_or(e.usb_hid);

    #[cfg(target_os = "windows")]
    {
        e.position_code = rdev::win_scancode_from_key(mapped).unwrap_or(e.position_code);
        e.platform_code = rdev::win_code_from_key(mapped).unwrap_or(e.platform_code);
    }
    #[cfg(target_os = "macos")]
    {
        e.platform_code = rdev::macos_keycode_from_key(mapped).unwrap_or(e.platform_code as _) as _;
        e.position_code = e.platform_code;
    }
    #[cfg(target_os = "linux")]
    {
        e.position_code = rdev::linux_keycode_from_key(mapped).unwrap_or(e.position_code);
        e.platform_code = e.position_code;
    }
    e
}
```

- [x] **Step 4: Run the test to verify it passes**

Same command as Step 2. Expected: `test result: ok. 5 passed`.

- [x] **Step 5: Call it from `event_to_key_events`**

In `src/keyboard.rs`, replace this block (currently lines 960–966):

```rust
    let mut key_event = KeyEvent::new();
    key_event.mode = keyboard_mode.into();

    let mut key_events = match keyboard_mode {
        KeyboardMode::Map => map_keyboard_mode(peer.as_str(), event, key_event),
        KeyboardMode::Translate => translate_keyboard_mode(peer.as_str(), event, key_event),
```

with:

```rust
    // The remap runs *after* `update_modifiers_state` and the `TO_RELEASE`
    // bookkeeping above, so both keep tracking real local keys. Releases
    // replayed out of `TO_RELEASE` come back through here and are remapped
    // identically, so press/release stay paired.
    let remap = crate::modifier_remap::for_peer(peer.as_str());
    let remapped;
    let event = if remap.is_identity() {
        event
    } else {
        remapped = remap_event(&remap, event);
        &remapped
    };

    let mut key_event = KeyEvent::new();
    key_event.mode = keyboard_mode.into();

    let mut key_events = match keyboard_mode {
        KeyboardMode::Map => map_keyboard_mode(peer.as_str(), event, key_event),
        KeyboardMode::Translate => translate_keyboard_mode(peer.as_str(), event, key_event),
```

In the same `match`, the legacy arm must now pass `peer` through. Replace:

```rust
                legacy_keyboard_mode(event, key_event)
```

with:

```rust
                legacy_keyboard_mode(peer.as_str(), event, key_event)
```

- [x] **Step 6: Thread `peer` and the map into the legacy path**

In `src/keyboard.rs:1018`, change the signature:

```rust
pub fn legacy_keyboard_mode(event: &Event, mut key_event: KeyEvent) -> Vec<KeyEvent> {
```

to:

```rust
pub fn legacy_keyboard_mode(peer: &str, event: &Event, mut key_event: KeyEvent) -> Vec<KeyEvent> {
```

Inside it, replace:

```rust
    let peer = get_peer_platform();
    let is_win = peer == "Windows";
```

with:

```rust
    // `peer` is already lowercased and whitespace-stripped by the caller.
    // Previously this re-derived it via `get_peer_platform()`, which returns
    // the *current* session and is wrong when several session windows are open.
    let is_win = peer == OS_LOWER_WINDOWS;
```

Then change `get_modifiers_state` at `src/keyboard.rs:348` to take the map:

```rust
    pub fn get_modifiers_state(
        remap: &crate::modifier_remap::ModifierMap,
        alt: bool,
        ctrl: bool,
        shift: bool,
        command: bool,
    ) -> (bool, bool, bool, bool) {
```

and replace its final line:

```rust
        (alt, ctrl, shift, command)
```

with:

```rust
        remap.map_state(alt, ctrl, shift, command)
```

Update the two call sites. At `src/keyboard.rs:1236` (inside `legacy_keyboard_mode`):

```rust
    let (alt, ctrl, shift, command) = client::get_modifiers_state(alt, ctrl, shift, command);
```

becomes:

```rust
    let remap = crate::modifier_remap::for_peer(peer);
    let (alt, ctrl, shift, command) = client::get_modifiers_state(&remap, alt, ctrl, shift, command);
```

At `src/keyboard.rs:1286` (inside `windows_peer_special_key`, whose `peer` is already in scope):

```rust
    let (alt, ctrl, shift, command) = client::get_modifiers_state(false, false, false, false);
```

becomes:

```rust
    let remap = crate::modifier_remap::for_peer(peer);
    let (alt, ctrl, shift, command) =
        client::get_modifiers_state(&remap, false, false, false, false);
```

Note: `legacy_keyboard_mode` reads the modifier booleans from `get_key_state()` on the physical keyboard, which is why the map is applied to the tuple rather than derived from the already-remapped event.

**Third call site, found during implementation.** `get_modifiers_state` is also
called on the *mouse* path at `src/ui_session_interface.rs:1249`, feeding
`send_mouse()`, which builds `MouseEvent.modifiers` and then hands them to
`swap_modifier_mouse()`. Remapping in both places would remap mouse modifiers
**twice**. The remap is therefore applied here, on the held-modifier state, and
`swap_modifier_mouse` is **deleted** in Task 4 rather than converted:

```rust
        // The modifier remap is applied here, to the held-modifier state, and
        // NOT to the resulting `MouseEvent.modifiers` further down in
        // `send_mouse`. Doing both would remap twice.
        let mut peer = self.peer_platform().to_lowercase();
        peer.retain(|c| !c.is_whitespace());
        let remap = crate::modifier_remap::for_peer(&peer);
        let (alt, ctrl, shift, command) =
            keyboard::client::get_modifiers_state(&remap, alt, ctrl, shift, command);
```

Out of scope, noted for the record: `send_pointer_device_event` (touch/pan/scale
from mobile) builds its modifiers from booleans passed straight in from Dart and
never went through `swap_modifier_mouse`. Touch gestures with modifiers were
never remapped by the old feature and are not remapped by this one either.

- [x] **Step 7: Verify the whole crate builds and all tests pass**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:RUSTUP_HOME='D:\dev\rustup'; $env:CARGO_HOME='D:\dev\cargo'; $env:VCPKG_ROOT='D:\dev\vcpkg'; $env:LIBCLANG_PATH='D:\dev\LLVM\bin'; $env:PATH='D:\dev\cargo\bin;D:\dev\LLVM\bin;'+$env:PATH; cd D:\Projects\rustdesk; cargo check --features hwcodec,vram,flutter; cargo test --lib --features hwcodec,vram,flutter -- --test-threads=1"
```

Expected: `cargo check` finishes clean; all `modifier_remap` and `remap_tests` tests pass.

If `cargo check` reports another caller of `legacy_keyboard_mode` or `get_modifiers_state`, update it the same way — pass the lowercased peer string through and build the map from it.

- [x] **Step 8: Stage and ask**

```bash
git add src/keyboard.rs
```

Then stop and ask. Suggested message: `feat: apply the modifier remap in local key space`.

---

## Task 4: Retire `allow_swap_key`

Removes the old mechanism now that the new one covers it, including the mouse-modifier path.

**Files:**
- Modify: `src/ui_session_interface.rs:700` (delete `swap_modifier_key`), `src/ui_session_interface.rs:779` (its call site), `src/ui_session_interface.rs:1938` (`swap_modifier_mouse`)
- Modify: `src/client.rs:2183`, `src/client.rs:2394`
- Modify: `flutter/lib/common/widgets/toolbar.dart:1214-1229`
- Modify: `src/ui/header.tis:222`, `src/ui/header.tis:492`

**Interfaces:**
- Consumes: `crate::modifier_remap::for_peer` from Task 2.
- Produces: nothing new. `Session::send_key_event` becomes a plain send.

- [x] **Step 1: Delete `swap_modifier_key` and its call**

In `src/ui_session_interface.rs`, delete the whole `pub fn swap_modifier_key(&self, msg: &mut KeyEvent) { ... }` function (lines 700–773) and replace `send_key_event`:

```rust
    pub fn send_key_event(&self, evt: &KeyEvent) {
        // mode: legacy(0), map(1), translate(2), auto(3)

        let mut msg = evt.clone();
        self.swap_modifier_key(&mut msg);
        let mut msg_out = Message::new();
        msg_out.set_key_event(msg);
        self.send(Data::Message(msg_out));
    }
```

with:

```rust
    pub fn send_key_event(&self, evt: &KeyEvent) {
        // mode: legacy(0), map(1), translate(2), auto(3)
        //
        // Modifier remapping happens upstream in `keyboard::event_to_key_events`,
        // in local key space. Events built directly here (the on-screen modifier
        // buttons on mobile, `ctrl_alt_del`, `lock_screen`) are explicit semantic
        // requests and are deliberately NOT remapped.
        let mut msg_out = Message::new();
        msg_out.set_key_event(evt.clone());
        self.send(Data::Message(msg_out));
    }
```

- [x] **Step 2: Delete `swap_modifier_mouse`**

**Changed from the original plan.** Task 3 established that the mouse path's
modifiers are already remapped upstream, at the `get_modifiers_state` call in
`Session::send_mouse`. Converting this function as well would remap them a
second time, so it is deleted outright instead, along with its `Interface`
trait declaration and its single call site at `src/client.rs:3232`.

Delete the whole function at `src/ui_session_interface.rs:1938`:

```rust
    fn swap_modifier_mouse(&self, msg: &mut hbb_common::protos::message::MouseEvent) {
        let allow_swap_key = self.get_toggle_option("allow_swap_key".to_string());
        if allow_swap_key {
            msg.modifiers = msg
                .modifiers
                .iter()
                .map(|ck| {
                    let ck = ck.enum_value_or_default();
                    let ck = match ck {
                        ControlKey::Control => ControlKey::Meta,
                        ControlKey::Meta => ControlKey::Control,
                        ControlKey::RControl => ControlKey::Meta,
                        ControlKey::RWin => ControlKey::Control,
                        _ => ck,
                    };
                    hbb_common::protobuf::EnumOrUnknown::new(ck)
                })
                .collect();
        };
    }
```

Then remove its declaration from the `Interface` trait and delete the call at
`src/client.rs:3232`:

```rust
    interface.swap_modifier_mouse(&mut mouse_event);
```

`ModifierMap::map_control_key` stays in the engine and keeps its tests — it is
the correct primitive if a future caller ever needs to remap a `ControlKey`
list directly — but nothing calls it after this task.

- [x] **Step 3: Remove the toggle handlers**

In `src/client.rs`, delete these two arms:

```rust
        } else if name == "allow_swap_key" {
            config.allow_swap_key.v = !config.allow_swap_key.v;
```

and

```rust
        } else if name == "allow_swap_key" {
            self.config.allow_swap_key.v
```

taking care to keep the surrounding `if / else if` chain syntactically valid — each deletion removes one `} else if ... {` header plus its single body line, so the next `} else if` becomes the continuation.

Leave the `allow_swap_key` field itself in `libs/hbb_common/src/config.rs:318`. Task 5's migration reads it, and removing it would break deserialisation of existing config files.

- [x] **Step 4: Remove the Flutter checkbox**

In `flutter/lib/common/widgets/toolbar.dart`, delete the whole block starting at the `// swap key` comment through the closing brace of its `if`, i.e. from:

```dart
  // swap key
  if (ffiModel.keyboard &&
      ((isMacOS && pi.platform != kPeerPlatformMacOS) ||
          (!isMacOS && pi.platform == kPeerPlatformMacOS))) {
```

down to and including:

```dart
        child: Text(translate('Swap control-command key'))));
  }
```

**Corrected during implementation:** the local `final pi = ffiModel.pi;` *does*
become dead — the code further down reaches through `ffi.ffiModel.pi.` directly
rather than using the local. Delete that line too, or `flutter analyze` reports
`unused_local_variable`. `ffiModel`, `sessionId` and `isDefaultConn` all remain
in use and must stay.

- [x] **Step 5: Remove the Sciter checkbox**

In `src/ui/header.tis:222`, delete the `<li #allow_swap_key ...>` list item entirely. In `src/ui/header.tis:492`, remove the `"allow_swap_key"` string from the array literal.

- [x] **Step 6: Verify it builds**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:RUSTUP_HOME='D:\dev\rustup'; $env:CARGO_HOME='D:\dev\cargo'; $env:VCPKG_ROOT='D:\dev\vcpkg'; $env:LIBCLANG_PATH='D:\dev\LLVM\bin'; $env:PATH='D:\dev\cargo\bin;D:\dev\LLVM\bin;'+$env:PATH; cd D:\Projects\rustdesk; cargo check --features hwcodec,vram,flutter"
```

Then check the Dart half:

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:PUB_CACHE='D:\dev\pub-cache'; $env:PATH='D:\dev\flutter\bin;'+$env:PATH; cd D:\Projects\rustdesk\flutter; flutter analyze lib/common/widgets/toolbar.dart"
```

Expected: no errors from either. Warnings about pre-existing issues elsewhere are acceptable; anything naming `allow_swap_key` or `swap_modifier_key` is not.

- [x] **Step 7: Stage and ask**

```bash
git add src/ui_session_interface.rs src/client.rs src/ui/header.tis flutter/lib/common/widgets/toolbar.dart
```

Then stop and ask. Suggested message: `refactor: retire the hardcoded allow_swap_key option`.

---

## Task 5: One-shot migration

Carries existing users' `allow_swap_key` setting into the new table so the upgrade is invisible to them.

**Files:**
- Modify: `src/modifier_remap.rs`
- Modify: `src/common.rs:124` (`global_init`)
- Test: `src/modifier_remap.rs`

**Interfaces:**
- Consumes: `write_map_for`, `Slot`, `ModifierMap`, `OPTION_MODIFIER_REMAP_MIGRATED` from Tasks 1–2.
- Produces: `pub fn migrate_from_allow_swap_key()`, and `pub fn swap_ctrl_cmd_map() -> ModifierMap` (also used by the preset in the UI's Rust-side tests).

- [x] **Step 1: Write the failing test**

**Discovered during implementation — read this before writing the tests.**
`LocalConfig` reads and writes the real `%APPDATA%\RustDesk\config\RustDesk_local.toml`,
which a *running RustDesk client also writes*. Any test that round-trips
through `LocalConfig` therefore races the live client and fails intermittently:
one such failure was observed (`121 passed; 1 failed`) followed by four clean
runs, and `RustDesk_local.toml` was confirmed to have been rewritten by
PID 59188 mid-run.

The logic is therefore split so the tests can be pure:

- `merge_into(raw, os, map) -> String` — the merge/serialise step, no config.
- `seeded_raw(raw) -> String` — what seeding produces, no config.
- `should_seed(existing_raw, any_peer_swap) -> bool` — the migration decision.
- `seed_targets() -> &'static [&'static str]` — which OS tables apply.

`write_map_for` and `migrate_from_allow_swap_key` become thin wrappers that add
only the `LocalConfig` read/write. The two genuine round-trip tests are kept but
marked `#[ignore]`, so they never run in the default suite:

```rust
    #[ignore = "reads/writes the real RustDesk config; races a running client"]
```

Run them deliberately, with every RustDesk instance closed:

```bash
cargo test --lib --features hwcodec,vram,flutter modifier_remap -- --ignored --test-threads=1
```

Append inside `mod tests` in `src/modifier_remap.rs` (see the committed file for
the final versions):

```rust
    #[test]
    fn swap_preset_matches_the_retired_checkbox() {
        let m = swap_ctrl_cmd_map();
        assert_eq!(m.map_key(Key::ControlLeft, 0x1D), Key::MetaLeft);
        assert_eq!(m.map_key(Key::MetaLeft, 0xE05B), Key::ControlLeft);
        assert_eq!(m.map_key(Key::Alt, 0x38), Key::Alt);
        assert_eq!(m.map_control_key(ControlKey::RControl), ControlKey::RWin);
    }

    #[test]
    fn migration_is_one_shot() {
        // Start from a clean slate.
        LocalConfig::set_option(OPTION_MODIFIER_REMAP.to_string(), String::new());
        LocalConfig::set_option(OPTION_MODIFIER_REMAP_MIGRATED.to_string(), String::new());
        invalidate_cache();

        migrate_from_allow_swap_key();
        assert_eq!(LocalConfig::get_option(OPTION_MODIFIER_REMAP_MIGRATED), "Y");

        // A second run must not touch a table the user has since edited.
        write_map_for("macos", mac_positional());
        migrate_from_allow_swap_key();
        assert_eq!(for_peer("macos"), mac_positional());
    }
```

- [x] **Step 2: Run the test to verify it fails**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:RUSTUP_HOME='D:\dev\rustup'; $env:CARGO_HOME='D:\dev\cargo'; $env:VCPKG_ROOT='D:\dev\vcpkg'; $env:LIBCLANG_PATH='D:\dev\LLVM\bin'; $env:PATH='D:\dev\cargo\bin;D:\dev\LLVM\bin;'+$env:PATH; cd D:\Projects\rustdesk; cargo test --lib --features hwcodec,vram,flutter modifier_remap -- --test-threads=1"
```

Expected: `cannot find function migrate_from_allow_swap_key in this scope`.

- [x] **Step 3: Write the implementation**

Add to `src/modifier_remap.rs`:

```rust
use hbb_common::config::PeerConfig;

/// The mapping the retired "Swap control-command key" checkbox produced.
pub fn swap_ctrl_cmd_map() -> ModifierMap {
    let mut m = ModifierMap::identity();
    m.set(Slot::Ctrl, Slot::Meta);
    m.set(Slot::Meta, Slot::Ctrl);
    m
}

/// Carries the retired per-peer `allow_swap_key` flag into the new global
/// table. Runs at most once, guarded by its own marker option.
///
/// The old checkbox was only ever offered when exactly one side of the
/// connection was macOS, so the OS it applied to is fully determined by which
/// side the local machine is on.
pub fn migrate_from_allow_swap_key() {
    if !LocalConfig::get_option(OPTION_MODIFIER_REMAP_MIGRATED).is_empty() {
        return;
    }
    LocalConfig::set_option(OPTION_MODIFIER_REMAP_MIGRATED.to_string(), "Y".to_string());

    if !LocalConfig::get_option(OPTION_MODIFIER_REMAP).is_empty() {
        return;
    }
    let any_swap = PeerConfig::peers(None)
        .into_iter()
        .any(|(_, _, c)| c.allow_swap_key.v);
    if !any_swap {
        return;
    }

    let swap = swap_ctrl_cmd_map();
    if cfg!(target_os = "macos") {
        write_map_for(OS_LOWER_WINDOWS, swap);
        write_map_for(OS_LOWER_LINUX, swap);
    } else {
        write_map_for(OS_LOWER_MACOS, swap);
    }
    hbb_common::log::info!("migrated allow_swap_key into {}", OPTION_MODIFIER_REMAP);
}
```

`src/keyboard.rs`'s `OS_LOWER_*` constants are private to that module, so declare local copies at the top of `src/modifier_remap.rs`:

```rust
const OS_LOWER_WINDOWS: &str = "windows";
const OS_LOWER_LINUX: &str = "linux";
const OS_LOWER_MACOS: &str = "macos";
```

Then call it once at startup. In `src/common.rs`, change `global_init`:

```rust
pub fn global_init() -> bool {
    #[cfg(target_os = "linux")]
    {
        if !crate::platform::linux::is_x11() {
            crate::server::wayland::init();
        }
    }
    true
}
```

to:

```rust
pub fn global_init() -> bool {
    #[cfg(target_os = "linux")]
    {
        if !crate::platform::linux::is_x11() {
            crate::server::wayland::init();
        }
    }
    crate::modifier_remap::migrate_from_allow_swap_key();
    true
}
```

- [x] **Step 4: Run the tests to verify they pass**

Same command as Step 2. Expected: all `modifier_remap` tests pass.

- [x] **Step 5: Stage and ask**

```bash
git add src/modifier_remap.rs src/common.rs
```

Then stop and ask. Suggested message: `feat: migrate allow_swap_key into the modifier remap table`.

---

## Task 6: The configuration dialog

The user-facing half.

**Files:**
- Create: `flutter/lib/common/widgets/modifier_remap_dialog.dart`
- Modify: `flutter/lib/desktop/widgets/remote_toolbar.dart:2410` (menu children) and `:2506` (beside `localKeyboardType`)
- Modify: `src/lang/template.rs`

**Interfaces:**
- Consumes: `bind.mainGetLocalOption` / `bind.mainSetLocalOption`, the `modifier-remap` option format from Task 1.
- Produces: `showModifierRemapDialog(String peerPlatform, OverlayDialogManager dialogManager)`.

- [x] **Step 1: Create the dialog file**

```dart
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/consts.dart';
import 'package:flutter_hbb/models/platform_model.dart';

const String kOptionModifierRemap = 'modifier-remap';

/// Canonical slot names. Must match `Slot::name()` in src/modifier_remap.rs.
const List<String> kModifierSlots = ['ctrl', 'meta', 'alt', 'shift'];

/// "Mac OS" -> "macos". Must match the OS keys the Rust side looks up.
String peerOsKey(String peerPlatform) =>
    peerPlatform.replaceAll(RegExp(r'\s+'), '').toLowerCase();

String _labelFor(String slot, String platform) {
  switch (slot) {
    case 'ctrl':
      return platform == kPeerPlatformMacOS ? 'Control' : 'Ctrl';
    case 'meta':
      if (platform == kPeerPlatformMacOS) return 'Command';
      if (platform == kPeerPlatformWindows) return 'Win';
      return 'Super';
    case 'alt':
      return platform == kPeerPlatformMacOS ? 'Option' : 'Alt';
    case 'shift':
      return 'Shift';
  }
  return slot;
}

String _localPlatformName() {
  if (isMacOS) return kPeerPlatformMacOS;
  if (isWindows) return kPeerPlatformWindows;
  return kPeerPlatformLinux;
}

Map<String, String> loadModifierRemap(String osKey) {
  final raw = bind.mainGetLocalOption(key: kOptionModifierRemap);
  if (raw.isEmpty) return {};
  try {
    final root = jsonDecode(raw);
    if (root is! Map) return {};
    final table = root[osKey];
    if (table is! Map) return {};
    final out = <String, String>{};
    table.forEach((k, v) {
      if (v is String && kModifierSlots.contains(v)) {
        out[k.toString()] = v;
      }
    });
    return out;
  } catch (_) {
    return {};
  }
}

Future<void> saveModifierRemap(String osKey, Map<String, String> table) async {
  final raw = bind.mainGetLocalOption(key: kOptionModifierRemap);
  Map<String, dynamic> root = {};
  if (raw.isNotEmpty) {
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map) root = Map<String, dynamic>.from(decoded);
    } catch (_) {}
  }
  final isIdentity = kModifierSlots.every((s) => (table[s] ?? s) == s);
  if (isIdentity) {
    root.remove(osKey);
  } else {
    root[osKey] = {for (final s in kModifierSlots) s: table[s] ?? s};
  }
  await bind.mainSetLocalOption(
      key: kOptionModifierRemap, value: root.isEmpty ? '' : jsonEncode(root));
}

void showModifierRemapDialog(
    String peerPlatform, OverlayDialogManager dialogManager) {
  final osKey = peerOsKey(peerPlatform);
  final saved = loadModifierRemap(osKey);
  final table = <String, String>{
    for (final s in kModifierSlots) s: saved[s] ?? s
  };
  final localPlatform = _localPlatformName();
  final isMacTarget = peerPlatform == kPeerPlatformMacOS;

  dialogManager.show((setState, close, context) {
    applyPreset(Map<String, String> preset) => setState(() {
          for (final s in kModifierSlots) {
            table[s] = preset[s] ?? s;
          }
        });

    return CustomAlertDialog(
      title: Text(translate('Modifier keys')),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('${translate('When controlling')}: $peerPlatform')
              .marginOnly(bottom: 12),
          ...kModifierSlots.map((from) => Row(
                children: [
                  SizedBox(
                      width: 96, child: Text(_labelFor(from, localPlatform))),
                  const Icon(Icons.arrow_forward, size: 16)
                      .marginSymmetric(horizontal: 8),
                  Expanded(
                    child: DropdownButton<String>(
                      isExpanded: true,
                      value: table[from],
                      onChanged: (v) {
                        if (v != null) setState(() => table[from] = v);
                      },
                      items: kModifierSlots
                          .map((to) => DropdownMenuItem(
                              value: to,
                              child: Text(_labelFor(to, peerPlatform))))
                          .toList(),
                    ),
                  ),
                ],
              ).marginOnly(bottom: 4)),
          const Divider(),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              dialogButton(translate('No remap'),
                  isOutline: true, onPressed: () => applyPreset({})),
              dialogButton(translate('Swap Ctrl/Cmd'),
                  isOutline: true,
                  onPressed: () =>
                      applyPreset({'ctrl': 'meta', 'meta': 'ctrl'})),
              if (isMacTarget)
                dialogButton(translate('Mac positional'),
                    isOutline: true,
                    onPressed: () => applyPreset(
                        {'ctrl': 'ctrl', 'meta': 'alt', 'alt': 'meta'})),
            ],
          ),
        ],
      ),
      actions: [
        dialogButton('Cancel', onPressed: close, isOutline: true),
        dialogButton('OK', onPressed: () async {
          await saveModifierRemap(osKey, table);
          close();
        }),
      ],
      onCancel: close,
    );
  });
}
```

- [x] **Step 2: Add the menu entry**

In `flutter/lib/desktop/widgets/remote_toolbar.dart`, add the import near the existing `import './kb_layout_type_chooser.dart';`:

```dart
import 'package:flutter_hbb/common/widgets/modifier_remap_dialog.dart';
```

Add this method to `_KeyboardMenu`, directly below `localKeyboardType()`:

```dart
  modifierKeys() {
    if (!ffi.ffiModel.keyboard) return Offstage();
    final enabled = !ffi.ffiModel.viewOnly;
    return MenuButton(
      child: Text(translate('Modifier keys')),
      trailingIcon: const Icon(Icons.settings),
      ffi: ffi,
      onPressed: enabled
          ? () => showModifierRemapDialog(pi.platform, ffi.dialogManager)
          : null,
    );
  }
```

Then add it to `menuChildrenGetter`, immediately after `localKeyboardType(),`:

```dart
              keyboardMode(),
              localKeyboardType(),
              modifierKeys(),
              inputSource(),
```

- [x] **Step 3: Register the new strings for translators**

In `src/lang/template.rs`, add these entries alongside the existing ones (the file is a list of `("key", "")` pairs; English falls back to the key itself, so no `en.rs` change is needed):

```rust
        ("Modifier keys", ""),
        ("When controlling", ""),
        ("No remap", ""),
        ("Swap Ctrl/Cmd", ""),
        ("Mac positional", ""),
```

Also delete the now-dead `("Swap control-command key", "")` entry from `src/lang/template.rs`. Leave the translated copies in the other `src/lang/*.rs` files alone — they are harmless and removing 40 of them is churn.

- [x] **Step 4: Verify the Dart analyses clean**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:PUB_CACHE='D:\dev\pub-cache'; $env:PATH='D:\dev\flutter\bin;'+$env:PATH; cd D:\Projects\rustdesk\flutter; flutter analyze lib/common/widgets/modifier_remap_dialog.dart lib/desktop/widgets/remote_toolbar.dart"
```

Expected: no errors. If `CustomAlertDialog`, `dialogButton`, `translate`, `marginOnly`, or `marginSymmetric` are unresolved, the import of `package:flutter_hbb/common.dart` is missing — they all live there.

- [x] **Step 5: Verify the Rust half still builds**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:RUSTUP_HOME='D:\dev\rustup'; $env:CARGO_HOME='D:\dev\cargo'; $env:VCPKG_ROOT='D:\dev\vcpkg'; $env:LIBCLANG_PATH='D:\dev\LLVM\bin'; $env:PATH='D:\dev\cargo\bin;D:\dev\LLVM\bin;'+$env:PATH; cd D:\Projects\rustdesk; cargo check --features hwcodec,vram,flutter"
```

Expected: `Finished`.

- [x] **Step 6: Stage and ask**

```bash
git add flutter/lib/common/widgets/modifier_remap_dialog.dart flutter/lib/desktop/widgets/remote_toolbar.dart src/lang/template.rs
```

Then stop and ask. Suggested message: `feat: add the modifier key remapping dialog`.

---

## Task 7: Build and verify against a live peer

**Files:** none modified. This task produces a runnable binary and a verification report.

**Interfaces:**
- Consumes: everything from Tasks 1–6.
- Produces: `flutter\build\windows\x64\runner\Release\rustdesk.exe`.

- [x] **Step 1: Run the full native build**

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -File 'D:\dev\setup\build-full.ps1'
```

Expected: `=== FULL BUILD DONE ===`. Roughly five minutes; almost all of it is linking, because `[profile.release]` sets `lto = true` and `codegen-units = 1`. There is no hot reload for Rust changes.

- [x] **Step 2: Confirm the new code actually landed in the binary**

```bash
cd /mnt/d/Projects/rustdesk/flutter/build/windows/x64/runner/Release && strings -a data/app.so | grep -i "modifier-remap\|Mac positional"
```

Expected: both strings present. If they are absent the Flutter half did not rebuild — rerun Step 1 rather than proceeding.

- [x] **Step 2b: Run the opt-in config tests while RustDesk is closed**

These are `#[ignore]`d in the normal suite because they race a running client
(see Task 5, Step 1). Step 3 closes every instance anyway, so run them in that
window:

```bash
powershell.exe -NoProfile -Command "Get-Process rustdesk -ErrorAction SilentlyContinue | Stop-Process -Force"
```

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:RUSTUP_HOME='D:\dev\rustup'; $env:CARGO_HOME='D:\dev\cargo'; $env:VCPKG_ROOT='D:\dev\vcpkg'; $env:LIBCLANG_PATH='D:\dev\LLVM\bin'; $env:PATH='D:\dev\cargo\bin;D:\dev\LLVM\bin;'+$env:PATH; cd D:\Projects\rustdesk; cargo test --lib --features hwcodec,vram,flutter modifier_remap -- --ignored --test-threads=1"
```

Expected: `2 passed`. Both restore the original option values on the way out;
confirm afterwards that `RustDesk_local.toml` contains no stray
`modifier-remap` entry.

- [x] **Step 3: Launch it**

Close any other running instance first; the Release exe shares `%APPDATA%\RustDesk` with an installed client and they conflict over the singleton and tray icon. Relaunch by path, never by a previously-noted PID.

```bash
powershell.exe -NoProfile -Command "Get-Process rustdesk -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Seconds 2; Start-Process 'D:\Projects\rustdesk\flutter\build\windows\x64\runner\Release\rustdesk.exe'"
```

- [ ] **Step 4: Verify against a macOS peer**

Connect to a Mac, open the toolbar's Keyboard menu, choose **Modifier keys…**, click **Mac positional**, and confirm:

1. The dialog shows Ctrl / Win / Alt / Shift on the left and Control / Option / Command / Shift in the dropdowns.
2. After OK, `Alt+C` on the Windows keyboard performs a copy on the Mac (it arrives as Cmd+C).
3. `Alt+Tab` switches applications on the Mac (Cmd+Tab) rather than switching windows locally.
4. `Win+click` behaves as Option+click.
5. Ctrl still behaves as Control — `Ctrl+A` moves to line start in a Mac text field rather than selecting all.
6. Reopening the dialog shows the saved mapping, and it persists across a restart of the app.
7. Repeat 2 and 5 with the keyboard mode set to Legacy mode, then Map mode, then Translate mode, from the same Keyboard menu.

- [~] **Step 5: Verify the AltGr regression** — WAIVED, see below

Connect to a **Windows** peer with a layout that has AltGr (for example UK or German) and a mapping active that remaps Ctrl. Type `AltGr+4` (UK: `€`) into Notepad on the peer and confirm the character appears. If a bare Ctrl arrives instead, the `0xE0` guard in `is_altgr_synthetic_ctrl` is not firing — check `event.position_code` at the call site.

- [ ] **Step 6: Verify the mid-hold safety**

With a mapping active, hold Alt down, and while still holding it open the dialog and switch to **No remap**, then release Alt. Confirm no modifier is stuck on the peer (typing a letter produces the letter, not a shortcut). This exercises the `release_remote_keys` call added in Task 2.

- [ ] **Step 7: Report and ask**

Report the results of Steps 4–6 as evidence, including anything that failed, then stage nothing and ask whether to commit the branch.

---

## Execution results (2026-08-14)

**Verified:**

- Release build clean: Rust 4m12s, Flutter 74.7s, `rustdesk.exe` produced.
- New code confirmed present in the artifacts: `modifier-remap`, `Modifier keys`,
  `Swap Ctrl/Cmd`, `Mac positional` in the Dart AOT snapshot; `modifier-remap`,
  `modifier-remap-migrated` and the migration log line in `librustdesk.dll`.
- The retired string `Swap control-command key` is gone from the Dart snapshot
  (0 occurrences). `allow_swap_key` survives in the DLL only as the config field
  the migration reads, as designed.
- App launches and reaches the main window.
- The two `#[ignore]`d config round-trip tests pass with RustDesk closed
  (`2 passed`), and restore the config with no residue. This also confirms the
  diagnosis that they only fail when racing a running client.
- **Migration verified against real data.** Both saved macOS peers had
  `allow_swap_key = true` (`config/peers/180824860.toml:132` and
  `config/peers/192.168.1.117.toml:29`). On first launch of the new build the
  migration seeded exactly the right table and set its marker:

```toml
modifier-remap-migrated = 'Y'
modifier-remap = '{"macos":{"alt":"alt","ctrl":"meta","meta":"ctrl","shift":"shift"}}'
```

  That is `ctrl→meta, meta→ctrl` — the retired checkbox's behaviour — seeded for
  `macos` because the local machine is Windows. Existing behaviour preserved.

**Blocked, NOT verified:**

- Steps 4 and 6 (the live macOS checks: Alt+C as copy, Alt+Tab, Option+click,
  Ctrl unchanged, all three keyboard modes, and the mid-hold safety check).
  A session was opened to peer `180824860`, which returned **"Wrong password"**.
  Credentials are the user's to supply; no retry was attempted.
- Step 5 (the AltGr regression) needs a Windows peer running a layout that has
  AltGr, such as UK or German. Not available in this environment.
- The dialog's visual layout has never been rendered — row alignment, dropdown
  width, and whether the three preset buttons wrap sensibly are all unconfirmed.

---

## Verification round 2 (2026-08-15) — mapping verified end-to-end

Session to `192.168.1.117` (stats-macbook-pro) connected this time. The active
config is now the **Mac positional** preset, not the migrated one:

```toml
modifier-remap = '{"macos":{"ctrl":"ctrl","meta":"alt","alt":"meta","shift":"shift"}}'
```

**Measurement rig (all objective, no eyeballing).** `/tmp/flags` on the Mac
(compiled Swift) prints `CGEventSource.flagsState(.combinedSessionState)` plus
the frontmost app name, read over the SSH control master.
`D:\dev\setup\rd-key.ps1` focuses the RustDesk session window, **verifies it is
foreground, and injects nothing if it is not** — so test keystrokes can never
leak into other applications. Target was a scratch TextEdit document, never a
real one.

**PASSED — the mapping itself, measured directly and repeatedly:**

| Windows key | Expected on Mac | Measured | Release |
| --- | --- | --- | --- |
| Shift | shift | shift | clean |
| Ctrl | control | control | clean |
| Alt | command | command | clean |
| Win | option | option | clean |

Run twice, ~20 minutes apart, identical both times. No modifier ever stuck
(`mods=none` after every release, on both machines). This covers plan item 5
(Ctrl still arrives as Control, not Command) at the mechanism level.

**PASSED — plan item 3, Alt+Tab:** Mac frontmost went `TextEdit → Xcode`
(Cmd+Tab fired) while the Windows foreground window stayed on the RustDesk
session — so it did **not** trigger a local Alt+Tab.

**PASSED — Alt+letter reaching the Mac as Cmd+letter:** Alt+A selected the whole
document (confirmed twice — visible selection highlight, then a subsequent
plain keystroke replaced the entire selected contents).

**RESOLVED — what first looked like an anomaly was the peer's own key bindings.**
Mid-session, Alt+A / Alt+C / Alt+Z went inert while plain letters still typed and
the modifier truth table still measured correct. That was **not** a client bug.
This Mac remaps the common editing actions off Command and onto Control, via
App Shortcuts (`defaults read -g NSUserKeyEquivalents`):

```
Copy = ^c   Paste = ^v   Cut = ^x   Select All = ^a
Undo = ^z   Redo = ^$z   Save = ^s  Find = ^f   New Window = ^n
```

plus `~/Library/KeyBindings/DefaultKeyBinding.dict` (symlinked to
`Projects/setup/macos/DefaultKeyBinding.dict`) binding `^z`/`^$z`/`^\U007F` by
selector. So on this peer **Cmd+C is bound to nothing** and Alt+C doing nothing
is the correct outcome. The timing fits exactly: those bindings were being
edited in another session while this run was in progress (the dict was linked
at 21:18, mid-test), which is why Cmd+A worked early and stopped later.

**PASSED — end-to-end, against the bindings this peer actually uses.** With the
remap active, Windows `Ctrl+key` arrives as Mac `Control+key` and fires the
rebound menu actions. Verified by clipboard round-trip on a scratch document:

| Sent from Windows | Arrives as | Rebound action | Result |
| --- | --- | --- | --- |
| Ctrl+A | Control+A | Select All | selected |
| Ctrl+C | Control+C | Copy | clipboard became the document text |
| Ctrl+V | Control+V | Paste | document became the clipboard text |

Read back with a fresh sentinel each time, so neither direction can be a stale
clipboard.

**Two plan steps are written against assumptions this peer breaks — fix the plan,
not the code:**

- Step 4 item 2 ("Alt+C performs a copy") is **not valid here**: Copy is `^c`,
  so Alt+C correctly does nothing. The equivalent check on this machine is
  Ctrl+C, which passes.
- Step 4 item 5 ("Ctrl+A moves to line start rather than selecting all") is
  **not valid here** either: Select All is bound to `^a`, so Ctrl+A selecting
  all is correct. What that item is really testing — that Ctrl arrives as
  Control and not as Command — is proven by the truth table above.

**Step 5 (AltGr) — WAIVED by the user, 2026-08-15.** Roy does not use UK,
German or any other layout that has an AltGr key, and has no Windows peer
running one, so this check will not be performed. Keep the guard itself: the
`0xE0` check in `is_altgr_synthetic_ctrl` exists because on those layouts
AltGr arrives as a synthetic Ctrl that must not be remapped, and the code is
still correct for users who do have such a layout. If it ever needs testing,
the check is: on a UK/German **Windows** peer with a mapping that remaps Ctrl,
type `AltGr+4` into Notepad and confirm `€` appears rather than a bare Ctrl.

**Unrelated observation:** the peer's BetterDisplay layout has changed since the
cursor work — all three displays now report `scale=1.0` (the U3415W virtual
display is no longer 2x). The Retina shim is therefore inert on this peer right
now, so this session exercised none of the cursor-fix parking logic.

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
| --- | --- |
| Configuration model, JSON shape, fallback rules | 1, 2 |
| `ModifierMap` operations, side preservation, AltGr protection | 1 |
| Cache and invalidation | 2 |
| Hook: `event_to_key_events` | 3 |
| Hook: `get_modifiers_state` | 3 |
| Hook: `swap_modifier_mouse` | 4 |
| Delete `swap_modifier_key` | 4 |
| Targeted cleanup: `peer` threaded into `legacy_keyboard_mode` | 3 |
| Mobile `_input_key` exclusion | 4 (documented in the `send_key_event` comment) |
| Held-key safety on mapping change | 2 (write hook), 7 (verification) |
| UI: dialog, presets, labels, no validation | 6 |
| UI: shown for every peer OS | 6 |
| UI: Sciter checkbox removed | 4 |
| Migration | 5 |
| Test plan | 1, 3, 5, 7 |
| Build and verification cost | 7 |

No gaps.

**Type consistency:** `ModifierMap`, `Slot`, `Slot::ALL`, `Slot::name`, `Slot::from_name`, `map_key`, `map_control_key`, `map_state`, `to_json_value`, `parse_all`, `for_peer`, `invalidate_cache`, `write_map_for`, `swap_ctrl_cmd_map`, `migrate_from_allow_swap_key`, `remap_event` are spelled identically everywhere they appear. `map_state` keeps the `(alt, ctrl, shift, command)` order of the existing `get_modifiers_state` in all three of its definition, its tests, and its call sites. The Dart `kModifierSlots` list matches `Slot::name()` exactly, and `peerOsKey()` matches the Rust OS-key normalisation.

**Placeholder scan:** no TBDs, no "handle errors appropriately", no "similar to Task N". Every code step carries the actual code.
