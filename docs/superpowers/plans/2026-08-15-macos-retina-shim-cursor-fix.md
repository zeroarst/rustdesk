# macOS Retina-Shim Cursor Fix ("logical everywhere + safe parking") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the cursor land correctly in every viewing mode against an unmodified macOS RustDesk 1.4.9 host whose displays mix scale factors, by keeping the host's per-connection Retina shim permanently inert and sending logical coordinates everywhere.

**Architecture:** The macOS host's `Retina::on_mouse_event` (host-side `connection.rs`) divides incoming absolute mouse coordinates by the *current* display's scale when they land in its mixed-unit rect (logical origin, physical extent), keyed on the connection's `display_idx`. The client fully controls `display_idx` — `SwitchDisplay` is the only message that moves it. The fix: (1) never send `SwitchDisplay` targeting a scaled display when a scale-1 display exists (intercept in `Session::switch_display`, park at connect if needed), so the shim never arms; (2) lay out and send all coordinates in logical units (extend the existing divide-by-scale from "multi-display only" to "whenever a scale-1 display exists"), which the inert shim passes through verbatim. When every peer display is scaled there is no parking spot: keep legacy behavior (info-space + armed shim), which is correct for the common single-Retina-MacBook case.

**Tech Stack:** Dart (Flutter client UI), Rust (client core `src/`), flutter_test, cargo test. Builds/tests run on Windows via `cmd.exe` interop from WSL.

**Background evidence (measured 2026-08-15, see `HANDOFF-cursor-hidpi.md`):** peer = Mac 192.168.1.117, RustDesk 1.4.9, displays `[id 1: (0,0) 1920x1200 @1x builtin] [id 113: (4672,-360) 1200x1920 @1x] [id 118: (1920,24) 2752x1152 logical @2x]`. Sweeps proved: `display_idx` parked on a scale-1 display ⇒ logical coords pass verbatim (100% correct); parked on the 2x display ⇒ everything right of x=1920 is halved and the portrait display becomes unreachable by any wire coordinate.

## Global Constraints

- **NEVER `git commit`.** Stage with `git add` only; the user commits personally. Every "commit" step in this plan means *stage and stop*.
- All Flutter/cargo commands run Windows-side via `cmd.exe /c '...'` with `dangerouslyDisableSandbox: true`; never run WSL-native `flutter`/`cargo` (toolchain lives on Windows; see project memory).
- Do not touch `flutter/lib/mobile/` (mobile has a pre-existing unrelated gap, documented out of scope).
- The peer-platform string for macOS is the Dart constant `kPeerPlatformMacOS` / Rust literal `"Mac OS"` — verify before use (Task 4 Step 1).
- Match surrounding comment density and style; comments explain *why* (the shim), pointing at `src/display_park.rs` as the canonical explanation.

---

### Task 1: Dart pure helper `useLogicalDisplayLayout` (TDD)

**Files:**
- Create: `flutter/lib/common/logical_display_layout.dart`
- Test: `flutter/test/logical_display_layout_test.dart`

**Interfaces:**
- Produces: `bool useLogicalDisplayLayout({required bool isPeerLinux, required bool isPeerMacOS, required Iterable<double> allDisplayScales})` — consumed by Task 2 at three sites.

- [ ] **Step 1: Check the package import name**

Run: `grep "^name:" flutter/pubspec.yaml` and look at the import style of `flutter/test/input_modifier_utils_test.dart`. Use the same `package:<name>/...` prefix in the new test (expected: `flutter_hbb`).

- [ ] **Step 2: Write the failing test**

`flutter/test/logical_display_layout_test.dart`:

```dart
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_hbb/common/logical_display_layout.dart';

void main() {
  test('linux peers always use logical layout', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: true, isPeerMacOS: false, allDisplayScales: [2.0]),
        true);
  });
  test('other peers never use logical layout', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false,
            isPeerMacOS: false,
            allDisplayScales: [1.0, 2.0]),
        false);
  });
  test('mac with a scale-1 display uses logical layout (parking possible)', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false,
            isPeerMacOS: true,
            allDisplayScales: [1.0, 1.0, 2.0]),
        true);
  });
  test('mac with all displays scaled keeps legacy info-space (no parking)', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false, isPeerMacOS: true, allDisplayScales: [2.0]),
        false);
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false,
            isPeerMacOS: true,
            allDisplayScales: [2.0, 2.0]),
        false);
  });
  test('mac all-1x is logical (divide is a harmless no-op)', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false, isPeerMacOS: true, allDisplayScales: [1.0]),
        true);
  });
  test('legacy scale-0 reports count as unscaled', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false,
            isPeerMacOS: true,
            allDisplayScales: [0.0, 2.0]),
        true);
  });
  test('empty display list is never logical', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false, isPeerMacOS: true, allDisplayScales: []),
        false);
  });
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cmd.exe /c 'cd /d D:\Projects\rustdesk\flutter && flutter test test\logical_display_layout_test.dart'`
Expected: FAIL — `Target of URI doesn't exist ... logical_display_layout.dart`

