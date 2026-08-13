# Per-Display Taskbar Window Order Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user assign an explicit taskbar order per remote display (rank 1..n, unranked last) that controls the order session windows open in the "use all my displays" flow — any display may be first.

**Architecture:** A pure ordering function sorts displays by stored rank; the connect flow (`tryUseAllMyDisplaysForTheRemoteSession`) already switches the session to display 0 explicitly, so it is generalized to switch to the rank-1 display instead, then opens the remaining displays sequentially in rank order. Ranks are stored one per-peer flutter option per display and edited from a radio group in the desktop toolbar's Display menu.

**Tech Stack:** Flutter 3.24.5 (Dart only, no Rust changes), existing per-peer option API (`bind.mainGetPeerFlutterOptionSync` / `bind.mainSetPeerFlutterOptionSync`), existing toolbar menu widgets (`TRadioMenu`, `RdoMenuButton`).

## Global Constraints

- **Never commit.** Stage with `git add` only; the user authorizes commits explicitly (their global rule overrides this plan template's commit steps).
- All commands run on the Windows side: `cmd.exe /c 'cd /d D:\Projects\rustdesk\flutter && set PUB_CACHE=D:\dev\pub-cache&& D:\dev\flutter\bin\flutter.bat <cmd>'` from WSL, `dangerouslyDisableSandbox: true`.
- New user-visible strings use `translate('...')` with English fallback; no translation files are edited.
- Option key prefix is exactly `taskbar-order-d` (rank stored as `'1'`..`'n'`; empty/absent = unranked).
- Rank semantics: sort by `(rank, display index)`; unranked after ranked; ranks for out-of-range displays ignored. No ranks set ⇒ behavior identical to current (index order, display 0 first).

---

### Task 1: Pure ordering function `computeDisplayOpenOrder`

**Files:**
- Modify: `flutter/lib/utils/window_transition.dart` (append at end)
- Test: `flutter/test/window_transition_test.dart` (append a new group)

**Interfaces:**
- Consumes: nothing.
- Produces: `List<int> computeDisplayOpenOrder(Map<int, int> ranks, int displayCount)` — returns every display index `0..displayCount-1` exactly once, ordered by `(rank, index)` with unranked displays last. Task 4 calls this.

- [ ] **Step 1: Write the failing tests** — append to `flutter/test/window_transition_test.dart` inside `main()`:

```dart
  group('computeDisplayOpenOrder', () {
    test('no ranks keeps index order', () {
      expect(computeDisplayOpenOrder({}, 4), [0, 1, 2, 3]);
    });

    test('full permutation follows ranks', () {
      // M1(d0)->3, M2(d1)->1, M3(d2)->4, M4(d3)->2
      expect(computeDisplayOpenOrder({0: 3, 1: 1, 2: 4, 3: 2}, 4),
          [1, 3, 0, 2]);
    });

    test('unranked displays go last in index order', () {
      expect(computeDisplayOpenOrder({3: 1}, 4), [3, 0, 1, 2]);
    });

    test('duplicate ranks tie-break by display index', () {
      expect(computeDisplayOpenOrder({1: 2, 2: 2}, 4), [1, 2, 0, 3]);
    });

    test('ranks for displays that no longer exist are ignored', () {
      expect(computeDisplayOpenOrder({7: 1}, 4), [0, 1, 2, 3]);
    });

    test('every display appears exactly once', () {
      final order = computeDisplayOpenOrder({2: 1, 0: 2}, 3);
      expect(order..sort(), [0, 1, 2]);
    });
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `flutter test test/window_transition_test.dart` (Windows-side wrapper from Global Constraints)
Expected: FAIL — compile error, `computeDisplayOpenOrder` not defined.

- [ ] **Step 3: Implement** — append to `flutter/lib/utils/window_transition.dart`:

```dart
/// Order in which a multi-display session opens its windows (and therefore
/// the taskbar button order): displays sorted by user-assigned rank, ties
/// and unranked displays by display index, unranked after ranked.
List<int> computeDisplayOpenOrder(Map<int, int> ranks, int displayCount) {
  final displays = List.generate(displayCount, (i) => i);
  int sortKey(int d) => ranks[d] ?? 1 << 30;
  displays.sort((a, b) {
    final byRank = sortKey(a).compareTo(sortKey(b));
    return byRank != 0 ? byRank : a.compareTo(b);
  });
  return displays;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `flutter test test/window_transition_test.dart`
Expected: PASS (all groups, including the pre-existing ones).

- [ ] **Step 5: Stage (do not commit)**

```bash
git add flutter/lib/utils/window_transition.dart flutter/test/window_transition_test.dart
```

---

### Task 2: Option key constant

**Files:**
- Modify: `flutter/lib/consts.dart` (next to `kOptionCloseAllWindowsTogether`, ~line 92)

**Interfaces:**
- Consumes: nothing.
- Produces: `const String kOptionTaskbarOrderPrefix = "taskbar-order-d";` — Tasks 3 and 4 build keys as `'$kOptionTaskbarOrderPrefix$display'`.

- [ ] **Step 1: Add the constant** — in `flutter/lib/consts.dart`, directly below the `kOptionCloseAllWindowsTogether` line:

```dart
// Peer flutter option prefix: taskbar order rank per display
// ('taskbar-order-d<display>' = '1'..'n'; empty/absent = unranked).
const String kOptionTaskbarOrderPrefix = "taskbar-order-d";
```

- [ ] **Step 2: Verify it compiles**

Run: `flutter analyze --no-pub` — expected: no new errors.

- [ ] **Step 3: Stage (do not commit)**

```bash
git add flutter/lib/consts.dart
```

---

### Task 3: Toolbar radio group "Taskbar order"

**Files:**
- Modify: `flutter/lib/common/widgets/toolbar.dart` (new function near `toolbarDisplayToggle`, ~line 1000)
- Modify: `flutter/lib/desktop/widgets/remote_toolbar.dart` (Display menu, near the `toolbarDisplayToggle` future-builder at ~line 1806)

**Interfaces:**
- Consumes: `kOptionTaskbarOrderPrefix` (Task 2).
- Produces: `Future<List<TRadioMenu<String>>> toolbarTaskbarOrder(BuildContext context, String id, FFI ffi)` — radio entries with values `''` (None) and `'1'..'n'`; used only by the desktop Display menu.

- [ ] **Step 1: Add the menu builder** — in `flutter/lib/common/widgets/toolbar.dart`, after `toolbarDisplayToggle`:

```dart
Future<List<TRadioMenu<String>>> toolbarTaskbarOrder(
    BuildContext context, String id, FFI ffi) async {
  final pi = ffi.ffiModel.pi;
  if (!pi.isSupportMultiDisplay || pi.displaysCount.value <= 1) {
    return [];
  }
  final display = pi.currentDisplay;
  if (display < 0 || display == kAllDisplayValue) {
    return [];
  }
  final key = '$kOptionTaskbarOrderPrefix$display';
  final groupValue = bind.mainGetPeerFlutterOptionSync(id: id, k: key);
  onChanged(String? value) {
    if (value == null) return;
    bind.mainSetPeerFlutterOptionSync(id: id, k: key, v: value);
  }

  return [
    TRadioMenu<String>(
        child: Text(translate('None')),
        value: '',
        groupValue: groupValue,
        onChanged: onChanged),
    for (var rank = 1; rank <= pi.displaysCount.value; rank++)
      TRadioMenu<String>(
          child: Text('$rank'),
          value: '$rank',
          groupValue: groupValue,
          onChanged: onChanged),
  ];
}
```

- [ ] **Step 2: Render it in the Display menu** — in `flutter/lib/desktop/widgets/remote_toolbar.dart`, locate the future-builder that consumes `toolbarDisplayToggle(context, id, ffi)` (~line 1806). Directly BEFORE that builder's widget in the surrounding `Column`/list, insert a section modeled on the `toolbarViewStyle` future-builder at ~line 1603:

```dart
      _futureBuilder(
        future: toolbarTaskbarOrder(context, id, ffi),
        hasData: (data) {
          final v = data as List<TRadioMenu<String>>;
          if (v.isEmpty) return Offstage();
          return Column(children: [
            Divider(),
            Padding(
              padding: EdgeInsets.symmetric(horizontal: 12.0),
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(translate('Taskbar order'),
                    style: Theme.of(context).textTheme.bodySmall),
              ),
            ),
            ...v.map((e) => RdoMenuButton<String>(
                value: e.value,
                groupValue: e.groupValue,
                onChanged: e.onChanged,
                child: e.child,
                ffi: ffi)),
          ]);
        },
      ),
```

Adapt the exact `_futureBuilder` / `RdoMenuButton` parameter shapes to the neighboring `toolbarViewStyle` usage in that file — copy the surrounding pattern verbatim (including any `dismissOnClicked`/`ffi` arguments the siblings pass) rather than inventing new ones.

- [ ] **Step 3: Verify it compiles**

Run: `flutter analyze --no-pub` — expected: no new errors.

- [ ] **Step 4: Stage (do not commit)**

```bash
git add flutter/lib/common/widgets/toolbar.dart flutter/lib/desktop/widgets/remote_toolbar.dart
```

---

### Task 4: Ordered open flow in `tryUseAllMyDisplaysForTheRemoteSession`

**Files:**
- Modify: `flutter/lib/models/model.dart:1544-1587` (the body of `tryUseAllMyDisplaysForTheRemoteSession` after the `screenRectList` guard)

**Interfaces:**
- Consumes: `computeDisplayOpenOrder` (Task 1), `kOptionTaskbarOrderPrefix` (Task 2).
- Produces: nothing new — behavior change only.

**Spec note:** the spec names `openMonitorInTheSameTab` for the in-place switch; this function already performs its own raw `bind.sessionSwitchDisplay(value: [0])` switch (the extra image-model cleanup in `openMonitorInTheSameTab` only matters when switching an actively rendering view). Generalizing the existing switch from `0` to the rank-1 display is the same mechanism with fewer moving parts.

- [ ] **Step 1: Replace the display-0-specific block.** Current code (after the `screenRectList.length <= 1` guard):

```dart
    // move to the first display and set fullscreen
    bind.sessionSwitchDisplay(
      isDesktop: isDesktop,
      sessionId: sessionId,
      value: Int32List.fromList([0]),
    );
    _pi.currentDisplay = 0;
    try {
      CurrentDisplayState.find(peerId).value = _pi.currentDisplay;
    } catch (e) {
      //
    }

    // If a frame was saved for a (peer, display) pair, restore it — position
    // and fullscreen/maximized state — instead of the default fullscreen on
    // the index-matched local screen.
    hasSavedDisplayFrame(int d) => bind
        .mainGetPeerFlutterOptionSync(
            id: peerId, k: perDisplayFrameKey(WindowType.RemoteDesktop, d))
        .isNotEmpty;

    if (hasSavedDisplayFrame(0)) {
      await restoreWindowPosition(WindowType.RemoteDesktop,
          windowId: stateGlobal.windowId, peerId: peerId, display: 0);
    } else {
      await tryMoveToScreenAndSetFullscreen(screenRectList[0]);
    }

    final length = _pi.displays.length < screenRectList.length
        ? _pi.displays.length
        : screenRectList.length;
    // Sequentially, so the windows are created in display order every time:
    // the taskbar lists buttons in creation order, and this keeps that order
    // stable across sessions. It also serializes the reuse of hidden windows.
    for (var i = 1; i < length; i++) {
      try {
        await openMonitorInNewTabOrWindow(i, peerId, _pi,
            screenRect: hasSavedDisplayFrame(i) ? null : screenRectList[i]);
      } catch (e) {
        // One display failing to open must not stop the remaining ones.
        debugPrint("Failed to open display $i in its own window: $e");
      }
    }
```

Replace with:

```dart
    final length = _pi.displays.length < screenRectList.length
        ? _pi.displays.length
        : screenRectList.length;

    // User-assigned taskbar order: this window shows the rank-1 display and
    // the rest open in rank order, so the taskbar button order matches the
    // ranks. No ranks stored keeps today's behavior (index order, d0 first).
    final ranks = <int, int>{};
    for (var i = 0; i < length; i++) {
      final rank = int.tryParse(bind.mainGetPeerFlutterOptionSync(
          id: peerId, k: '$kOptionTaskbarOrderPrefix$i'));
      if (rank != null) ranks[i] = rank;
    }
    final order = computeDisplayOpenOrder(ranks, length);
    final first = order.first;

    // Show the rank-1 display in this window (generalized from the previous
    // hardcoded display 0).
    bind.sessionSwitchDisplay(
      isDesktop: isDesktop,
      sessionId: sessionId,
      value: Int32List.fromList([first]),
    );
    _pi.currentDisplay = first;
    try {
      CurrentDisplayState.find(peerId).value = _pi.currentDisplay;
    } catch (e) {
      //
    }

    // If a frame was saved for a (peer, display) pair, restore it — position
    // and fullscreen/maximized state — instead of the default fullscreen on
    // the index-matched local screen.
    hasSavedDisplayFrame(int d) => bind
        .mainGetPeerFlutterOptionSync(
            id: peerId, k: perDisplayFrameKey(WindowType.RemoteDesktop, d))
        .isNotEmpty;

    if (hasSavedDisplayFrame(first)) {
      await restoreWindowPosition(WindowType.RemoteDesktop,
          windowId: stateGlobal.windowId, peerId: peerId, display: first);
    } else {
      await tryMoveToScreenAndSetFullscreen(screenRectList[first]);
    }

    // Sequentially, so the windows are created in rank order every time:
    // the taskbar lists buttons in creation order. It also serializes the
    // reuse of hidden windows.
    for (final i in order.skip(1)) {
      try {
        await openMonitorInNewTabOrWindow(i, peerId, _pi,
            screenRect: hasSavedDisplayFrame(i) ? null : screenRectList[i]);
      } catch (e) {
        // One display failing to open must not stop the remaining ones.
        debugPrint("Failed to open display $i in its own window: $e");
      }
    }
```

Add the imports at the top of `model.dart` if not present: `import 'package:flutter_hbb/utils/window_transition.dart';` (`consts.dart` is already imported).

- [ ] **Step 2: Verify compile + full test suite**

Run: `flutter analyze --no-pub` — expected: no new errors.
Run: `flutter test` — expected: all pass.

- [ ] **Step 3: Stage (do not commit)**

```bash
git add flutter/lib/models/model.dart
```

---

### Task 5: End-to-end verification (user in the loop)

**Files:** none (verification only).

- [ ] **Step 1: Launch the instrumented build**

Run `dev-run.ps1` in background, log to `D:\dev\setup\dev-run.log` (established pattern).

- [ ] **Step 2: User sets ranks via the new toolbar menu**

While connected: on M1's window set Taskbar order = 1, on M4's window set 2, leave M2/M3 as None. Wait ~2 s (option writes are immediate; frame saves debounced).

- [ ] **Step 3: Confirm stored options**

```bash
grep -a 'taskbar-order' /mnt/c/Users/zeroa/AppData/Roaming/RustDesk/config/peers/180824860.toml
```

Expected: `taskbar-order-d0 = '1'` and `taskbar-order-d3 = '2'`.

- [ ] **Step 4: Close-all, reconnect, measure**

User closes all (X) and reconnects without killing. Verify with
`powershell.exe -File D:\dev\setup\rd-verify.ps1`: all 4 windows on correct
monitors/states, AND user visually confirms taskbar order M1, M4, M2, M3.
Then a second cycle setting M4 = 1 (M4-first) to prove arbitrary order:
expected taskbar M4, M1, M2, M3 with M4's window being the connecting one.

- [ ] **Step 5: Ask the user for commit approval**

Everything staged; user decides commit message split (suggested: single commit `feat: user-defined taskbar order for multi-display session windows`).
