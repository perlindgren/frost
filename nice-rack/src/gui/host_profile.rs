//! The rack's own share of its time, in the file `FROST_PROFILE` names.
//!
//! frost's profile times the frame it drives, and the heaviest thing the rack
//! does does not happen in a frame: plugin editor windows are serviced,
//! captured and pumped from the idle hook, which sits outside the frame's
//! span. A hook call that takes three quarters of a second — one editor
//! opening, with the plugin building its whole GUI on the UI thread — is
//! invisible to a frame profile whose frames are five milliseconds, which is
//! exactly the state of a rack that "updates fine" and still feels dead. So
//! the hook's three calls are timed here and appended to the same report on
//! the same cadence, and one read of one file says where the time is.
//!
//! Off unless `FROST_PROFILE` names a file, like the frost side of it.

use std::io::Write;
use std::path::PathBuf;

/// How long a report window is, matched to frost's so the lines interleave.
const REPORT_MS: f64 = 2_000.0;

/// Servicing the plugin windows: what an editor opening costs lives here.
pub(super) const SERVICE: usize = 0;
/// Pulling the editors' rendered frames in for the rack to show.
pub(super) const CAPTURE: usize = 1;
/// The rack's own per-tick work: meters, persistence, the audio side's calls.
pub(super) const TICK: usize = 2;
/// A window event handed to the editor windows. Dragging an editor reaches
/// `capture` from here, once per move message and so outside every frame.
pub(super) const EVENT: usize = 3;

/// The parts, in the order the hook calls them.
const PARTS: [(usize, &str); 4] = [
    (SERVICE, "service"),
    (CAPTURE, "capture"),
    (TICK, "tick   "),
    (EVENT, "event  "),
];

/// Milliseconds since the epoch, sub-millisecond part kept: what is being
/// looked for is a call of hundreds, but the ones that are not it will be
/// fractions, and fractions of a truncated millisecond all read as zero.
pub(super) fn millis() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}

/// The hook's parts since the last report.
pub(super) struct HostProfile {
    /// Where summaries are appended — the same file frost is writing to.
    path: PathBuf,
    /// Milliseconds and worst call per part.
    sums: [f64; 4],
    maxes: [f64; 4],
    /// Calls per part; the hook's three are called together, so the first is
    /// the count for the line.
    counts: [u32; 4],
    /// When this report window opened.
    opened: f64,
}

impl HostProfile {
    /// A profile writing beside frost's, or none at all.
    pub(super) fn from_env() -> Option<Self> {
        let path = std::env::var_os("FROST_PROFILE").map(PathBuf::from)?;
        if path.as_os_str().is_empty() {
            return None;
        }
        Some(Self {
            path,
            sums: [0.0; 4],
            maxes: [0.0; 4],
            counts: [0; 4],
            opened: millis(),
        })
    }

    /// One part of one hook call. The report goes out when the window is up,
    /// which is on the last part so all three of a call are in it.
    pub(super) fn note(&mut self, part: usize, ms: f64, now: f64) {
        self.sums[part] += ms;
        self.counts[part] += 1;
        self.maxes[part] = self.maxes[part].max(ms);
        if part == TICK && now - self.opened >= REPORT_MS {
            self.report(now);
        }
    }

    /// Append the window's line and start a new one.
    fn report(&mut self, now: f64) {
        let span = (now - self.opened).max(1.0) / 1000.0;
        let calls = self.counts[SERVICE].max(1) as f64;
        let mut line = format!("--- hook: {} calls in {span:.2} s,", self.counts[SERVICE],);
        for (part, label) in PARTS {
            line.push_str(&format!(
                " {label} avg {:.1} max {:.1} ms,",
                self.sums[part] / calls,
                self.maxes[part],
            ));
        }
        line.truncate(line.len() - 1);
        line.push('\n');
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(line.as_bytes());
        }
        self.sums = [0.0; 4];
        self.maxes = [0.0; 4];
        self.counts = [0; 4];
        self.opened = now;
    }
}
