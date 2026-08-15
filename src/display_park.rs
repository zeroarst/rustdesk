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
//! - a `SwitchDisplay` that arms a scaled display is sent normally (its echo
//!   carries the `SupportedResolutions` the resolution menu needs) and then
//!   immediately chased with a `SwitchDisplay` to a scale-1 display
//!   (`needs_repark_after_switch` / `choose_park_display`), so the shim is
//!   armed only for the instant between the two messages; park at connect
//!   when the session would start on a scaled display;
//! - the Dart layer sends logical coordinates whenever a scale-1 display
//!   exists (`useLogicalDisplayLayout` in
//!   flutter/lib/common/logical_display_layout.dart — keep the two
//!   predicates in sync).
//!
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

/// Whether a client-side `switch_display(target)` must be chased with a
/// re-parking switch so the host's `display_idx` returns to a scale-1
/// display.
pub fn needs_repark_after_switch(displays: &[DisplayInfo], target: usize) -> bool {
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
    fn reparks_only_scaled_targets_with_a_parking_spot() {
        let mixed = [di(1.0), di(2.0)];
        assert!(needs_repark_after_switch(&mixed, 1));
        assert!(!needs_repark_after_switch(&mixed, 0));
        let all_scaled = [di(2.0), di(2.0)];
        assert!(!needs_repark_after_switch(&all_scaled, 0));
        // Out-of-range target: never repark.
        assert!(!needs_repark_after_switch(&mixed, 5));
    }
}
