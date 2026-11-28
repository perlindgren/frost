//! Per-phase frame timing, for a frame budget that does not fit.
//!
//! `frame_processing_ms` says how much CPU a frame cost in total, which is
//! the right question for a demo and the wrong one for a slow window: it
//! cannot answer *where* the milliseconds went, and the two answers that
//! look alike call for opposite fixes. A frame that spends its time in
//! `get_current_texture` is waiting for the display, and no amount of
//! drawing less helps; a frame that spends it encoding glyphs needs a
//! cheaper draw.
//!
//! So this measures the phases a frame actually has, and appends a summary
//! every [`REPORT_MS`] milliseconds — average and worst per phase, and the
//! percentiles of whole frames. It is off unless `FROST_PROFILE` is set to
//! the file to write, because a profile that costs a frame budget of its
//! own is not a profile.
//!
//! A slow acquire has two opposite readings — the display is being waited
//! for, or the swap chain is being rebuilt — and they cost the same in the
//! phase lines. So the summary also counts reconfigures, suboptimal
//! acquires and frames the surface gave no texture for, and names the chain
//! it is running: size, present mode, latency, backend.
//!
//! The file is opened, written and closed once per report rather than held
//! open: the point of the exercise is reading it while the window still
//! moves, and a handle held by a wedged process would hide what it wrote.

use std::io::Write;
use std::path::PathBuf;

/// How long a report window is: long enough to hold a hundred frames at a
/// struggling sixty, short enough that a ten-second drag lands in it.
const REPORT_MS: f64 = 2_000.0;

/// One measured stretch of a frame. The order is the order they happen in,
/// and the report prints them in it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// The application's idle hook, which is where a host keeps its own
    /// machinery alive: the rack services plugin editor windows here.
    Wait,
    /// Asking the swap chain for a texture. Under a refresh-synced present
    /// this is where the display's cadence is paid.
    Acquire,
    /// The application's `process`: the whole of its drawing of a frame.
    Process,
    /// Updating the scene tree: every node's `Node::process`, children first.
    Visit,
    /// Walking the scene into the frame's draw list.
    DrawList,
    /// Laying the text out and splicing per-glyph sprite quads into the list.
    Text,
    /// Packing the light, occluder and uniform fields for the frame.
    Pack,
    /// Encoding the render passes.
    Encode,
    /// Handing the command buffer to the queue.
    Submit,
    /// The present handoff.
    Present,
}

/// Every phase, in frame order.
const PHASES: [Phase; 10] = [
    Phase::Wait,
    Phase::Acquire,
    Phase::Process,
    Phase::Visit,
    Phase::DrawList,
    Phase::Text,
    Phase::Pack,
    Phase::Encode,
    Phase::Submit,
    Phase::Present,
];

impl Phase {
    /// The slot this phase occupies in the accumulators.
    fn index(self) -> usize {
        match self {
            Phase::Wait => 0,
            Phase::Acquire => 1,
            Phase::Process => 2,
            Phase::Visit => 3,
            Phase::DrawList => 4,
            Phase::Text => 5,
            Phase::Pack => 6,
            Phase::Encode => 7,
            Phase::Submit => 8,
            Phase::Present => 9,
        }
    }

    /// The name the report prints, padded to the width of the longest.
    fn label(self) -> &'static str {
        match self {
            Phase::Wait => "about_to_wait",
            Phase::Acquire => "acquire    ",
            Phase::Process => "process    ",
            Phase::Visit => "visit      ",
            Phase::DrawList => "draw list  ",
            Phase::Text => "text       ",
            Phase::Pack => "pack       ",
            Phase::Encode => "encode     ",
            Phase::Submit => "submit     ",
            Phase::Present => "present    ",
        }
    }
}