- [ ] **Step 4: Write the implementation**

`flutter/lib/common/logical_display_layout.dart`:

```dart
/// Decides whether the client lays out a peer's displays in logical units
/// (origins AND sizes both in points) instead of raw `DisplayInfo` units
/// (logical origins, physical sizes).
///
/// macOS peers report each display's origin in logical points
/// (`CGDisplayBounds`) but its size in physical pixels, and the host runs a
/// per-connection "Retina shim" (`Retina::on_mouse_event` in the host's
/// connection.rs) that divides incoming mouse coordinates by the CURRENT
/// display's scale when they land in its mixed-unit rect. The Rust client
/// keeps that shim permanently inert by parking the host's current display
/// on a scale-1 display (see src/display_park.rs) — but only when one
/// exists. Hence:
///
/// - macOS with at least one scale<=1 display: parking is guaranteed, the
///   shim stays inert, and logical coordinates pass through verbatim —
///   use logical layout.
/// - macOS with every display scaled: no parking spot; keep the legacy
///   info-space behaviour and rely on the armed shim (correct for the
///   common single-Retina-MacBook case).
/// - Linux (Wayland) peers: always logical (pre-existing behaviour).
///
/// This predicate MUST be evaluated over ALL of the peer's displays
/// (`pi.displays`), never a single window's subset: parking is a
/// per-connection state shared by every window of the session.
bool useLogicalDisplayLayout({
  required bool isPeerLinux,
  required bool isPeerMacOS,
  required Iterable<double> allDisplayScales,
}) {
  if (isPeerLinux) return true;
  if (!isPeerMacOS) return false;
  bool any = false;
  bool anyUnscaled = false;
  for (final s in allDisplayScales) {
    any = true;
    if (s <= 1.0) anyUnscaled = true;
  }
  return any && anyUnscaled;
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cmd.exe /c 'cd /d D:\Projects\rustdesk\flutter && flutter test test\logical_display_layout_test.dart'`
Expected: PASS (7 tests)

- [ ] **Step 6: Stage (do NOT commit)**

```bash
git add flutter/lib/common/logical_display_layout.dart flutter/test/logical_display_layout_test.dart
```

---

### Task 2: Wire the helper into the three existing unit-decision sites

**Files:**
- Modify: `flutter/lib/models/model.dart:191-233` (`_getDisplaysRect`)
- Modify: `flutter/lib/desktop/pages/remote_page.dart:1182-1196` (`_buildScrollAutoNonTextureRender`)
- Modify: `flutter/lib/desktop/pages/remote_page.dart:1206-1220` (`_BuildPaintTextureRender`)

**Interfaces:**
- Consumes: `useLogicalDisplayLayout(...)` from Task 1.
- Produces: no new interfaces; the three sites now agree on a session-global condition.

- [ ] **Step 1: Update `_getDisplaysRect` in model.dart**

Add the import at the top of `flutter/lib/models/model.dart` following its existing import style (it already imports e.g. `package:flutter_hbb/common.dart`; add `package:flutter_hbb/common/logical_display_layout.dart`).

Replace lines 195-213 (the comment block and condition — currently ending in `if (isPeerLinux || (isPeerMacOS && displays.length > 1)) { useDisplayScale = true; }`) with:

