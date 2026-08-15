# Handoff — HiDPI macOS cursor offset (2026-08-14/15)

Investigation log for the mixed-scale macOS cursor fix (committed with it).
Companion to `BRIEF-cursor-offset-hidpi-macos.md` (untracked), the original
Mac-side brief. **The brief's fix is wrong as written** — see below before
applying anything from it.

## Where it landed

Commit `6a39317e` on `modifier-key-remapping`, two files, four hunks:

- `flutter/lib/models/model.dart` — `isPeerMacOS` getter; `_getDisplaysRect`
  divides by `scale` when `isPeerLinux || (isPeerMacOS && displays.length > 1)`
- `flutter/lib/desktop/pages/remote_page.dart` — the same condition at the two
  texture-sizing sites (`_buildScrollAutoNonTextureRender`,
  `_BuildPaintTextureRender`)

Modifier-remap WIP is untouched and still staged.

## Ground truth (measured, not assumed)

Mac `192.168.1.117`, via `CGDisplayBounds` + `CGDisplayCopyDisplayMode`:

| id  | origin      | logical   | pixels    | scale |
|-----|-------------|-----------|-----------|-------|
| 1   | (0, 0)      | 1920x1200 | 1920x1200 | 1.0   |
| 118 | (1920, 24)  | 2752x1152 | 5504x2304 | 2.0   |
| 113 | (4672,-360) | 1200x1920 | 1200x1920 | 1.0   |

`4672 == 1920 + 2752` proves origins are laid out in **logical** width while
`DisplayInfo.width` ships **physical** pixels. Confirmed client-side too, in
`handle_peer_info`: `width: 5504` beside `original_resolution: 2752, scale: 2.0`.

## What the brief got wrong

Its diagnosis is right. Its fix patches only `_getDisplaysRect`, which fixes the
cursor and **regresses rendering** — the 2x display paints at double size and
cropped, because the same `Display.width` also sizes the video texture. Upstream
already solved this for Wayland and wired the compensation through all three
sites; the brief found one. Mobile (`flutter/lib/mobile/pages/remote_page.dart`,
`isPeerLinux`) has the same gap, left alone as out of scope.

## Why the condition is a display *count*

The unit mismatch only causes harm when several displays share one canvas, so a
physical width is laid out against the next display's logical origin. A
single-display rect has nothing to reconcile against and is self-consistent in
physical pixels — dividing it puts the pointer at half rate.

Reachability of each mode (from instrumented logging of `_getDisplaysRect`):

- combined "all displays" view (the wide `1 3 2` toolbar button):
  `currentDisplay == kAllDisplayValue (-1)` → `n=3` → **the only mode where the
  divide branch fires**
- one window per display, and single-display sessions: `n=1` → branch inert
- `globalDisplaysRect()` always passes `true` and always sees all displays; it
  only feeds the toolbar, and is not the cursor/canvas path

## Verification (host pointer read over SSH)

| mode | before | after |
|---|---|---|
| combined view, right of the 2x display | x=**7304** on a 5872-wide desktop | 2131..3883, linear, in bounds |
| single display @2x | 1.064 | 1.064 (unchanged) |
| single display @1x | 1.0000 | 1.0000 (unchanged) |

## Open, for tomorrow

1. **Combined view paints ~1.25x oversized** on this 125%-scaled Windows client
   (`dpr=1.25`, `kIgnoreDpi=true`) — user confirmed by eye. Relative proportions
   between displays are correct, so this is DPI handling, not units. Believed to
   predate the fix; **not verified as pre-existing**. UPDATE 2026-08-15: the
   "pointer cannot reach display 113" part of this item was the Retina shim
   (open item 2), now resolved — in shim-inert state A the pointer reaches all
   displays. Only the paint-size question remains here.
2. **Pointer movement halves per-display over the 2x display.** ROOT CAUSE
   FOUND AND MEASURED (2026-08-15) — see "Root cause of item 2" below. The
   per-display transform is on the **host**, in the Mac server's `Retina` shim
   (`src/server/connection.rs`). `input_model.dart:2028` IS the client's only
   conversion site after all; its logged values diverged from measurement over
   the 2x display because the host modified them afterwards. Confirmed by the
   differential sweep (results below): the halving is controlled entirely by
   which display was last viewed single before entering the combined view.
