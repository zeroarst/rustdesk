# Configurable modifier key remapping

**Date:** 2026-08-14
**Status:** Design approved, not yet implemented

## Problem

RustDesk offers exactly one modifier remapping: a per-peer checkbox, "Swap
control-command key", that swaps Control and Meta in both directions. It is
hardcoded and only appears when exactly one side of the connection is macOS.

That produces a single fixed mapping — `Ctrl→Cmd`, `Win→Ctrl`, `Alt→Option` —
and there is no way to express any other. In particular there is no way to get
the *positional* mapping that a Mac-keyboard user wants when driving a Mac from
a Windows keyboard:

| Local (Windows keyboard) | Peer (macOS) |
| --- | --- |
| Ctrl | Control |
| Win | Option |
| Alt | Command |

This matches physical key position: a Mac keyboard reads Ctrl · Option · Cmd
left-to-right, a Windows keyboard reads Ctrl · Win · Alt.

## Goals

- Let the user freely choose which peer modifier each of their four local
  modifiers sends.
- Configure it once, globally, per target OS — not per saved peer.
- Work in all three keyboard modes (legacy, map, translate), for keys pressed
  as the primary key *and* keys held as modifiers of another key or of a mouse
  click.
- Do not break AltGr.

## Non-goals

- Mapping a modifier to nothing (swallowing it locally).
- Mapping non-modifier keys (CapsLock→Ctrl, F13→Cmd, and similar).
- Configuring left and right modifiers independently.
- A configuration UI in the Sciter (legacy, non-Flutter) client.
- The web client, whose key handling lives in a separate closed JavaScript
  engine reached through `flutter/lib/web/bridge.dart`.

## Existing behaviour being replaced

| Location | Role |
| --- | --- |
| `src/ui_session_interface.rs:700` | `swap_modifier_key()` — rewrites the `ControlKey` union, the `modifiers` list, and the raw `chr` keycode |
| `src/ui_session_interface.rs:1938` | `swap_modifier_mouse()` — rewrites modifiers attached to mouse events |
| `src/client.rs:2183`, `src/client.rs:2394` | `allow_swap_key` toggle read/write |
| `flutter/lib/common/widgets/toolbar.dart:1216` | the Flutter checkbox |
| `src/ui/header.tis:222` | the Sciter checkbox |

`swap_modifier_key()` runs at the very end of the pipeline, in *peer* key space
— the code has already been translated into the peer's scancode numbering by
then. That forces a three-way `match` on `"windows"` / `"macos"` / `_` to decode,
swap, and re-encode, and it means the local context (notably AltGr) is no longer
visible.

## Approach

Remap in **local key space, before any peer translation**.

Both input paths converge on a single function earlier than the existing hook:

- The native `rdev` grab loop reaches it via `keyboard::client::process_event()`.
- The Flutter and mobile path reaches it via
  `keyboard::client::process_event_with_session()`, after
  `Session::_handle_key_non_flutter_simulation()`
  (`src/ui_session_interface.rs:1042`) synthesises an equivalent `rdev::Event`.

Both then call `event_to_key_events()` at `src/keyboard.rs:940`. Remapping there
means:

- No per-platform decode/re-encode; the existing translation tables operate on
  the already-remapped key.
- AltGr stays detectable, because the extended-scancode marker is still on the
  event.
- The semantics are the honest ones: "my Alt key *is* Command", rather than
  "rewrite Command into Alt after the fact".

Rejected alternatives:

- **Extend `swap_modifier_key()` into a table lookup.** Smallest diff and closest
  to upstream, but keeps the peer-platform branching and cannot see AltGr, so the
  AltGr protection would be approximate.
- **Remap on the peer.** Changes the protocol, requires the far end to run this
  fork, and the peer cannot know what keyboard the operator is using.

## Configuration

A single global option, read and written through the existing local-config
plumbing (`main_get_local_option` / `main_set_local_option`), under the key
`modifier-remap`. The value is a JSON object keyed by lowercased peer OS, using
the constants already defined at `src/keyboard.rs:27`
(`windows`, `linux`, `macos`, `android`):

