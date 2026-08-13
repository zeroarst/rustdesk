# Per-display taskbar window order — design

Date: 2026-08-12
Status: approved (design), pending implementation
Feature branch: per-display-window-memory

## Problem

With "Use all my displays for the remote session" enabled, each remote
display opens in its own local window. Windows orders taskbar buttons by
window creation order, and offers no API to reorder buttons afterwards, so
the taskbar order is whatever order the windows were opened in. The user
wants to assign an explicit order per display (e.g. M1 → 1, M4 → 2), with
unranked displays last, and have the windows open — and therefore appear in
the taskbar — in that order. Any display may be first, including one other
than display 0.

## Design

### Storage & semantics

- New LOCAL flutter option per (peer, display):
  `taskbar-order-d<display>_<peerId>`, value `'1'`, `'2'`, … Absent means
  unranked. (Amended during implementation: originally a per-peer
  `ui_flutter` option, but every live session window holds its own
  PeerConfig snapshot and re-stores it wholesale on its own events — with
  one session per display, a peer-config rank written from any window is
  reverted by the next save of any other window's session. The local
  config has no such snapshot writers.)
- Open order = all displays sorted by `(rank, display index)`; unranked
  displays sort after all ranked ones, by display index. Duplicate ranks
  tie-break by display index. Ranks for displays that no longer exist are
  ignored.
- Applies only to the "use all my displays" multi-window flow. A changed
  order takes effect on the next connection; nothing reshuffles live.
- With no ranks set, behavior is identical to today: index order, display
  0 first.

### Toolbar UI

- Each session window's display-settings menu (same menu as "Close all
  windows of this session together") gains a **"Taskbar order"** submenu
  with entries `1…n` (n = remote display count) and `None`.
- The submenu reflects and edits the rank of the display the window is
  currently showing, via the per-peer option above.
- Two new translatable strings: "Taskbar order", "None". English fallback
  is acceptable initially, consistent with the branch's other new strings.

### Open flow (model.dart, tryUseAllMyDisplaysForTheRemoteSession)

1. Compute the ordered display list with a pure helper
   `computeDisplayOpenOrder(ranks, displayCount)`.
2. If the first display in the list differs from the display this window
   is showing, switch in place with `openMonitorInTheSameTab` (the same
   mechanism as the toolbar monitor selector), then restore that display's
   saved frame (`restoreWindowPosition(display: first)`), falling back to
   the blind screen mapping when no frame is saved.
3. Open the remaining displays with the existing sequential awaited loop,
   iterating the ordered list instead of index order. Display 0 opens in a
   normal monitor window when it is not first.
4. The existing transition-save suppression covers the extra in-place
   switch; no new suppression sites are expected.

### Edge handling

- Duplicate ranks: tie-break by display index (no validation UI).
- Fewer displays than ranks (display removed remotely): ignore the
  orphaned rank.
- Single display or the option "use all my displays" off: no behavior
  change.

### Testing

- Unit tests for `computeDisplayOpenOrder` in
  `flutter/test/window_transition_test.dart` (or sibling): empty ranks,
  full permutation, partial ranks, duplicate ranks, orphaned ranks.
- Live verification with `D:\dev\setup\rd-verify.ps1` measurements (window
  → monitor/state) plus visual taskbar-order check across
  connect / close-all / reconnect cycles, as done for the restore fixes.

## Out of scope

- Reading or preserving manual taskbar-button drag order (no Windows API).
- Live reordering of already-open windows.
- Ordering for view-camera windows.
