//! The status band's log: the fifty-message history and the
//! docked panel's geometry.
// The subjects this one reads.
use crate::ron_panels::*;
use crate::strip::*;

/// The work area's center height above the window's bottom-edge frame:
/// the work area spans from the strip's top edge to the window's top, so
/// its center sits half a strip above the bottom — whatever the height,
/// since the strip's height is fixed.
/// The status band's height: the window's bottommost row belongs to
/// the log bar, and everything else stands on it — the window's floor
/// for the rest of the tool is [`floor_y`].
pub(crate) const STATUS_H: f32 = 26.0;

/// The log panel's row pitch and padding, its text inset, and the
/// vertical thumb's strip: the panel is four rows of the sidecar
/// tree's own rhythm, docked.
pub(crate) const LOG_ROWS: usize = 4;

pub(crate) const LOG_KEEP: usize = 50;

pub(crate) const LOG_LINE: f32 = RON_LINE;

pub(crate) const LOG_PAD: f32 = 6.0;

pub(crate) const LOG_INSET: f32 = 12.0;

pub(crate) const LOG_TRACK_W: f32 = 12.0;

/// Both thumbs' thickness, so the sideways one weighs the same as
/// the up-and-down one: one widget, two directions.
pub(crate) const LOG_THICK: f32 = 6.0;

/// The status band's rect, and the open panel's: the band spans the
/// window's bottom edge, and the panel docks on the work area's floor
/// just above it — clear of the slots, over the desk's lower rows.
pub(crate) fn log_bar_rect(w: f32, h: f32) -> [f32; 4] {
    [-w / 2.0, -h / 2.0, w / 2.0, floor_y(h)]
}

pub(crate) fn log_panel_rect(w: f32, h: f32) -> [f32; 4] {
    let bottom = floor_y(h) + STRIP_H;
    [
        -w / 2.0,
        bottom,
        w / 2.0,
        bottom + LOG_ROWS as f32 * LOG_LINE + 2.0 * LOG_PAD,
    ]
}

/// The panel's row region: the plate inside its padding, with the
/// thumb's strip reserved at the right.
pub(crate) fn log_body(panel: [f32; 4]) -> [f32; 4] {
    [
        panel[0],
        panel[1] + LOG_PAD,
        panel[2] - LOG_TRACK_W,
        panel[3] - LOG_PAD,
    ]
}

/// Capture one frame into the log: a message is an event when the
/// status changed, and the log records changes, not frames. Repeats
/// collapse, silence records nothing, and the oldest message dies at
/// `LOG_KEEP`.
pub(crate) fn log_note(log: &mut Vec<String>, seen: &mut String, now: &str) {
    if now.is_empty() || now == seen {
        return;
    }
    *seen = now.to_string();
    if log.last().map(String::as_str) != Some(now) {
        log.push(now.to_string());
    }
    while log.len() > LOG_KEEP {
        log.remove(0);
    }
}

/// The panel shows the log newest first: display row `i` is the
/// `i`th-newest message.
pub(crate) fn log_line(log: &[String], display: usize) -> Option<&str> {
    log.len().checked_sub(display + 1).map(|i| log[i].as_str())
}

/// Keep the highlighted row inside the shown window with the least
/// motion of the window.
pub(crate) fn log_reveal(len: usize, top: usize, sel: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let sel = sel.min(len - 1);
    let top = top.min(len.saturating_sub(LOG_ROWS));
    if sel < top {
        sel
    } else if sel >= top + LOG_ROWS {
        sel + 1 - LOG_ROWS
    } else {
        top
    }
}

/// The row under a window point in the open panel: the shown window
/// decided by `top`, the newest message at the panel's top.
pub(crate) fn log_row_at(len: usize, top: usize, body: [f32; 4], y: f32) -> Option<usize> {
    if y > body[3] || y < body[1] {
        return None;
    }
    let row = ((body[3] - y) / LOG_LINE).floor().max(0.0) as usize;
    let row = top + row;
    (row < len && row < top + LOG_ROWS).then_some(row)
}