```json
{
  "macos": { "ctrl": "ctrl", "meta": "alt", "alt": "meta", "shift": "shift" }
}
```

Four canonical slot names on both sides: `ctrl`, `meta`, `alt`, `shift`.

Fallback rules — every one of these means "leave this modifier alone", so the
default configuration is `{}` and costs nothing:

- The `modifier-remap` option is absent or unparseable.
- The peer's OS has no entry.
- A slot is absent from an OS entry.
- A slot's value is not one of the four canonical names.

Slot names are neutral in storage but **displayed per target OS**: the macOS
table reads Control / Option / Command / Shift, the Windows table reads
Ctrl / Alt / Win / Shift.

## Components

### `src/modifier_remap.rs` (new)

A small pure-function module. Public surface:

- `ModifierMap` — an immutable value type holding the four slot assignments.
- `ModifierMap::is_identity()` — lets every call site short-circuit to zero cost
  when unconfigured.
- `ModifierMap::map_key(rdev::Key, position_code) -> rdev::Key`
- `ModifierMap::map_control_key(ControlKey) -> ControlKey`
- `ModifierMap::map_state((alt, ctrl, shift, command)) -> (alt, ctrl, shift, command)`
- `for_peer(peer_platform: &str) -> ModifierMap`
- `invalidate_cache()`

Parsed maps are cached in a `RwLock<HashMap<String, ModifierMap>>`, invalidated
when the setting is written. This is the keystroke hot path; it must not parse
JSON per key.

Two invariants enforced inside `map_key`:

**Side is preserved.** `ControlLeft` maps to the left-hand instance of its
target, `ControlRight` to the right-hand one. This is what makes a four-row
table behave sanely without exposing eight rows.

**AltGr is never remapped.** `Key::AltGr` passes through untouched. So does a
`Key::ControlLeft` whose `position_code >> 8 == 0xE0` — the synthetic Control
that Windows injects ahead of AltGr. The legacy path already uses exactly this
test at `src/keyboard.rs:1067`, so it is a known-good check rather than a new
heuristic. Without this, remapping Ctrl would break typing accented characters.

After mapping, the caller recomputes the event's `position_code` and
`platform_code` for the **local** platform from the new key
(`rdev::win_scancode_from_key`, `rdev::code_from_key`,
`rdev::macos_keycode_from_key`), so downstream translation, dead-key handling,
and key-release tracking all operate on the remapped key without knowing
anything happened.

### Hook sites

| Location | Change |
| --- | --- |
| `src/keyboard.rs:940` — `event_to_key_events()` | Remap the incoming `rdev::Event` at the top, then recompute its platform codes. Covers the grab loop and the Flutter/mobile path, in all three keyboard modes. |
| `src/keyboard.rs:348` — `get_modifiers_state()` | Apply `map_state()` to the held-modifier tuple, so legacy mode reports `command` when remapped-Alt is held. Requires threading `peer` in. |
| `src/ui_session_interface.rs:1938` — `swap_modifier_mouse()` | Becomes a `map_control_key()` pass, preserving Ctrl+click behaviour. |

`swap_modifier_key()` at `src/ui_session_interface.rs:700` is deleted outright,
together with its three-way peer-platform `match`.

### Targeted cleanup

`legacy_keyboard_mode()` currently re-derives the peer platform by calling
`get_peer_platform()`, which returns the *current* session and is therefore
wrong in a multi-window setup. Since `event_to_key_events()` already receives
`peer`, thread it through instead. This is required anyway to give
`get_modifiers_state()` its peer context.

### Deliberate exclusion

The on-screen modifier buttons on mobile go through
`Session::_input_key()` (`src/ui_session_interface.rs:1109`), which builds a
`KeyEvent` directly rather than an `rdev::Event`. Those are explicit semantic
requests — tapping "Ctrl" means "send Ctrl to the peer" — and are **not**
remapped.

### Held-key safety