3. The commit's condition is **verified by measurement, not derived**. Do not
   send upstream until (2) is understood.

## Root cause of item 2 — found by code trace (2026-08-15)

The full move pipeline, every stage read end to end:

- Dart producers (hover/drag) → `handleMouse` → `processEventToPeer`
  (`input_model.dart:1811`) → `handlePointerDevicePos` →
  `_handlePointerDevicePos` (`input_model.dart:1992`, the :2028 site) — the
  **only** client-side conversion. `modify()` touches modifiers only.
- `flutter_ffi.rs session_send_mouse` (:1932) — parses JSON, passes x/y verbatim.
- `ui_session_interface.rs send_mouse` (:1148) → `client.rs send_mouse` (:3192)
  — verbatim (the trackpad-factor branch is non-flutter only).
- Wire → host `connection.rs` — **`Retina::on_mouse_event`
  (`src/server/connection.rs`, ~:6721) transforms the event** → `input_mouse` →
  `input_service.rs` `MOUSE_TYPE_MOVE` (:1143) → enigo
  `mouse_move_to` → CGEvent at the given point, verbatim, in **global logical**
  coordinates (`libs/enigo/src/macos/macos_impl.rs:496`).

The Retina shim (macOS host only):

```rust
let Some(d) = self.displays.get(current) else { return };  // current = display_idx
let s = d.scale;
if s > 1.0 && e.x >= d.x && ... && e.x < d.x + d.width {
    e.x = d.x + ((e.x - d.x) as f64 / s) as i32;           // physical → logical
```

Three properties combine into the bug:

1. **Mixed-unit guard**: `d.x` is a logical origin (1920) but `d.width` is
   physical (5504), so the guard covers x ∈ [1920, 7424) — which swallows the
   *entire* rest of the logical desktop (d118's [1920,4672) AND d113's
   [4672,5872)).
2. **Keyed on `display_idx`, not on the point**: it converts iff the point lands
   in the *currently subscribed* display's rect.
3. **`display_idx` is stale in the combined view**: entering "all displays"
   sends only `CaptureDisplays` (`src/flutter.rs:2250` — `switch_display` is
   called only when `value.len()==1`), and `SwitchDisplay(-1)` would be ignored
   (`connection.rs switch_display_to` rejects out-of-range). So `display_idx`
   stays at the last single display viewed (primary at connect).

Protocol contract this reveals: **the client is expected to send info-space
coordinates** (logical origins, physical extents — exactly what `DisplayInfo`
ships), and the host's Retina shim converts to logical for the current display.
The commit's client-side divide makes the client send logical coords, which the
host then divides *again* whenever `display_idx` happens to be the retina
display. `Retina::on_cursor_pos` is the same mapping in reverse (host logical →
info space, `*s`) feeding the client's remote-cursor drawing.

Retro-fit of every measured number (window scale 1/canvas.scale = 2.133,
display_idx = d118 in the after-run):

- d1 painted region: client x ∈ [0,1920) → outside guard → verbatim →
  slope **2.133** ✓
- d118 painted region: x ∈ [1920,4672) → divided → slope **1.066** ✓
- d113 painted region: x ∈ [4672,5872) → **still inside the guard** → divided
  into [3296, 3896) — measured sweep ceiling **3883** ✓, and d113 is
  mathematically unreachable (client would have to send x ≥ 7424) — a second,
  sufficient cause for open item 1's "cannot reach display 113", independent of
  the 1.25x paint size.
- before-fix verbatim **7304**: requires `s > 1.0` to be false, i.e. that run's
  `display_idx` was a 1x display (fresh connect → primary d1). The shim makes
  results **stateful**: same build, same sweep, different last-viewed display →
  different slopes.

**Differential sweep — MEASURED 2026-08-15, both predictions exact:**

