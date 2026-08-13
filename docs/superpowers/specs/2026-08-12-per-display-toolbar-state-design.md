# Per-display toolbar collapse/hide memory — design

Date: 2026-08-12
Status: approved (design)
Feature branch: per-display-window-memory
Sibling spec: 2026-08-12-taskbar-window-order-design.md (same storage pattern)

## Problem

The toolbar's collapsed and hidden states are stored as single per-peer
options (`collapse_toolbar`, `hide_toolbar`), so all session windows of a
peer share one state and the last toggle wins for every window on the next
connection. The user wants each window (each remote display) to remember
its own toolbar state across reconnects — e.g. M4's toolbar collapsed, the
rest expanded.

## Design

### Storage

- Two LOCAL flutter options per (peer, display), values `'Y'`/`'N'`:
  - `toolbar-collapse-d<display>_<peerId>`
  - `toolbar-hide-d<display>_<peerId>`
- Built by a shared helper `perDisplayLocalKey(prefix, peerId, display)` in
  `window_transition.dart`; `taskbarOrderKey` is refactored onto it (output
  unchanged — pinned by unit test).
- Local config (not peer config) for the same reason as the taskbar ranks:
  live session windows re-store their own PeerConfig snapshots and clobber
  peer-config keys written after connect.

### Read (toolbar init)

- `ToolbarState.init(sessionId, {peerId, display})` — new optional params,
  provided by the remote toolbar (it has `ffi` and `id`; display =
  `ffi.ffiModel.pi.currentDisplay` at init).
- A stored per-display value wins; empty/absent falls back to the existing
  per-peer session toggles, so behavior is unchanged until a window's
  toolbar is toggled once. Callers that pass no peerId/display (e.g. view
  camera) keep stock behavior.

### Write (toggles)

- `switchCollapse` / `switchHide`: when peerId+display are known, flip the
  Rx value and write `'Y'`/`'N'` to the per-display local key — the
  per-peer session option is no longer written from these toggles (it
  remains the settings-page default / fallback). Without peerId+display,
  the legacy session-toggle path is kept.

### Edge cases

- Switching the shown display in-window after init: toolbar state stays
  keyed to the display it initialized with until the next connection.
- Duplicate windows of the same display would share a key — not a
  supported layout; ignored.

### Testing

- Unit tests: `perDisplayLocalKey` output, `taskbarOrderKey` format pinned
  unchanged, collapse/hide key helpers.
- Live: collapse only M4's toolbar → close-all → reconnect → M4 collapsed,
  others expanded; repeat with hide on one window.