Changing the mapping while a key is physically held would send a press of the
old target and a release of the new one, stranding a modifier down on the peer.
On any mapping change, call the existing `release_remote_keys_for_events()` to
clear held keys first.

## UI

There is no Keyboard tab in desktop Settings (the tabs are General, Security,
Network, Display, Account, Printer, About). There *is* an exact precedent for a
global keyboard setting edited from inside a session:
`localKeyboardType()` at `flutter/lib/desktop/widgets/remote_toolbar.dart:2506`
writes a global value from the session's Keyboard menu.

The new UI follows that shape: a **"Modifier keys…"** entry in the session's
Keyboard menu, opening a dialog. It sits where the old checkbox was, and because
it launches from a live session it already knows the target OS — there is no
OS picker to get wrong.

```
Modifier keys — when controlling macOS

  Ctrl      →  [ Control  ▾ ]
  Win       →  [ Option   ▾ ]
  Alt       →  [ Command  ▾ ]
  Shift     →  [ Shift    ▾ ]

  Presets:  [ No remap ]  [ Swap Ctrl/Cmd ]  [ Mac positional ]
```

- The left column is labelled for the local OS, the dropdowns for the peer's OS.
- `Swap Ctrl/Cmd` reproduces the retired checkbox exactly
  (`ctrl→meta`, `meta→ctrl`).
- `Mac positional` is `ctrl→ctrl`, `meta→alt`, `alt→meta`.
- Changes apply on save and take effect immediately in every open session with
  that peer OS.
- No collision validation. Mapping both Ctrl and Win to Command is legitimate,
  and so is leaving a peer modifier unreachable; a warning would be noise.

Unlike the old checkbox, the entry is shown for **every** peer OS, not only
cross-macOS sessions.

The Sciter checkbox at `src/ui/header.tis:222` is removed with no replacement.
Once the core stops honouring `allow_swap_key` it would silently become a no-op,
and that client is deprecated.

## Migration

One-shot, guarded by a `modifier-remap-migrated` marker option so it cannot run
twice.

On first launch after the upgrade, if `modifier-remap` is unset, scan saved peer
configs for `allow_swap_key = true`. If any is found, seed the Ctrl↔Meta swap
into the table for the OS it applied to. Because the old checkbox only appeared
when exactly one side was macOS, that is:

- Local machine is macOS → seed both `windows` and `linux`.
- Otherwise → seed `macos`.

The `allow_swap_key` field stays in the config struct so migration can read it
and older config files still deserialize. Only its toggle handlers in
`src/client.rs` and its two UI sites are removed.

## Testing

Unit tests beside the engine in `src/modifier_remap.rs`. Everything there is a
pure function, so this is cheap and covers the risky parts:

- Identity map short-circuits; `{}`, unknown OS keys, unknown slot names, and
  unparseable JSON all fall back to identity rather than erroring.
- The positional mapping: LCtrl→LCtrl, LWin→LAlt, LAlt→LWin, plus right-hand
  mirrors.
- AltGr protection: `Key::AltGr` unchanged, and `ControlLeft` carrying the
  `0xE0` marker unchanged, even with a Ctrl remap active.
- Held-modifier tuple: with `alt→meta`, holding local Alt yields
  `command = true, alt = false`.
- `ControlKey` mapping for the mouse-modifier path.
- Press/release symmetry: the same input key produces mirrored down and up
  events.

Plus a table-driven test over `event_to_key_events()` for a macOS peer in each
of legacy, map, and translate mode, asserting the emitted codes.

## Build and verification cost

This is a Rust change, so there is no hot reload.

- Compile feedback: `cargo check --features hwcodec,vram,flutter` — about one
  minute.
- Full release build for hands-on testing: `D:\dev\setup\build-full.ps1` — about
  five minutes, almost entirely linking.

Final verification requires driving a real macOS peer from a Windows keyboard and
confirming that Cmd+C, Cmd+Tab, and Option+click land correctly, plus a
regression check that AltGr still types accented characters against a
Windows peer.