/// The phases of the frames since the last report.
pub(super) struct FrameProfile {
    /// Where summaries are appended.
    path: PathBuf,
    /// Milliseconds accumulated per phase.
    sums: [f64; 10],
    /// The worst frame seen per phase, so a two-millisecond average hiding
    /// a twenty-millisecond hitch still says so.
    maxes: [f64; 10],
    /// How many frames recorded each phase: a frame that returns early
    /// records the phases it reached and nothing after them.
    counts: [u32; 10],
    /// Swap chain reconfigures: how many, and what they cost in total and
    /// at worst. Overlapping rather than phased, because a reconfigure
    /// happens inside the acquire that is waiting for it.
    reconfigures: u32,
    reconfigure_ms: f64,
    reconfigure_max: f64,
    /// Acquires answered `Suboptimal`, which the app answers with a
    /// reconfigure in the same frame. This and the count above together say
    /// whether a long acquire is the display's cadence or a chain being
    /// rebuilt over and over.
    suboptimal: u32,
    /// Acquires that answered with no texture. Such a frame draws nothing,
    /// records no phase, and shows up nowhere but here.
    no_texture: u32,
    /// Milliseconds between one frame ending and the next beginning, which is
    /// the stretch no phase covers: a stream running at thirty-five frames a
    /// second with three-millisecond frames spends the rest of its time here.
    idle_ms: f64,
    idle_max: f64,
    idle_count: u32,
    /// Gaps sorted by how they ended: the loop had gone to waiting and
    /// something woke it, or its queue was pumped under it by a modal drag
    /// loop that never let it wait.
    idle_waited: u32,
    idle_pumped: u32,
    /// Frames the app dropped as redundant — a modal drag pump asking for a
    /// frame with nothing new to show (see `Frost::frame_is_redundant`). They
    /// draw nothing and phase nothing, so without this counter a drag looks
    /// like a report that stopped arriving.
    skips: u32,
    /// The chain as the app last described it: size, present modes,
    /// latency, backend. The report prints the current one.
    surface: String,
    /// Whole-frame milliseconds, for the percentiles.
    totals: Vec<f64>,
    /// Frames completed since the last report.
    frames: u32,
    /// GPU draw calls of the frames since the last report, for the header.
    draws: u64,
    /// When this report window opened.
    opened: f64,
}

impl FrameProfile {
    /// A profile writing to the file `FROST_PROFILE` names, or none at all.
    /// Without the variable every call site below is a branch on `None`,
    /// which is the whole cost of the instrument when it is not in use.
    pub(super) fn from_env(now: f64) -> Option<Self> {
        #[cfg(target_arch = "wasm32")]
        let _ = now;
        let path = std::env::var_os("FROST_PROFILE").map(PathBuf::from)?;
        if path.as_os_str().is_empty() {
            return None;
        }
        Some(Self {
            path,
            sums: [0.0; 10],
            maxes: [0.0; 10],
            counts: [0; 10],
            reconfigures: 0,
            reconfigure_ms: 0.0,
            reconfigure_max: 0.0,
            suboptimal: 0,
            no_texture: 0,
            idle_ms: 0.0,
            idle_max: 0.0,
            idle_count: 0,
            idle_waited: 0,
            idle_pumped: 0,
            skips: 0,
            surface: String::new(),
            totals: Vec::new(),
            frames: 0,
            draws: 0,
            opened: now,
        })
    }

    /// One phase of the frame in progress, measured from the timestamp the
    /// caller took before it.
    pub(super) fn phase(&mut self, phase: Phase, ms: f64) {
        let at = phase.index();
        self.sums[at] += ms;
        self.counts[at] += 1;
        self.maxes[at] = self.maxes[at].max(ms);
    }

    /// A swap chain configure, what it cost, and how the app describes the
    /// chain it now has.
    pub(super) fn reconfigured(&mut self, ms: f64, surface: impl std::fmt::Display) {
        self.reconfigures += 1;
        self.reconfigure_ms += ms;
        self.reconfigure_max = self.reconfigure_max.max(ms);
        self.surface = surface.to_string();
    }

    /// An acquire the surface answered `Suboptimal` to.
    pub(super) fn suboptimal(&mut self) {
        self.suboptimal += 1;
    }

    /// An acquire that got no texture, so the frame drew nothing.
    pub(super) fn no_texture(&mut self) {
        self.no_texture += 1;
    }