Setup: peer confirmed RustDesk 1.4.9 (shim in the 1.4.9 tag is byte-identical
to this checkout's). Maximized combined-view window on the 2752-wide monitor,
sweep at local y=556, x 20→2730, 40 steps, `measure-cursor.sh`.

- State B (viewed display 3 = the 2x single, then combined): slope
  **2.132** over local x 20..900, breaking to **1.059–1.075** at exactly
  remote x=1920, continuing at half rate through BOTH the 2x and portrait
  regions (no second break — the mixed-unit guard [1920,7424) swallows
  everything right of 1920). Far edge reached remote x=3872 → portrait
  (starts 4672) unreachable. remote_y snapped 570→297 at the same step:
  24+(570−24)/2 = 297 — the shim divides y identically. Spot checks exact:
  local 968 → client 2065 → shim 1992 (observed 1992); local 2730 → client
  5825 → shim 3872 (observed 3872).
- State A (viewed display 1 = built-in 1x single, then combined): uniform
  **2.132** across the whole canvas, remote_y constant 570, far edge reached
  remote x=5825 — portrait fully reachable. Zero halving.

Same build, same view, same sweep — only the previously-viewed display
differs. Mechanism confirmed; open item 2 resolved. This also resolves the
"pointer cannot reach display 113" half of open item 1 (it's the shim state,
not DPI); whether the combined view *paints* 1.25x oversized remains a
separate, unverified question.

Display numbering corrected (via CGGetActiveDisplayList + CGDisplayIsBuiltin
over SSH): toolbar 1 = id 1 = the BUILT-IN panel at 1920x1200 @1x; toolbar 2 =
id 113 portrait; toolbar 3 = id 118 = the 2x 2752x1152 display, which is NOT
built-in — likely a BetterDisplay virtual HiDPI screen (its size matches the
Windows center monitor). Enumeration order [1, 113, 118] = the DisplayInfo
vector order = toolbar numbering (scrap uses CGGetOnlineDisplayList).

**Fix design constraint (proved by the numbers):** client-side pre-inversion
of the shim CANNOT fix state B. For a target on the portrait display
(x∈[4672,5872), y≥24): sending the value itself lands in the guard and gets
divided; sending the pre-multiplied value (≥7424) escapes the guard and
arrives verbatim — both wrong. While `display_idx` is a scaled display, those
points are unreachable by ANY wire coordinate. Therefore the client fix is
state management, not math: **on entering the combined view against a macOS
peer, first send a single-display switch to a scale-1.0 display** (parking
`display_idx` where the shim is inert — state A is measured 100% correct),
then capture all displays. If every peer display is scaled there is no safe
parking spot and the combined view stays partially broken against 1.4.9
hosts — document as a limitation; the real fix for that case is host-side
(fix the shim upstream: logical-unit guard, keyed on the point's display).

## Per-display windows: same root cause, opposite direction (2026-08-15)

The user's real workflow is one window per display, never the combined view.
Reported: 3-window mode fine, but the middle (2x) display's window has the
cursor off. Code trace explains it completely:

- Per-display windows JOIN one shared connection: `sessionAddExistedSync`
  (`model.dart:3895` → `flutter.rs:1306 insert_peer_session_id`). One
  connection ⇒ ONE host-side `display_idx` for all windows.
- Opening such a window sends only `capture_displays(add=[D])` + refresh
  (`flutter_ffi.rs:195`), NEVER `SwitchDisplay` ⇒ `display_idx` stays at the
  primary (display 1) from connect.
- Shim inert ⇒ the 2x window's info-space coords ([1920,7424)) are never
  divided ⇒ cursor overshoots up to 2x. Scale-1 windows unaffected
  (info-space == logical for them).
- With one shared `display_idx` NO state fixes all three windows under
  today's client behavior: armed-for-2x fixes the middle window but breaks
  the portrait window (its y≥24 region falls inside the 2x mixed guard);
  inert breaks the 2x window. Fundamental vs 1.4.9 hosts.
- User's own combined-view test triple-confirms the shim: park display 1 ✓
  fine, park display 2 ✓ fine, park display 3 ✗ cursor off.

## Fix status — BUILT AND VERIFIED (2026-08-15)

Implemented per `docs/superpowers/plans/2026-08-15-macos-retina-shim-cursor-fix.md`
(all staged, not committed):

- `flutter/lib/common/logical_display_layout.dart` + test —
  `useLogicalDisplayLayout` predicate (7 unit tests), wired into
  `_getDisplaysRect` and both texture-sizing sites in
  `desktop/pages/remote_page.dart`. Condition is session-global (all
  `pi.displays`), replacing the old `displays.length > 1` gate.
- `src/display_park.rs` + `src/lib.rs` — `choose_park_display` /
  `should_intercept_switch` (4 unit tests).
- `src/ui_session_interface.rs` — `Session::switch_display` CHASES any
  arming switch with an immediate re-parking switch (gated on texture
  render, macOS peer, multi-ui version, scaled target, parking spot
  exists). First design suppressed the arming switch entirely; that broke
  the resolution menu because the switch echo is what carries
  `SupportedResolutions`, so the switch is now sent normally and the shim
  is armed only for the instant between the two back-to-back messages.
  The flutter desktop layer prunes the park display's video subscription
  right afterwards (`check_remove_unused_displays`).
  `handle_peer_info` parks at connect when the session would start on a
  scaled display. Verified: `Session::switch_display` is the ONLY
  client-side `SwitchDisplay` sender, so nothing can arm the shim and
  stay armed.

Sweep verification matrix (peer 1.4.9, all measured):

1. Lone middle (2x) window — the user's original bug: slope uniform
   1.0625, remote x 2007→4650 within [1920,4672), y 585. FIXED (was 2x
   overshoot off-desktop).
2. All three per-display windows simultaneously: d3 1.0625 in-bounds; d1
   1.0000 exact ([0,1920)); d2 1.0000 exact (4750→5852 at y=588 — the
   points that were mathematically unreachable when armed). No
   order-dependence. This state was impossible before the fix.
3. In-tab switch d1→d3: 1.0625 uniform, identical to the parked values.
   Re-swept after the intercept was replaced by chase-repark — same values
   to the pixel, proving the chase re-parks (an armed shim would have read
   half-rate in this exact sweep).
4. Combined view both orders (d3→combined and d1→combined): identical
   uniform 2.132, far edge 5825, portrait reachable. This morning's
   order-dependent halving is gone.
5. Resolution menu: list still populates (SupportedResolutions arrives on
   capture start, not only on switch echo). Two menu regressions found by
   the user and fixed same-day:
   a. `scaledRect()` in remote_toolbar.dart divided the now-logical rect
      by scale again → highlight showed 1376x576 instead of 2752x1152.
      Fixed with the same `useLogicalDisplayLayout` predicate.
   b. `pi.resolutions` is a single global list, clobbered by whichever
      display-changed message came last; with parking suppressing most
      switch echoes the stale list stuck (d3's modes shown while viewing
      d1). Fixed in three rounds, all needed together:
      - `PeerInfo.resolutionsMap` per-display cache; the switch echo
        stores under ITS display; the desktop menu reads the map (legacy
        field kept in sync for mobile). No fallback to the legacy list —
        an unknown display shows an empty list rather than another
        display's modes.
      - The connect-time list is keyed by the event's `current_display`
        (the HOST's current display), not the window's own — a
        per-display window joining a session has already overwritten
        `_pi.currentDisplay`.
      - The suppress-style intercept was replaced by chase-repark (see
        above) because a suppressed switch produces NO echo at all, which
        left freshly-switched scaled displays with an empty (hidden)
        resolution menu.
      User-verified: D1↔D3↔D2 switching now shows each display's own
      list.

## Fix design — "logical everywhere + safe parking" (as built)

Client-side, works against unmodified 1.4.9 macOS hosts:

1. **Send logical coordinates in every mode** for macOS peers: extend the
   commit's divide-by-scale to single-display rects (condition becomes
   `isPeerLinux || isPeerMacOS`, no display-count gate) at the same three
   sites (rect + two texture-sizing). Input and rendering stay consistent;
   textures downsample to fit as before.
2. **Park `display_idx` on a scale-1 display, permanently**: after peer info
   (and after any user display switch that would arm the shim), send a
   `SwitchDisplay` to a scale-1 display WITHOUT changing the client's viewed
   display. With texture render on, `Session::switch_display` skips its
   `capture_displays(set=..)` teardown, so the parking switch is capture-safe.
   Suppress/redirect the arming switch in `flutter.rs:2254` (len==1 path)
   for scaled displays on macOS peers.
3. Cursor-position feedback: parked ⇒ `on_cursor_pos` inert ⇒ host sends
   logical positions ⇒ matches the logical client rect. Consistent.

Known risks / open points for the plan:
- The host answers any SwitchDisplay with a display-changed message
  (`make_display_changed_msg`); the client's handler must not flip the UI to
  the parking display. Find and neutralize that reaction for parking
  switches (implementation risk #1).
- Custom per-display resolutions ride on SwitchDisplay (w/h only set when
  configured — `ui_session_interface.rs:787`); a suppressed arming switch
  must apply them via explicit ChangeResolution instead.
- All-displays-scaled peers: no safe parking spot; document limitation. Real
  cure is host-side (fix the shim: logical guard, key on the point's
  display) — separate upstream PR; client workaround stays for old hosts.
- Mobile client has the same gap (noted before), out of scope here.

Verification: rerun the sweep matrix — middle window alone (broken today),
all three per-display windows, each single view, combined both parkings.

## Instrumentation recipe (first thing to redo)

Release builds have no usable console, so log to a file. Put this at top level
in `flutter/lib/models/model.dart` (needs `import 'dart:io' as dbgio;`), call it
from wherever you are testing, and **strip it before committing**:

```dart
String _dbgLast = '';
void dbgLog(String s) {
  if (s == _dbgLast) return;            // dedupe; call sites can be per-frame
  _dbgLast = s;
  try {
    final dir = dbgio.Platform.environment['APPDATA'];
    if (dir == null) return;
    dbgio.File('$dir\\RustDesk\\log\\cursordbg.log').writeAsStringSync(
        '${DateTime.now().toIso8601String()} $s\n', mode: dbgio.FileMode.append);
  } catch (_) {}
}
```

Rebuild is Dart-only (~80s): `cd flutter && flutter build windows --release`.
Log lands at `/mnt/c/Users/zeroa/AppData/Roaming/RustDesk/log/cursordbg.log`.

**Log the caller, not just the values.** Last time `_getDisplaysRect` was logged
without recording whether `displaysRect()` or `globalDisplaysRect()` called it,
and the two had to be told apart by `curDisp` (`-1` = `kAllDisplayValue` = the
combined view). Pass a tag.

For open item 2, the useful call site is wherever a mouse **move** is turned into
remote coordinates — log the incoming local x and the outgoing remote x in the
same line. `input_model.dart:2028` is NOT it: instrumenting there produced values
matching measurement only on 1x displays, and a single linear mapping cannot
produce the slope change observed across the canvas. Start by finding every
producer of a `MOUSE_TYPE_MOVE` event rather than assuming.

## Tooling built (all reusable)

- `D:\dev\setup\list-windows.ps1` — every visible window with process and rect
- `D:\dev\setup\focus-window.ps1 <x> <y>` — focus by rect, no click (a click
  would be forwarded to the remote host)
- `D:\dev\setup\grab-region.ps1 <out> <x> <y> <w> <h>` and `grab-all.ps1`
- `D:\dev\setup\click-at.ps1 <x> <y>` — synthesized left click (added
  2026-08-15; safe only on client chrome — over a session canvas it forwards)
- scratchpad `measure-cursor.sh <x0> <y> <x1> <n>` — sweeps the local pointer and
  reads the host pointer via `ssh mac /tmp/getpos`, printing per-segment slope
- Mac side: `/tmp/getpos` (compiled), `/tmp/displays.swift`. SSH key auth is set
  up from this machine; revoke by editing `~/.ssh/authorized_keys` on the Mac.

Measurement gotchas: each remote window only forwards input when focused; the
Windows Terminal at (3432,-256) overlaps the portrait RustDesk window and will
silently steal a sweep.
