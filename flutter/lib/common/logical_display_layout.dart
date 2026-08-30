/// Decides whether the client lays out a peer's displays in logical units
/// (origins AND sizes both in points) instead of raw `DisplayInfo` units
/// (logical origins, physical sizes).
///
/// macOS peers report each display's origin in logical points
/// (`CGDisplayBounds`) but its size in physical pixels, so a multi-display
/// canvas built from raw `DisplayInfo` lays a physical width against the next
/// display's logical origin and the rects overlap. Logical units are the only
/// self-consistent basis for the canvas, and the Rust client converts each
/// outgoing absolute coordinate into the info space of the display it lands
/// on, arming the host's Retina shim for that display (src/retina_shim.rs).
/// That works for any mix of display scales, so no display-count or
/// scale-mix gate is needed here.
///
/// - macOS peers: always logical.
/// - Linux (Wayland) peers: always logical (pre-existing behaviour).
/// - Everything else: raw `DisplayInfo` units, which are already logical.
bool useLogicalDisplayLayout({
  required bool isPeerLinux,
  required bool isPeerMacOS,
}) {
  return isPeerLinux || isPeerMacOS;
}