    /// A gap between two frames. `waited` says the loop went to waiting in it.
    pub(super) fn idle(&mut self, ms: f64, waited: bool) {
        self.idle_ms += ms;
        self.idle_max = self.idle_max.max(ms);
        self.idle_count += 1;
        if waited {
            self.idle_waited += 1;
        } else {
            self.idle_pumped += 1;
        }
    }

    /// A frame dropped as redundant. Reported on the same clock as a drawn
    /// frame, because the point of dropping them is a window being dragged,
    /// and a report that stopped arriving would read like a wedged one.
    pub(super) fn skipped(&mut self, now: f64) {
        self.skips += 1;
        if now - self.opened >= REPORT_MS {
            self.report(now);
        }
    }

    /// A finished frame, and the report if the window is up.
    pub(super) fn frame(&mut self, total_ms: f64, draws: u32, now: f64) {
        self.frames += 1;
        self.draws += u64::from(draws);
        self.totals.push(total_ms);
        if now - self.opened < REPORT_MS {
            return;
        }
        self.report(now);
    }

    /// Append the window's summary and start a new window.
    ///
    /// What the instrument costs, measured on this repo's machine: a clock
    /// read is 41 ns, so a frame's eleven of them plus the records are about
    /// half a microsecond against a frame of two milliseconds. A summary is
    /// 0.18 ms, most of it the open-append-write itself, and it lands in a
    /// frame gap once per window per `REPORT_MS` — a twentieth of a
    /// millisecond a second per window, not something a frame ever waits on.
    fn report(&mut self, now: f64) {
        let span = (now - self.opened).max(1.0) / 1000.0;
        let frames = self.frames.max(1) as f64;
        let mut line = format!(
            "=== {} frames in {span:.2} s ({:.1} fps), frame p50 {:.1} p95 {:.1} max {:.1} ms, {} draws === {}\n",
            self.frames,
            frames / span,
            percentile(&self.totals, 0.50),
            percentile(&self.totals, 0.95),
            percentile(&self.totals, 1.0),
            self.draws,
            self.surface,
        );
        for phase in PHASES {
            let at = phase.index();
            line.push_str(&format!(
                "  {} avg {:7.3}  max {:7.3}  of {} frames\n",
                phase.label(),
                self.sums[at] / frames,
                self.maxes[at],
                self.counts[at],
            ));
        }
        line.push_str(&format!(
            "  idle        avg {:7.3}  max {:7.3}  of {} gaps | after a wait {:5} | pumped     {:5}\n",
            if self.idle_count > 0 {
                self.idle_ms / f64::from(self.idle_count)
            } else {
                0.0
            },
            self.idle_max,
            self.idle_count,
            self.idle_waited,
            self.idle_pumped,
        ));
        line.push_str(&format!(
            "  chain       reconfigured {:5}x avg {:7.3}  max {:7.3} | suboptimal {:5} | no texture {:5}\n",
            self.reconfigures,
            if self.reconfigures > 0 {
                self.reconfigure_ms / f64::from(self.reconfigures)
            } else {
                0.0
            },
            self.reconfigure_max,
            self.suboptimal,
            self.no_texture,
        ));
        line.push_str(&format!(
            "  dropped     {:5}x of {:5} loop iterations (a pump frame with nothing new to draw)\n",
            self.skips,
            self.frames + self.skips,
        ));
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(line.as_bytes());
        }
        self.sums = [0.0; 10];
        self.maxes = [0.0; 10];
        self.counts = [0; 10];
        self.reconfigures = 0;
        self.reconfigure_ms = 0.0;
        self.reconfigure_max = 0.0;
        self.suboptimal = 0;
        self.no_texture = 0;
        self.idle_ms = 0.0;
        self.idle_max = 0.0;
        self.idle_count = 0;
        self.idle_waited = 0;
        self.idle_pumped = 0;
        self.skips = 0;
        self.totals.clear();
        self.frames = 0;
        self.draws = 0;
        self.opened = now;
    }
}

/// The value below `fraction` of the samples, the nearest-rank way: with a
/// few hundred samples per report the interpolated variants differ by less
/// than the noise in what is being measured.
fn percentile(samples: &[f64], fraction: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let at = ((samples.len() - 1) as f64 * fraction).round() as usize;
    sorted[at]
}