```dart
    // macOS reports each display's origin in logical points (CGDisplayBounds)
    // but its width/height in physical pixels. When the peer has a scale-1
    // display, the Rust side parks the host's current display there so its
    // Retina shim stays inert (src/display_park.rs), and this rect must then
    // be fully logical — divide sizes by `scale`. When every display is
    // scaled there is no parking spot and the legacy info-space rect is kept
    // (the armed shim divides for us). Linux keeps its unconditional
    // behaviour.
    //
    // NOTE: this rect is the cursor/input coordinate space. The video frame
    // is still delivered in physical pixels, so every site that sizes a
    // frame from `Display.width/height` must divide by `scale` to match —
    // see the `useLogicalDisplayLayout` call sites in
    // desktop/pages/remote_page.dart. The predicate runs on ALL peer
    // displays (`_pi.displays`), not the passed subset: parking is
    // per-connection state shared by every window.
    if (useLogicalDisplayLayout(
        isPeerLinux: isPeerLinux,
        isPeerMacOS: isPeerMacOS,
        allDisplayScales: _pi.displays.map((d) => d.scale))) {
      useDisplayScale = true;
    }
```

- [ ] **Step 2: Update `_buildScrollAutoNonTextureRender` in remote_page.dart**

Add the same import to `flutter/lib/desktop/pages/remote_page.dart`.

Replace the condition at lines 1185-1192 (comment + `if (widget.ffi.ffiModel.isPeerLinux || (widget.ffi.ffiModel.isPeerMacOS && displays.length > 1)) {`) with:

```dart
    // The canvas rect is in logical points for these peers (see
    // FfiModel._getDisplaysRect) but the decoded frame is in physical pixels,
    // so the painted size has to be divided back down by the display scale.
    // The condition must match _getDisplaysRect exactly — session-global, all
    // peer displays.
    final displays = widget.ffi.ffiModel.pi.getCurDisplays();
    if (useLogicalDisplayLayout(
        isPeerLinux: widget.ffi.ffiModel.isPeerLinux,
        isPeerMacOS: widget.ffi.ffiModel.isPeerMacOS,
        allDisplayScales:
            widget.ffi.ffiModel.pi.displays.map((d) => d.scale))) {
```

(The body — `if (displays.isNotEmpty) { sizeScale = s / displays[0].scale; }` — stays unchanged.)

- [ ] **Step 3: Update `_BuildPaintTextureRender` in remote_page.dart**

Replace the `isScaledPeer` assignment at lines 1215-1219 (comment + `final isScaledPeer = ffiModel.isPeerLinux || (ffiModel.isPeerMacOS && displays.length > 1);`) with:

```dart
    // See _buildScrollAutoNonTextureRender: the rect is logical, the texture
    // is physical, so its painted size must be divided by the display scale.
    // The condition must match _getDisplaysRect exactly — session-global, all
    // peer displays.
    final isScaledPeer = useLogicalDisplayLayout(
        isPeerLinux: ffiModel.isPeerLinux,
        isPeerMacOS: ffiModel.isPeerMacOS,
        allDisplayScales: ffiModel.pi.displays.map((d) => d.scale));
```

- [ ] **Step 4: Analyze**

Run: `cmd.exe /c 'cd /d D:\Projects\rustdesk\flutter && flutter analyze lib\models\model.dart lib\desktop\pages\remote_page.dart lib\common\logical_display_layout.dart'`
Expected: No issues (pre-existing warnings unrelated to these files are acceptable; zero NEW errors).

- [ ] **Step 5: Run the full Dart test suite**

Run: `cmd.exe /c 'cd /d D:\Projects\rustdesk\flutter && flutter test'`
Expected: all tests pass (same set as before this task plus the 7 new ones).

- [ ] **Step 6: Stage (do NOT commit)**

```bash
git add flutter/lib/models/model.dart flutter/lib/desktop/pages/remote_page.dart
```

---

### Task 3: Rust `display_park` module (TDD)

**Files:**
- Create: `src/display_park.rs`
- Modify: `src/lib.rs` (add `pub mod display_park;` next to the existing `pub mod modifier_remap;` entry)

**Interfaces:**
- Produces: `pub fn choose_park_display(displays: &[DisplayInfo]) -> Option<usize>` and `pub fn should_intercept_switch(displays: &[DisplayInfo], target: usize) -> bool` — consumed by Tasks 4 and 5. `DisplayInfo` is `hbb_common::message_proto::DisplayInfo` (fields used: `scale: f64`).

- [ ] **Step 1: Determine the cargo test invocation**

Read `D:\dev\setup\build-full.ps1` (WSL path `/mnt/d/dev/setup/build-full.ps1`) and copy its cargo feature flags. The test command below assumes `--features flutter`; adjust to match the script exactly.

- [ ] **Step 2: Write the module with failing tests**

