//! Configurable modifier key remapping.
//!
//! Rewrites the four modifier keys (Ctrl, Meta, Alt, Shift) in *local* key
//! space, before any peer scancode translation happens. See
//! `docs/superpowers/specs/2026-08-14-modifier-key-remapping-design.md`.

use hbb_common::config::{LocalConfig, PeerConfig};
use hbb_common::message_proto::ControlKey;
use hbb_common::serde_json;
use rdev::Key;
use std::collections::HashMap;
use std::sync::RwLock;

pub const OPTION_MODIFIER_REMAP: &str = "modifier-remap";
pub const OPTION_MODIFIER_REMAP_MIGRATED: &str = "modifier-remap-migrated";

// `src/keyboard.rs` has its own private copies of these; they must stay in
// sync, because they are the OS keys of the stored `modifier-remap` object.
const OS_LOWER_WINDOWS: &str = "windows";
const OS_LOWER_LINUX: &str = "linux";
const OS_LOWER_MACOS: &str = "macos";

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
/// characters. The legacy path uses the same test in `src/keyboard.rs`.
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

/// Merges one OS entry into a raw option string and returns the new raw value.
/// An identity map removes the entry entirely, so an untouched config collapses
/// back to empty and costs nothing to parse.
///
/// Pure, so it is testable without touching the shared on-disk config.
fn merge_into(raw: &str, os_lower: &str, map: ModifierMap) -> String {
    let mut maps = parse_all(raw);
    if map.is_identity() {
        maps.remove(os_lower);
    } else {
        maps.insert(os_lower.to_string(), map);
    }
    if maps.is_empty() {
        return String::new();
    }
    let mut root = serde_json::Map::new();
    for (os, m) in maps.iter() {
        root.insert(os.clone(), m.to_json_value());
    }
    serde_json::Value::Object(root).to_string()
}

/// Merges one OS entry into the stored option.
pub fn write_map_for(os_lower: &str, map: ModifierMap) {
    let raw = merge_into(
        &LocalConfig::get_option(OPTION_MODIFIER_REMAP),
        os_lower,
        map,
    );
    LocalConfig::set_option(OPTION_MODIFIER_REMAP.to_string(), raw);
    invalidate_cache();
}

/// The mapping the retired "Swap control-command key" checkbox produced.
pub fn swap_ctrl_cmd_map() -> ModifierMap {
    let mut m = ModifierMap::identity();
    m.set(Slot::Ctrl, Slot::Meta);
    m.set(Slot::Meta, Slot::Ctrl);
    m
}

/// Which OS tables the retired checkbox's setting applies to. It was only ever
/// offered when exactly one side of the connection was macOS, so the answer is
/// fully determined by which side the local machine is on.
fn seed_targets() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[OS_LOWER_WINDOWS, OS_LOWER_LINUX]
    } else {
        &[OS_LOWER_MACOS]
    }
}

/// Pure: the raw option value produced by seeding the swap map for whichever
/// OS tables the retired checkbox applied to.
fn seeded_raw(raw: &str) -> String {
    let swap = swap_ctrl_cmd_map();
    let mut out = raw.to_string();
    for os in seed_targets() {
        out = merge_into(&out, os, swap);
    }
    out
}

/// Pure: whether the one-shot migration should seed anything. Only when the
/// user has not already configured a table and at least one saved peer had the
/// retired flag on.
fn should_seed(existing_map_raw: &str, any_peer_swap: bool) -> bool {
    existing_map_raw.is_empty() && any_peer_swap
}

