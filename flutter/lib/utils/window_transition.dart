/// Helpers for programmatic window transitions (restore, reuse, teardown).
///
/// Two problems they solve, both found while debugging multi-window session
/// layouts (one window per remote display):
///
/// 1. Reusing hidden windows raced: several concurrent session-open calls
///    could all pick the same hidden window before any of them marked it as
///    taken, so remaining displays piled up as tabs in one window. The pick
///    must reserve synchronously — no await between choosing and removing.
///
/// 2. Programmatic transitions (setFrame → delayed maximize()/fullscreen,
///    exiting leftover fullscreen on window reuse) fire native move/resize
///    events indistinguishable from user actions. The frame-save listeners
///    then persist a transitional state — e.g. "maximized, not fullscreen"
///    mid-way through a fullscreen restore — clobbering the user's arranged
///    layout. Saves are suppressed while a transition is in flight, so only
///    user-initiated changes update the remembered layout.

/// Picks the first window of [orderedWindows] that is in [inactive] and
/// removes it from [inactive] before returning, so a concurrent caller can
/// never pick the same window. Returns null when none is available.
int? pickAndReserveInactiveWindow(
    List<int> orderedWindows, Set<int> inactive) {
  for (final windowId in orderedWindows) {
    if (inactive.remove(windowId)) {
      return windowId;
    }
  }
  return null;
}

/// Time-boxed suppression flag for frame saves during programmatic window
/// transitions. Per isolate: each window suppresses its own saves.
class WindowTransitionSuppressor {
  WindowTransitionSuppressor({DateTime Function()? clock})
      : _clock = clock ?? DateTime.now;

  final DateTime Function() _clock;
  DateTime? _until;

  /// Suppress saves for [duration] from now. A shorter new suppression never
  /// shrinks a longer one already in force.
  void suppressFor(Duration duration) {
    final until = _clock().add(duration);
    if (_until == null || until.isAfter(_until!)) {
      _until = until;
    }
  }

  bool get isActive => _until != null && _clock().isBefore(_until!);
}

/// Shared instance used by the window save/restore paths.
final windowTransitionSuppressor = WindowTransitionSuppressor();

/// LOCAL-config key scoped to one remote display of one peer. Per-display
/// settings deliberately avoid peer-config (ui_flutter) keys: every live
/// session window holds its own PeerConfig snapshot and re-stores it
/// wholesale on its own events, clobbering peer-config keys written after
/// connect — with one session per display, a value saved from any window
/// would be reverted by the next save of any other window's session.
String perDisplayLocalKey(String prefix, String peerId, int display) =>
    '$prefix-d${display}_$peerId';

/// Taskbar-order rank of one remote display ('1'..'n', empty = unranked).
String taskbarOrderKey(String peerId, int display) =>
    perDisplayLocalKey('taskbar-order', peerId, display);

/// Toolbar collapsed state of one window ('Y'/'N', empty = use the
/// per-peer default).
String toolbarCollapseKey(String peerId, int display) =>
    perDisplayLocalKey('toolbar-collapse', peerId, display);

/// Toolbar hidden state of one window ('Y'/'N', empty = use the per-peer
/// default).
String toolbarHideKey(String peerId, int display) =>
    perDisplayLocalKey('toolbar-hide', peerId, display);

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
