//! Client-side control of the macOS host's per-connection Retina shim.
//!
//! `DisplayInfo` from a macOS host is mixed-unit: `x`/`y` are logical points
//! (`CGDisplayBounds`) while `width`/`height` are physical pixels. The host
//! compensates in `Retina::on_mouse_event` (`src/server/connection.rs`),
//! which divides an incoming absolute coordinate by the scale of the
//! connection's CURRENT display when the coordinate lands inside that
//! display's mixed-unit rect. "Current" is `display_idx`, moved only by the
//! `SwitchDisplay` message.
//!
//! Sweeping every possible wire coordinate against a shim armed on display
//! `A` (logical origin `O`, physical size `P`, scale `s`) gives:
//!
//! ```text
//! guard G = [O, O+P)              (A's logical rect blown up by s)
//! inside G:  out = O + (in-O)/s   -> covers exactly A's logical rect
//! outside G: out = in             -> covers exactly the complement of G
//! ```
//!
//! so the reachable set is `logical(A) | !G`, and every point in
//! `G \ logical(A)` is unreachable by ANY wire coordinate. That is why
//! pre-inverting the shim cannot work while the armed display is treated as
//! fixed, and why parking `display_idx` on a scale-1 display (the previous
//! strategy, `display_park.rs`) had no answer when every display was scaled.
//!
//! The armed display is not fixed, though — the client owns it. Arming the
//! display the pointer is currently over makes every point reachable
//! exactly, in every configuration:
//!
//! ```text
//! arm D, send W = O_D + (T - O_D) * s_D   =>  shim returns exactly T
//! ```
//!
//! `W` is the info-space coordinate, always inside `G_D`. For `s == 1` it
//! degenerates to `W == T` against an inert shim, so one rule covers scaled
//! and unscaled displays alike and the client can lay its canvas out in
//! logical points unconditionally.
//!
//! Ordering is safe: `MouseEvent` and `Misc::SwitchDisplay` are arms of the
//! same `match` in the host's single `async fn on_message`, and
//! `switch_display_to` assigns `display_idx` synchronously, so a switch sent
//! before a move is always applied first.

use hbb_common::message_proto::DisplayInfo;

/// The shim only ever divides by a scale greater than 1; treat anything else
/// (including the `0.0` old hosts report) as unscaled, matching its `s > 1.0`
/// guard.
#[inline]
fn effective_scale(d: &DisplayInfo) -> f64 {
    if d.scale > 1.0 {
        d.scale
    } else {
        1.0
    }
}

/// Size of a display in logical points. `DisplayInfo` ships physical pixels.
#[inline]
fn logical_size(d: &DisplayInfo) -> (f64, f64) {
    let s = effective_scale(d);
    (d.width as f64 / s, d.height as f64 / s)
}

/// Index of the display whose LOGICAL rect contains the logical point, or
/// `None` for a point in a gap between displays or off the desktop.
pub fn display_at_logical_point(displays: &[DisplayInfo], x: i32, y: i32) -> Option<usize> {
    displays.iter().position(|d| {
        let (w, h) = logical_size(d);
        let (x, y) = (x as f64, y as f64);
        x >= d.x as f64 && y >= d.y as f64 && x < d.x as f64 + w && y < d.y as f64 + h
    })
}

/// Convert a logical point on `d` into the info-space coordinate the shim
/// maps back to it. Always lands inside the shim's guard for `d`.
pub fn to_info_space(d: &DisplayInfo, x: i32, y: i32) -> (i32, i32) {
    let s = effective_scale(d);
    if s == 1.0 {
        return (x, y);
    }
    (
        d.x + ((x - d.x) as f64 * s).round() as i32,
        d.y + ((y - d.y) as f64 * s).round() as i32,
    )
}