`src/display_park.rs`:

```rust
//! Client-side compensation for the macOS host's per-connection Retina shim.
//!
//! The shim (`Retina::on_mouse_event` in the host's `connection.rs`, present
//! unchanged in RustDesk 1.4.9) divides incoming absolute mouse coordinates
//! by the CURRENT display's scale when they land inside that display's
//! mixed-unit rect (logical origin, physical extent). "Current" is the
//! connection's `display_idx`, moved ONLY by the `SwitchDisplay` message.
//! Measured consequences (see HANDOFF-cursor-hidpi.md, 2026-08-15): with the
//! current display parked on a scale-1 display the shim is inert and logical
//! coordinates pass through verbatim (fully correct); parked on a scaled
//! display, every logical coordinate to its right is halved and displays
//! beyond it become unreachable by ANY wire coordinate.
//!
//! Strategy ("logical everywhere + safe parking"):
//! - never send `SwitchDisplay` for a scaled display when a scale-1 display
//!   exists (`should_intercept_switch`); park at connect when needed
//!   (`choose_park_display`);
//! - the Dart layer sends logical coordinates whenever a scale-1 display
//!   exists (`useLogicalDisplayLayout` in
//!   flutter/lib/common/logical_display_layout.dart — keep the two
//!   predicates in sync).
//! When every display is scaled there is no parking spot; legacy behaviour
//! (info-space coordinates + armed shim) is kept.

use hbb_common::message_proto::DisplayInfo;

/// The display index the host's `display_idx` should be parked on, or `None`
/// when parking is unnecessary (no scaled display) or impossible (every
/// display scaled). Old hosts may report `scale == 0.0`; treat it as
/// unscaled, matching the shim's own `s > 1.0` guard.
pub fn choose_park_display(displays: &[DisplayInfo]) -> Option<usize> {
    if !displays.iter().any(|d| d.scale > 1.0) {
        return None;
    }
    displays.iter().position(|d| d.scale <= 1.0)
}

/// Whether a client-side `switch_display(target)` must be replaced by a
/// capture-only path so the host's `display_idx` stays on a scale-1 display.
pub fn should_intercept_switch(displays: &[DisplayInfo], target: usize) -> bool {
    let Some(d) = displays.get(target) else {
        return false;
    };
    d.scale > 1.0 && choose_park_display(displays).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn di(scale: f64) -> DisplayInfo {
        DisplayInfo {
            scale,
            ..Default::default()
        }
    }

    #[test]
    fn no_scaled_display_means_no_parking() {
        assert_eq!(choose_park_display(&[di(1.0), di(1.0)]), None);
        assert_eq!(choose_park_display(&[]), None);
    }

    #[test]
    fn parks_on_first_unscaled_display() {
        assert_eq!(choose_park_display(&[di(1.0), di(1.0), di(2.0)]), Some(0));
        assert_eq!(choose_park_display(&[di(2.0), di(1.0)]), Some(1));
        // Old hosts may report scale 0; it counts as unscaled.
        assert_eq!(choose_park_display(&[di(0.0), di(2.0)]), Some(0));
    }

    #[test]
    fn all_scaled_cannot_park() {
        assert_eq!(choose_park_display(&[di(2.0)]), None);
        assert_eq!(choose_park_display(&[di(2.0), di(2.0)]), None);
    }

    #[test]
    fn intercepts_only_scaled_targets_with_a_parking_spot() {
        let mixed = [di(1.0), di(2.0)];
        assert!(should_intercept_switch(&mixed, 1));
        assert!(!should_intercept_switch(&mixed, 0));
        let all_scaled = [di(2.0), di(2.0)];
        assert!(!should_intercept_switch(&all_scaled, 0));
        // Out-of-range target: never intercept.
        assert!(!should_intercept_switch(&mixed, 5));
    }
}
```

Add to `src/lib.rs`, next to the existing module declarations (find `pub mod modifier_remap;` and add below it):

```rust
pub mod display_park;
```

- [ ] **Step 3: Run the tests**

Run: `cmd.exe /c 'cd /d D:\Projects\rustdesk && cargo test --lib display_park --features flutter'` (features per Step 1)
Expected: 4 tests pass. (First run in a while may recompile dependencies — warm target dir exists, so no Kaspersky issue per project memory.)

- [ ] **Step 4: Stage (do NOT commit)**

```bash
git add src/display_park.rs src/lib.rs
```

