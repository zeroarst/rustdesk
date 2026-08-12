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