/// Convert a cursor position the host reported back into logical points.
///
/// `Retina::on_cursor_pos` is the same mapping in reverse: it multiplies by
/// the armed display's scale whenever the host's own pointer sits on that
/// display, so with a scaled display armed the client receives info-space
/// positions while its canvas is laid out in logical points.
///
/// The reported value carries no unit tag, so this inverts exactly when the
/// shim would have converted: a scaled armed display and a value inside its
/// guard. A host pointer resting on a DIFFERENT display whose logical
/// position happens to fall inside the armed display's blown-up guard is
/// indistinguishable and would be divided wrongly — cosmetic only (it moves
/// the drawn remote cursor, not the pointer), and it cannot happen while the
/// armed display is the one being pointed at.
pub fn from_info_space(d: &DisplayInfo, x: i32, y: i32) -> (i32, i32) {
    let s = effective_scale(d);
    if s == 1.0 {
        return (x, y);
    }
    let inside = x >= d.x && y >= d.y && x < d.x + d.width && y < d.y + d.height;
    if !inside {
        return (x, y);
    }
    (
        d.x + ((x - d.x) as f64 / s).round() as i32,
        d.y + ((y - d.y) as f64 / s).round() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A macOS-style `DisplayInfo`: logical origin, physical size.
    fn di(x: i32, y: i32, logical_w: i32, logical_h: i32, scale: f64) -> DisplayInfo {
        let s = if scale > 1.0 { scale } else { 1.0 };
        DisplayInfo {
            x,
            y,
            width: (logical_w as f64 * s) as i32,
            height: (logical_h as f64 * s) as i32,
            scale,
            ..Default::default()
        }
    }

    /// The layout that produced the bug: every display scaled, so the old
    /// `choose_park_display` had nowhere to park.
    fn all_scaled() -> Vec<DisplayInfo> {
        vec![
            di(0, 0, 1728, 1117, 2.0),
            di(4480, -402, 1200, 1920, 2.0),
            di(1728, 0, 2752, 1152, 2.0),
        ]
    }

    /// What the host's shim does to a coordinate, armed on `current`.
    fn shim(displays: &[DisplayInfo], current: usize, x: i32, y: i32) -> (i32, i32) {
        let d = &displays[current];
        let s = d.scale;
        if s > 1.0 && x >= d.x && y >= d.y && x < d.x + d.width && y < d.y + d.height {
            (
                d.x + ((x - d.x) as f64 / s) as i32,
                d.y + ((y - d.y) as f64 / s) as i32,
            )
        } else {
            (x, y)
        }
    }

    #[test]
    fn finds_the_display_under_a_logical_point() {
        let ds = all_scaled();
        assert_eq!(display_at_logical_point(&ds, 100, 100), Some(0));
        assert_eq!(display_at_logical_point(&ds, 2000, 600), Some(2));
        assert_eq!(display_at_logical_point(&ds, 5000, 1000), Some(1));
        // Logical rects, not the physical extents: 3400 is past display 0's
        // logical right edge (1728) even though its `width` says 3456.
        assert_eq!(display_at_logical_point(&ds, 3400, 100), Some(2));
    }

    #[test]
    fn point_outside_every_display_has_no_home() {
        let ds = all_scaled();
        assert_eq!(display_at_logical_point(&ds, -5, 0), None);
        assert_eq!(display_at_logical_point(&ds, 9999, 9999), None);
        assert_eq!(display_at_logical_point(&[], 0, 0), None);
    }

    #[test]
    fn round_trips_through_the_shim_on_every_display() {
        let ds = all_scaled();
        for (idx, d) in ds.iter().enumerate() {
            let (lw, lh) = logical_size(d);
            for &(dx, dy) in &[(0.0, 0.0), (1.0, 1.0), (lw / 2.0, lh / 2.0), (lw - 1.0, lh - 1.0)] {
                let (tx, ty) = (d.x + dx as i32, d.y + dy as i32);
                let (wx, wy) = to_info_space(d, tx, ty);
                assert_eq!(
                    shim(&ds, idx, wx, wy),
                    (tx, ty),
                    "display {idx} target ({tx},{ty}) sent as ({wx},{wy})"
                );
            }
        }
    }

    /// The reported bug: armed on display 0, the left part of display 2 is
    /// dragged onto display 0. Arming display 2 instead lands it exactly.
    #[test]
    fn arming_the_wrong_display_is_what_moved_the_pointer() {
        let ds = all_scaled();
        let (target_x, target_y) = (2000, 600);
        let d2 = &ds[2];
        let (wx, wy) = to_info_space(d2, target_x, target_y);

        let wrong = shim(&ds, 0, wx, wy);
        assert_eq!(display_at_logical_point(&ds, wrong.0, wrong.1), Some(0));
        assert_ne!(wrong, (target_x, target_y));

        assert_eq!(shim(&ds, 2, wx, wy), (target_x, target_y));
    }

    #[test]
    fn unscaled_displays_pass_through_untouched() {
        let ds = vec![di(0, 0, 1920, 1200, 1.0), di(1920, 0, 2752, 1152, 2.0)];
        assert_eq!(to_info_space(&ds[0], 500, 500), (500, 500));
        // Old hosts report scale 0.0 for unscaled displays.
        let legacy = di(0, 0, 1920, 1200, 0.0);
        assert_eq!(to_info_space(&legacy, 500, 500), (500, 500));
        assert_eq!(from_info_space(&legacy, 500, 500), (500, 500));
    }

    #[test]
    fn cursor_feedback_inverts_what_the_shim_sent() {
        let d = di(1728, 0, 2752, 1152, 2.0);
        // Host logical 2000,600 -> shim reports info space -> client divides back.
        let reported = (d.x + (2000 - d.x) * 2, d.y + (600 - d.y) * 2);
        assert_eq!(from_info_space(&d, reported.0, reported.1), (2000, 600));
        // A value outside the guard is already logical and must be left alone.
        assert_eq!(from_info_space(&d, 9000, 600), (9000, 600));
    }
}