---

### Task 4: Intercept arming switches in `Session::switch_display`

**Files:**
- Modify: `src/ui_session_interface.rs:786-806` (`Session::switch_display`)

**Interfaces:**
- Consumes: `crate::display_park::should_intercept_switch` (Task 3); existing `Session` methods `capture_displays(add, sub, set)` (line ~773), `try_change_init_resolution(display)` (line ~1502), and `refresh_video` (verify exact name/signature in Step 1).

- [ ] **Step 1: Verify the pieces this task assumes**

Run and eyeball:
- `rg -n "fn refresh_video" src/ui_session_interface.rs` — confirm a `pub fn refresh_video(&self, display: i32)` (or similar) exists; adjust the call below to its real signature.
- `rg -n '"Mac OS"' src/ flutter/lib/consts.dart | head` — confirm the macOS platform string literal used by Rust peer checks and that `kPeerPlatformMacOS == "Mac OS"`.
- `rg -n "fn use_texture_render" src/ | head` — confirm the texture-render predicate's path/visibility from `ui_session_interface.rs` (it is already called at line ~803 in this same function).
- `rg -n "fn peer_platform" src/ui_session_interface.rs` — confirm `Session::peer_platform()` exists.

- [ ] **Step 2: Add the intercept**

At the TOP of `pub fn switch_display(&self, display: i32)` (before the custom-resolution lookup), insert:

```rust
        // macOS peers with mixed display scales: never move the host's
        // `display_idx` onto a scaled display — its Retina shim would divide
        // the logical coordinates this client sends (src/display_park.rs).
        // Subscribe the display's video without switching instead. Only for
        // the texture-render flow: the legacy flow relies on the host-side
        // switch for video routing.
        if display >= 0 && use_texture_render() && self.peer_platform() == "Mac OS" {
            let (intercept, is_multi_ui) = {
                let lc = self.lc.read().unwrap();
                match lc.peer_info.as_ref() {
                    Some(pi) => (
                        crate::display_park::should_intercept_switch(
                            &pi.displays,
                            display as usize,
                        ),
                        crate::common::is_support_multi_ui_session(&pi.version),
                    ),
                    None => (false, false),
                }
            };
            if intercept && is_multi_ui {
                self.capture_displays(vec![display], vec![], vec![]);
                self.refresh_video(display);
                // A configured custom resolution normally rides on the
                // SwitchDisplay echo; apply it explicitly since none is sent.
                self.try_change_init_resolution(display);
                return;
            }
        }
```