/// Carries the retired per-peer `allow_swap_key` flag into the new global
/// table. Runs at most once, guarded by its own marker option.
pub fn migrate_from_allow_swap_key() {
    if !LocalConfig::get_option(OPTION_MODIFIER_REMAP_MIGRATED).is_empty() {
        return;
    }
    LocalConfig::set_option(OPTION_MODIFIER_REMAP_MIGRATED.to_string(), "Y".to_string());

    let existing = LocalConfig::get_option(OPTION_MODIFIER_REMAP);
    let any_swap = PeerConfig::peers(None)
        .into_iter()
        .any(|(_, _, c)| c.allow_swap_key.v);
    if !should_seed(&existing, any_swap) {
        return;
    }

    LocalConfig::set_option(OPTION_MODIFIER_REMAP.to_string(), seeded_raw(&existing));
    invalidate_cache();
    hbb_common::log::info!("migrated allow_swap_key into {}", OPTION_MODIFIER_REMAP);
}

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
        assert_eq!(
            m.map_state(true, false, false, false),
            (false, false, false, true)
        );
        // Holding local Win/Meta must report `alt`.
        assert_eq!(
            m.map_state(false, false, false, true),
            (true, false, false, false)
        );
        // Ctrl maps to itself.
        assert_eq!(
            m.map_state(false, true, false, false),
            (false, true, false, false)
        );
        // Shift is untouched.
        assert_eq!(
            m.map_state(false, false, true, false),
            (false, false, true, false)
        );
    }

    /// Two sources may legitimately share one target; the result is a union.
    #[test]
    fn held_state_unions_colliding_targets() {
        let mut m = ModifierMap::identity();
        m.set(Slot::Ctrl, Slot::Meta);
        m.set(Slot::Meta, Slot::Meta);
        assert_eq!(
            m.map_state(false, true, false, false),
            (false, false, false, true)
        );
        assert_eq!(
            m.map_state(false, false, false, true),
            (false, false, false, true)
        );
        assert_eq!(
            m.map_state(false, true, false, true),
            (false, false, false, true)
        );
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

    // ---- Pure tests: no shared on-disk config, so always deterministic. ----

    #[test]
    fn merge_into_adds_updates_and_removes_entries() {
        let raw = merge_into("", "macos", mac_positional());
        assert_eq!(*parse_all(&raw).get("macos").unwrap(), mac_positional());

        // A second OS is added alongside, not replacing.
        let raw = merge_into(&raw, "windows", swap_ctrl_cmd());
        let all = parse_all(&raw);
        assert_eq!(all.len(), 2);
        assert_eq!(*all.get("macos").unwrap(), mac_positional());
        assert_eq!(*all.get("windows").unwrap(), swap_ctrl_cmd());

        // Writing identity removes that entry rather than storing a no-op table.
        let raw = merge_into(&raw, "macos", ModifierMap::identity());
        let all = parse_all(&raw);
        assert_eq!(all.len(), 1);
        assert!(!all.contains_key("macos"));

        // Removing the last entry collapses back to empty, not "{}".
        let raw = merge_into(&raw, "windows", ModifierMap::identity());
        assert!(raw.is_empty());
    }

    #[test]
    fn swap_preset_matches_the_retired_checkbox() {
        let m = swap_ctrl_cmd_map();
        assert_eq!(m.map_key(Key::ControlLeft, 0x1D), Key::MetaLeft);
        assert_eq!(m.map_key(Key::MetaLeft, 0xE05B), Key::ControlLeft);
        assert_eq!(m.map_key(Key::Alt, 0x38), Key::Alt);
        assert_eq!(m.map_control_key(ControlKey::RControl), ControlKey::RWin);
    }

    /// The old checkbox only ever appeared when exactly one side was macOS, so
    /// which OS it applied to is fully determined by the local platform.
    #[test]
    fn seeding_writes_the_swap_for_the_opposite_platform() {
        let raw = seeded_raw("");
        let all = parse_all(&raw);
        for os in seed_targets() {
            assert_eq!(
                all.get(*os).copied().unwrap_or_default(),
                swap_ctrl_cmd_map(),
                "expected the swap map seeded for {os}"
            );
        }
        if cfg!(target_os = "macos") {
            assert!(!all.contains_key(OS_LOWER_MACOS));
            assert_eq!(all.len(), 2);
        } else {
            assert!(!all.contains_key(OS_LOWER_WINDOWS));
            assert_eq!(all.len(), 1);
        }
    }

    #[test]
    fn seeding_only_when_untouched_and_a_peer_had_the_flag() {
        assert!(should_seed("", true));
        // The user already configured something: never overwrite it.
        assert!(!should_seed(r#"{"macos":{"alt":"meta"}}"#, true));
        // Nobody had the old flag on: nothing to carry forward.
        assert!(!should_seed("", false));
        assert!(!should_seed(r#"{"macos":{"alt":"meta"}}"#, false));
    }

    // ---- Opt-in tests: these read and write the REAL %APPDATA%\RustDesk
    // config, which a running RustDesk client also writes. They race it, so
    // they are ignored by default. Close every RustDesk instance, then:
    //     cargo test --lib --features hwcodec,vram,flutter modifier_remap \
    //         -- --ignored --test-threads=1

    fn with_saved_options<T>(f: impl FnOnce() -> T) -> T {
        let original_map = LocalConfig::get_option(OPTION_MODIFIER_REMAP);
        let original_marker = LocalConfig::get_option(OPTION_MODIFIER_REMAP_MIGRATED);
        let result = f();
        LocalConfig::set_option(OPTION_MODIFIER_REMAP.to_string(), original_map);
        LocalConfig::set_option(OPTION_MODIFIER_REMAP_MIGRATED.to_string(), original_marker);
        invalidate_cache();
        result
    }

    #[test]
    fn unknown_os_is_always_identity() {
        assert!(for_peer("definitely-not-an-os").is_identity());
        assert!(for_peer("").is_identity());
    }

    #[test]
    #[ignore = "reads/writes the real RustDesk config; races a running client"]
    fn write_then_read_round_trips_through_the_cache() {
        with_saved_options(|| {
            LocalConfig::set_option(OPTION_MODIFIER_REMAP.to_string(), String::new());
            invalidate_cache();

            write_map_for("macos", mac_positional());
            assert_eq!(for_peer("macos"), mac_positional());
            assert!(for_peer("windows").is_identity());

            write_map_for("macos", ModifierMap::identity());
            assert!(for_peer("macos").is_identity());
            assert!(LocalConfig::get_option(OPTION_MODIFIER_REMAP).is_empty());
        });
    }

    #[test]
    #[ignore = "reads/writes the real RustDesk config; races a running client"]
    fn migration_is_one_shot() {
        with_saved_options(|| {
            LocalConfig::set_option(OPTION_MODIFIER_REMAP.to_string(), String::new());
            LocalConfig::set_option(OPTION_MODIFIER_REMAP_MIGRATED.to_string(), String::new());
            invalidate_cache();

            migrate_from_allow_swap_key();
            assert_eq!(LocalConfig::get_option(OPTION_MODIFIER_REMAP_MIGRATED), "Y");

            // A second run must not touch a table the user has since edited.
            write_map_for("macos", mac_positional());
            migrate_from_allow_swap_key();
            assert_eq!(for_peer("macos"), mac_positional());
        });
    }
}
