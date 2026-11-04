//! Safe Windows placement methods, available only with the `unsafe` feature.
//!
//! Native handles are borrowed for one call and never cached or exposed as
//! integers. The feature permits unsafe code in this wrapper, not its caller.
#![cfg_attr(not(feature = "unsafe"), forbid(unsafe_code))]
#![deny(
    unsafe_op_in_unsafe_fn,
    clippy::undocumented_unsafe_blocks,
    missing_docs
)]

#[cfg(all(windows, feature = "unsafe"))]
mod native;
#[cfg(all(windows, feature = "unsafe"))]
pub use native::{restore, top_left};

#[cfg(any(test, all(windows, feature = "unsafe")))]
fn centered(work: [i32; 4], window: [i32; 4]) -> Option<(i32, i32)> {
    let [left, top, right, bottom] = work.map(i64::from);
    let [wl, wt, wr, wb] = window.map(i64::from);
    if right < left || bottom < top || wr < wl || wb < wt {
        return None;
    }
    Some((
        i32::try_from(left + (right - left - (wr - wl)) / 2).ok()?,
        i32::try_from(top + (bottom - top - (wb - wt)) / 2).ok()?,
    ))
}

#[cfg(any(test, all(windows, feature = "unsafe")))]
fn title_probe(saved: (i32, i32), window: [i32; 4]) -> Option<(i32, i32)> {
    let width = i64::from(window[2]) - i64::from(window[0]);
    if width < 0 {
        return None;
    }
    Some((
        i32::try_from(i64::from(saved.0) + width / 2).ok()?,
        saved.1.checked_add(16)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_in_work_area_with_negative_monitor_coordinates() {
        assert_eq!(
            centered([-1920, 40, 0, 1080], [0, 0, 640, 400]),
            Some((-1280, 360))
        );
    }

    #[test]
    fn large_windows_and_coordinates_do_not_overflow() {
        assert_eq!(
            centered([0, 0, 100, 100], [0, 0, 200, 200]),
            Some((-50, -50))
        );
        assert_eq!(
            centered([i32::MIN, 0, i32::MAX, 100], [0, 0, 1, 1]),
            Some((-1, 49))
        );
        assert_eq!(title_probe((i32::MAX, 0), [0, 0, 100, 100]), None);
        assert_eq!(title_probe((0, i32::MAX), [0, 0, 100, 100]), None);
    }

    #[test]
    fn malformed_rectangles_are_rejected() {
        assert_eq!(centered([20, 0, 10, 100], [0, 0, 10, 10]), None);
        assert_eq!(centered([0, 0, 100, 100], [20, 0, 10, 10]), None);
    }

    #[test]
    fn probe_checks_the_saved_title_bar_center() {
        assert_eq!(
            title_probe((-800, 100), [50, 50, 690, 450]),
            Some((-480, 116))
        );
    }
}