Adjust `use_texture_render()` to its verified path (Step 1) and `refresh_video` to its verified signature. If `refresh_video` does not exist on `Session`, use the same call the Flutter layer uses (`src/flutter.rs:2290` area calls `s.refresh_video(*display)` — copy that method's real name).

- [ ] **Step 3: Compile-check and run tests**

Run: `cmd.exe /c 'cd /d D:\Projects\rustdesk && cargo check --features flutter'`
Expected: clean check, no new warnings about unused imports.
Run: `cmd.exe /c 'cd /d D:\Projects\rustdesk && cargo test --lib display_park --features flutter'`
Expected: 4 tests still pass.

- [ ] **Step 4: Stage (do NOT commit)**

```bash
git add src/ui_session_interface.rs
```

---

### Task 5: Park at connect in `handle_peer_info`

**Files:**
- Modify: `src/ui_session_interface.rs:1734-1790` (`impl Interface for Session<T>::handle_peer_info`)

**Interfaces:**
- Consumes: `crate::display_park::choose_park_display` (Task 3); `Session::switch_display` (Task 4 — a park target is scale-1, so the intercept never fires for it).

- [ ] **Step 1: Add the parking call**

Inside `fn handle_peer_info`, in the `else if !self.is_port_forward() && !self.is_terminal()` branch, immediately after the `self.set_display(...)` call (line ~1775), insert:

```rust
            // Park the host's `display_idx` on a scale-1 display when the
            // macOS peer mixes display scales and the connection starts on a
            // scaled one, so the host's Retina shim stays inert for the
            // logical coordinates this client sends (src/display_park.rs).
            // Multi-ui peers (>=1.2.4) ignore the resulting switch echo for
            // current-display purposes, so the UI does not flip.
            if pi.platform == "Mac OS"
                && crate::common::is_support_multi_ui_session(&pi.version)
            {
                if let Some(park) = crate::display_park::choose_park_display(&pi.displays) {
                    let cur = pi.current_display as usize;
                    let cur_scaled = pi
                        .displays
                        .get(cur)
                        .map(|d| d.scale > 1.0)
                        .unwrap_or(false);
                    if cur != park && cur_scaled {
                        self.switch_display(park as i32);
                    }
                }
            }
```

- [ ] **Step 2: Compile-check and test**

Run: `cmd.exe /c 'cd /d D:\Projects\rustdesk && cargo check --features flutter'`
Expected: clean.
Run: `cmd.exe /c 'cd /d D:\Projects\rustdesk && cargo test --lib display_park --features flutter'`
Expected: 4 tests pass.

- [ ] **Step 3: Stage (do NOT commit)**

```bash
git add src/ui_session_interface.rs
```

---

### Task 6: Full rebuild and deploy

**Files:** none (build artifacts only)

- [ ] **Step 1: Full build (Rust changed — Dart-only rebuild is NOT enough)**

Run the project's full build per memory (`D:\dev\setup\build-full.ps1`; read the script first for its expected working directory and arguments):
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File 'D:\dev\setup\build-full.ps1'`
Expected: build succeeds; `flutter/build/windows/x64/runner/Release/rustdesk.exe` timestamp updates. Allow several minutes; Kaspersky is quiet in the warm target dir (project memory).

- [ ] **Step 2: Restart the client**

Kill the running rustdesk.exe and start the freshly built one (or run `D:\dev\setup\dev-run.ps1` if that is its purpose — read it first). Confirm the process start time is after the build.

---

### Task 7: Sweep verification matrix (user-assisted)

**Files:** none. Uses the rig from `HANDOFF-cursor-hidpi.md`: `measure-cursor.sh` (session scratchpad), `focus-window.ps1`, `list-windows.ps1`, SSH `statsunnycare@192.168.1.117` + `/tmp/getpos`. The user arranges windows and display switches; never click inside a session canvas.

Pass criteria reference: parked-state slope on a maximized 2752-wide combined window was measured at ≈2.132 uniformly; single-display windows have their own scale — the criterion in every scenario is a **single uniform slope across the whole sweep** (no mid-sweep slope break) and **reachability of the far edge** of the viewed area.

- [ ] **Scenario 1 — the user's actual bug:** only the middle (2x) display's per-display window open. Sweep full width. Expect: uniform slope, remote x spans [1920, 4672) (never past 4672), remote y stays within [24, 1176).
- [ ] **Scenario 2 — all three per-display windows open:** sweep each window (user re-focuses between sweeps or use focus-window.ps1). Expect: uniform slope in each; portrait window reaches [4672, 5872); no window influenced by which other windows are open or which was opened last.
- [ ] **Scenario 3 — in-tab single view of display 3, after viewing display 1:** expect uniform slope over [1920, 4672) (this is the intercepted-switch path).
- [ ] **Scenario 4 — combined view, both orders (display 3 → combined, display 1 → combined):** expect identical uniform ≈2.132 slope both times — the order-dependence measured on 2026-08-15 must be gone.
- [ ] **Scenario 5 — regression, toolbar resolution menu:** with display 3 viewed single, open the resolution submenu. Note whether it still lists resolutions (the SwitchDisplay echo that used to carry `SupportedResolutions` is suppressed on this path — degradation is possible; record what is seen, fix in a follow-up if the user cares).
- [ ] Record all results in `HANDOFF-cursor-hidpi.md`.

---

### Task 8: Documentation close-out

**Files:**
- Modify: `HANDOFF-cursor-hidpi.md`

- [ ] **Step 1: Update the handoff**

Mark the fix design as implemented; record the sweep matrix results; keep the known limitations list: (a) all-displays-scaled macOS peers unfixable client-side (host shim fix needed upstream — future PR); (b) mobile client gap pre-existing and untouched; (c) possible resolution-menu degradation on scaled displays (Scenario 5 outcome); (d) Rule A parking may leave the parking display's video subscribed on hosts >1.2.4 until a `CaptureDisplays` set/sub tidies it — only reachable when the peer's primary display is scaled, not the current setup.

- [ ] **Step 2: Stage (do NOT commit)**

```bash
git add HANDOFF-cursor-hidpi.md docs/superpowers/plans/2026-08-15-macos-retina-shim-cursor-fix.md
```
