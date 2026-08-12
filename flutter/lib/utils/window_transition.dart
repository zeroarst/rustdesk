/// Helpers for programmatic window transitions (restore, reuse, teardown).
///
/// Reusing hidden windows raced: several concurrent session-open calls
/// could all pick the same hidden window before any of them marked it as
/// taken, so remaining displays piled up as tabs in one window. The pick
/// must reserve synchronously — no await between choosing and removing.

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