/// The vertical thumb's `(offset from the track's top, length)`: a
/// log that fits owns the whole track — there is nowhere to go.
pub(crate) fn log_v_thumb(len: usize, track_h: f32, top: usize) -> (f32, f32) {
    if len <= LOG_ROWS {
        return (0.0, track_h);
    }
    let kh = (track_h * LOG_ROWS as f32 / len as f32).max(16.0);
    (top as f32 / (len - LOG_ROWS) as f32 * (track_h - kh), kh)
}

/// The window top a vertical thumb drag asks for: `y` is the
/// pointer's distance below the track's top, less the grab.
pub(crate) fn log_top_at(len: usize, track_h: f32, y: f32) -> usize {
    if len <= LOG_ROWS {
        return 0;
    }
    let (_o, kh) = log_v_thumb(len, track_h, 0);
    let f = (y / (track_h - kh).max(1.0)).clamp(0.0, 1.0);
    (f * (len - LOG_ROWS) as f32).round() as usize
}

/// The log's widest line in characters: FiraCode's advance is
/// constant, so the panel's content width is pure arithmetic.
pub(crate) fn log_wide(log: &[String]) -> usize {
    log.iter().map(|l| l.chars().count()).max().unwrap_or(0)
}

/// The horizontal thumb's `(offset from the track's left, length)`
/// over `content_w` of content in a `view_w` window.
pub(crate) fn log_h_thumb(view_w: f32, content_w: f32, track_w: f32, sx: f32) -> (f32, f32) {
    let over = content_w - view_w;
    if over <= 0.0 {
        return (0.0, track_w);
    }
    let kw = (track_w * (view_w / content_w)).max(16.0);
    ((sx / over).min(1.0) * (track_w - kw), kw)
}

/// The x offset a horizontal thumb drag asks for.
pub(crate) fn log_sx_at(view_w: f32, content_w: f32, track_w: f32, x: f32) -> f32 {
    let over = (content_w - view_w).max(0.0);
    if over == 0.0 {
        return 0.0;
    }
    let (_o, kw) = log_h_thumb(view_w, content_w, track_w, 0.0);
    (x / (track_w - kw).max(1.0)).clamp(0.0, 1.0) * over
}

