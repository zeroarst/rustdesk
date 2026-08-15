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