/// The band's one message: the newest status, with its head kept and
/// its tail cut to `room` characters when the view name needs space.
pub(crate) fn log_head(msg: &str, room: usize) -> String {
    let chars = msg.chars().count();
    if chars <= room {
        return msg.to_string();
    }
    if room <= 3 {
        return ".".repeat(room);
    }
    msg.chars().take(room - 3).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_records_changes_not_frames() {
        let mut log = Vec::new();
        let mut seen = String::new();
        log_note(&mut log, &mut seen, ""); // silence records nothing
        log_note(&mut log, &mut seen, "opened brick.png");
        log_note(&mut log, &mut seen, "opened brick.png"); // same frame, same voice
        assert_eq!(log, vec!["opened brick.png".to_string()]);
        log_note(&mut log, &mut seen, "cropped");
        log_note(&mut log, &mut seen, "opened brick.png"); // back again: a new event
        assert_eq!(log.len(), 3);
        assert_eq!(log.last().unwrap(), "opened brick.png");
    }

    #[test]
    fn the_log_forgets_its_oldest_at_the_cap() {
        let mut log = Vec::new();
        let mut seen = String::new();
        for i in 0..LOG_KEEP + 10 {
            log_note(&mut log, &mut seen, &format!("message {i}"));
        }
        assert_eq!(log.len(), LOG_KEEP);
        assert_eq!(log.first().unwrap(), &format!("message {}", 10));
        assert_eq!(log.last().unwrap(), &format!("message {}", LOG_KEEP + 9));
    }

    #[test]
    fn the_panel_shows_the_newest_first() {
        let log: Vec<String> = ["one", "two", "three"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(log_line(&log, 0), Some("three"));
        assert_eq!(log_line(&log, 2), Some("one"));
        assert_eq!(log_line(&log, 3), None);
    }

    #[test]
    fn the_highlight_travels_and_pulls_the_window() {
        // A highlight inside the window moves nothing; one below the
        // foot slides it exactly onto the foot; one above the brow
        // snaps the brow down to it.
        let len = 10;
        assert_eq!(log_reveal(len, 2, 4), 2);
        assert_eq!(log_reveal(len, 2, 6), 3);
        assert_eq!(log_reveal(len, 4, 1), 1);
        // And the window never overpays its fare at either end.
        assert_eq!(log_reveal(len, 99, 0), 0);
        assert_eq!(log_reveal(2, 0, 1), 0);
    }

    #[test]
    fn the_rows_map_their_pixels_to_the_shown_window() {
        let body = [0.0, 0.0, 400.0, 4.0 * LOG_LINE];
        // With row 5 of the display at the brow, its middle is the
        // half-row below the top; the foot's last pixel still counts.
        assert_eq!(log_row_at(10, 5, body, body[3] - LOG_LINE / 2.0), Some(5));
        assert_eq!(
            log_row_at(10, 5, body, body[1] + 0.5),
            Some(5 + LOG_ROWS - 1)
        );
        // Outside the body, and past the last message: no row.
        assert_eq!(log_row_at(10, 5, body, body[3] + 1.0), None);
        assert_eq!(log_row_at(6, 5, body, body[3] - 3.5 * LOG_LINE), None);
    }

    #[test]
    fn the_vertical_thumb_is_proportional_and_round_trips() {
        // A log that fits owns the whole track; one twice as long as
        // the window takes half of it.
        assert_eq!(log_v_thumb(3, 80.0, 0), (0.0, 80.0));
        let (o, kh) = log_v_thumb(8, 80.0, 0);
        assert_eq!((o, kh), (0.0, 40.0));
        let (o, _) = log_v_thumb(8, 80.0, 2);
        assert!((o - 20.0).abs() < 1e-3);
        let (o, _) = log_v_thumb(8, 80.0, 4);
        assert!((o - 40.0).abs() < 1e-3); // the last window sits at the end
        // The drag reads the same arithmetic backwards, clamped.
        assert_eq!(log_top_at(8, 80.0, 0.0), 0);
        assert_eq!(log_top_at(8, 80.0, 20.0), 2);
        assert_eq!(log_top_at(8, 80.0, 40.0), 4);
        assert_eq!(log_top_at(8, 80.0, 999.0), 4);
    }

    #[test]
    fn the_horizontal_thumb_is_proportional_too() {
        assert_eq!(log_h_thumb(100.0, 80.0, 100.0, 5.0), (0.0, 100.0));
        let (o, kw) = log_h_thumb(100.0, 200.0, 100.0, 0.0);
        assert_eq!((o, kw), (0.0, 50.0));
        let (o, _) = log_h_thumb(100.0, 200.0, 100.0, 100.0);
        assert!((o - 50.0).abs() < 1e-3);
        // A dragged thumb asks for the offset it stands for, clamped.
        assert!((log_sx_at(100.0, 200.0, 100.0, 25.0) - 50.0).abs() < 1e-3);
        assert!((log_sx_at(100.0, 200.0, 100.0, -9.0) - 0.0).abs() < 1e-3);
        assert!((log_sx_at(100.0, 200.0, 100.0, 1_000.0) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn the_band_keeps_the_head_of_its_message() {
        // The head is the news; the tail is the ellipsis. A room too
        // small for letters is a room of dots.
        assert_eq!(log_head("cropped to 8 x 8", 40), "cropped to 8 x 8");
        assert_eq!(log_head("cropped to 8 x 8", 9), "croppe...");
        assert_eq!(log_head("cropped to 8 x 8", 2), "..");
    }

    #[test]
    fn the_panel_docks_on_the_work_floor_above_the_band() {
        let (w, h) = (800.0, 600.0);
        let bar = log_bar_rect(w, h);
        assert_eq!(bar, [-400.0, -300.0, 400.0, floor_y(h)]);
        // The panel stands on the work area's floor — clear of the
        // slots, over the desk's lower rows — and is four rows tall.
        let panel = log_panel_rect(w, h);
        assert_eq!(panel[1], floor_y(h) + STRIP_H);
        assert_eq!(
            panel[3] - panel[1],
            LOG_ROWS as f32 * LOG_LINE + 2.0 * LOG_PAD
        );
    }
}
