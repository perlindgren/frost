//! The analyser: what the picture under the sliders is made of.
//!
//! This is the half of the plugin that does not touch sound. It sits on the
//! editor's thread, holds the newest samples of both ends of the equaliser, and
//! answers one question in one shape: *how loud is each pixel column right
//! now*. Everything about that question is here — the transform lengths, the
//! window, the scaling, the frequency axis, and the reduction of thousands of
//! bins to the width of the widget — so that the drawing code can be about
//! drawing and the audio thread can stay free of all of it.
//!
//! Three shapes of the answer are worth naming before the code:
//!
//! - **One snapshot per tap, six sizes from it.** Each transform length reads
//!   its own tail of the same samples ([`Scope::All`]'s overlay is then six
//!   views of one instant, not six instants).
//! - **One value per pixel column, by maximum.** Sixteen thousand bins in six
//!   hundred pixels is a blur; taking the loudest bin per column keeps every
//!   partial the eye can resolve, and makes *bar* and *interpolated* two
//!   renderings of one array rather than two measurements.
//! - **Nothing computed that nobody asked for.** [`Scope::Off`] asks for no
//!   transform and the count in [`Analyser::computes`] says so; a test reads
//!   that count, because "off means no FFT at all" is a promise about cost.

use crate::dsp::Scope;
use realfft::RealFftPlanner;
use realfft::num_complex::Complex32;
use std::sync::Arc;

/// The left end of the frequency axis, in hertz. Fixed, whatever the sample
/// rate: a picture whose axis silently changes meaning when the audio
/// interface changes rate is worse than one that wastes some width at 44.1 kHz.
pub const FLO: f32 = 20.0;

/// The right end of the frequency axis. Above Nyquist there is nothing to
/// draw, and the axis says 20 kHz because that is the top of what the picture
/// is for.
pub const FHI: f32 = 20_000.0;

/// The bottom of the level axis, in dBFS. Below this a curve is a flat line at
/// the floor of the panel: the noise floor of a converter is a good deal higher
/// than the −144 dB a sine could measure, and a picture that tracks it would be
/// a picture of nothing.
pub const FLOOR_DB: f32 = -84.0;

/// The most recent samples the analyser keeps: the largest transform length, so
/// the longest window always has a full window to read. At 48 kHz that is
/// 0.68 s of signal and 128 KiB per tap.
pub const MAX_SAMPLES: usize = 1 << 15;

/// One transform length the selector offers.
///
/// The label names the transform length, not the bin count: a real FFT of N
/// samples yields N/2 + 1 magnitudes, so "1k" is a 1024-point transform with 513
/// bins. The README says so in words; this is where the arithmetic lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FftSize {
    S1024,
    S2048,
    S4096,
    S8192,
    S16384,
    S32768,
}

impl FftSize {
    /// Every length, smallest first: the order the overlay draws in, and the
    /// order the click-cycle moves through.
    pub const ALL: [FftSize; 6] = [
        FftSize::S1024,
        FftSize::S2048,
        FftSize::S4096,
        FftSize::S8192,
        FftSize::S16384,
        FftSize::S32768,
    ];

    /// Where the selector opens: 4096 samples is 11.7 Hz bins at 48 kHz, wide
    /// enough to see a chord and short enough to follow a riff.
    pub const DEFAULT: FftSize = FftSize::S4096;

    pub const fn len(self) -> usize {
        match self {
            FftSize::S1024 => 1024,
            FftSize::S2048 => 2048,
            FftSize::S4096 => 4096,
            FftSize::S8192 => 8192,
            FftSize::S16384 => 16384,
            FftSize::S32768 => 32768,
        }
    }

    /// Magnitudes out of the transform: DC through Nyquist.
    pub const fn bins(self) -> usize {
        self.len() / 2 + 1
    }

    pub const fn index(self) -> usize {
        match self {
            FftSize::S1024 => 0,
            FftSize::S2048 => 1,
            FftSize::S4096 => 2,
            FftSize::S8192 => 3,
            FftSize::S16384 => 4,
            FftSize::S32768 => 5,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            FftSize::S1024 => "1k",
            FftSize::S2048 => "2k",
            FftSize::S4096 => "4k",
            FftSize::S8192 => "8k",
            FftSize::S16384 => "16k",
            FftSize::S32768 => "32k",
        }
    }

    /// The next length to the right, wrapping: clicking the picture walks the
    /// selector round the ring rather than parking it at the top.
    pub fn next(self) -> FftSize {
        FftSize::ALL[(self.index() + 1) % FftSize::ALL.len()]
    }

    /// The next length to the left, wrapping — the wheel's other direction.
    pub fn prev(self) -> FftSize {
        FftSize::ALL[(self.index() + FftSize::ALL.len() - 1) % FftSize::ALL.len()]
    }

    /// Out of range reads as [`FftSize::DEFAULT`], the position the panel opens
    /// on. A setting file from a build with more positions must not open the
    /// analyser at a length it never asked for.
    pub fn from_param(value: i32) -> FftSize {
        match usize::try_from(value) {
            Ok(i) if i < FftSize::ALL.len() => FftSize::ALL[i],
            _ => FftSize::DEFAULT,
        }
    }

    pub const fn to_param(self) -> i32 {
        self.index() as i32
    }
}

/// How a column array becomes ink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// One bar per column, drawn from the floor up: where each measurement
    /// begins and ends.
    Bars,
    /// The same columns joined by straight segments: the contour, which is
    /// readable at widths where bars turn into a fence.
    Line,
}

impl Style {
    pub const ALL: [Style; 2] = [Style::Bars, Style::Line];

    pub fn from_param(value: i32) -> Style {
        match value {
            0 => Style::Bars,
            _ => Style::Line,
        }
    }

    pub const fn to_param(self) -> i32 {
        match self {
            Style::Bars => 0,
            Style::Line => 1,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Style::Bars => "BARS",
            Style::Line => "LINE",
        }
    }
}

/// Which end of the box a snapshot belongs to. [`Tap::Both`](crate::dsp::Tap)
/// fills two of these; the analyser itself never needs to know that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    In,
    Out,
}

impl Slot {
    pub const BOTH: [Slot; 2] = [Slot::In, Slot::Out];

    pub const fn index(self) -> usize {
        match self {
            Slot::In => 0,
            Slot::Out => 1,
        }
    }
}

/// The x of a frequency inside a strip `width` pixels wide starting at `left`,
/// and the inverse. One pair of functions for the whole plugin: the gridlines,
/// the band ticks, the overlay curves and the readout under the pointer all
/// agree because they all come through here.
pub fn hz_to_x(hz: f32, left: f32, width: f32) -> f32 {
    let t = (hz.max(FLO) / FLO).ln() / (FHI / FLO).ln();
    left + t.clamp(0.0, 1.0) * width
}

/// The frequency under a pixel. Outside the strip it reads the end of the
/// axis, which is what a pointer over the margin is nearest to.
pub fn x_to_hz(x: f32, left: f32, width: f32) -> f32 {
    let t = ((x - left) / width.max(1.0)).clamp(0.0, 1.0);
    FLO * (FHI / FLO).powf(t)
}

/// The sizes one [`Scope`] asks for, most-requested last, written into `out`;
/// returns how many were written.
///
/// This is a free function rather than a loop in the drawing code because it is
/// the whole of the off/top/all promise: `off` yields nothing, so nothing is
/// transformed, and the count is checked against [`Analyser::computes`] in a
/// test. `all` puts the selected length last so it is the line the eye keeps,
/// and the others underneath it.
pub fn sizes_for(
    scope: Scope,
    selected: FftSize,
    out: &mut [FftSize; FftSize::ALL.len()],
) -> usize {
    match scope {
        Scope::Off => 0,
        Scope::Top => {
            out[0] = selected;
            1
        }
        Scope::All => {
            let mut n = 0;
            for size in FftSize::ALL {
                if size != selected {
                    out[n] = size;
                    n += 1;
                }
            }
            out[n] = selected;
            n + 1
        }
    }
}

/// The newest samples of one signal, kept so that any window length can be
/// taken whole. Linear, not circular, because it is read backwards from the end
/// and shifted down in one memmove when it fills — a fraction of a millisecond
/// every two thirds of a second, on a thread that is drawing anyway.
struct Snapshot {
    data: Vec<f32>,
    len: usize,
}

impl Snapshot {
    fn new() -> Self {
        Self {
            data: vec![0.0; MAX_SAMPLES],
            len: 0,
        }
    }

    fn push(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        // A chunk bigger than the buffer is rare (a host flushing a file
        // position), and only its tail can still be the newest window.
        let fresh = if samples.len() > MAX_SAMPLES {
            &samples[samples.len() - MAX_SAMPLES..]
        } else {
            samples
        };
        if self.len + fresh.len() > MAX_SAMPLES {
            let keep = MAX_SAMPLES - fresh.len();
            let start = self.len - keep;
            self.data.copy_within(start..self.len, 0);
            self.len = keep;
        }
        self.data[self.len..self.len + fresh.len()].copy_from_slice(fresh);
        self.len += fresh.len();
    }

    /// The newest `n` samples, oldest first. Empty if the window has not
    /// filled yet, which is the honest answer to a question about a moment that
    /// has not happened.
    fn tail(&self, n: usize) -> &[f32] {
        if n > self.len {
            &[]
        } else {
            &self.data[self.len - n..self.len]
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// The analyser: six plans, two snapshots, and one measurement per pixel
/// column.
///
/// It owns its buffers so a redraw allocates nothing beyond the growth of an
/// output `Vec` that the caller keeps between frames. Building it costs six FFT
/// plans and about a megabyte, which is why it is built when an editor opens
/// and not when a plugin activates: close the window and the analysis is gone.
pub struct Analyser {
    plan: Vec<Arc<dyn realfft::RealToComplex<f32>>>,
    /// The Hann weights, one set per length: the same numbers every frame, so
    /// the multiplication is the only windowing cost there is.
    window: Vec<Vec<f32>>,
    /// The windowed time domain, reused: the transform eats its input.
    time: Vec<Vec<f32>>,
    /// The transform's output.
    spectrum: Vec<Vec<Complex32>>,
    /// rustfft's workspace, asked for once per plan instead of once per frame.
    scratch: Vec<Vec<Complex32>>,
    snapshot: [Snapshot; 2],
    /// Transforms run since this analyser existed. Not used by the picture; it
    /// exists so that a test can prove what `off` and `top` cost.
    pub computes: u64,
}

impl Default for Analyser {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyser {
    /// Plan every length, once. The planner is rustfft's, and it is the part
    /// that allocates — which is the whole reason this happens on the editor's
    /// thread, in [`Analyser::new`], and never on the audio thread.
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let mut plan = Vec::with_capacity(FftSize::ALL.len());
        let mut window = Vec::with_capacity(FftSize::ALL.len());
        let mut time = Vec::with_capacity(FftSize::ALL.len());
        let mut spectrum = Vec::with_capacity(FftSize::ALL.len());
        let mut scratch = Vec::with_capacity(FftSize::ALL.len());
        for size in FftSize::ALL {
            // `plan_fft_forward` on a real-to-complex planner: the forward
            // transform of reals, which is the only half of this library the
            // analyser has any use for. It hands back an `Arc`, so the plan is
            // shareable; we keep one handle per length and never build a plan
            // again.
            let fft = planner.plan_fft_forward(size.len());
            // Hann: −31 dB sidelobes and a 6 dB/octave skirt. Its coherent
            // gain of 0.5 is undone in `measure`, so a full-scale sine still
            // reads 0 dBFS rather than −6.
            let w: Vec<f32> = (0..size.len())
                .map(|i| {
                    0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / size.len() as f32).cos()
                })
                .collect();
            window.push(w);
            time.push(fft.make_input_vec());
            spectrum.push(fft.make_output_vec());
            scratch.push(fft.make_scratch_vec());
            plan.push(fft);
        }
        Self {
            plan,
            window,
            time,
            spectrum,
            scratch,
            snapshot: [Snapshot::new(), Snapshot::new()],
            computes: 0,
        }
    }

    /// Add samples to one tap's history. Called from the editor's drain loop,
    /// where the host's audio arrives through the ring the plugin feeds.
    ///
    /// Non-finite samples become silence: one NaN from a misbehaving plugin
    /// upstream would otherwise sit in the window and turn every frame of the
    /// rest of the picture into nothing.
    pub fn push(&mut self, slot: Slot, samples: &[f32]) {
        // The common case is all-finite, and it stays a memcpy.
        if samples.iter().any(|s| !s.is_finite()) {
            let clean: Vec<f32> = samples
                .iter()
                .map(|&s| if s.is_finite() { s } else { 0.0 })
                .collect();
            self.snapshot[slot.index()].push(&clean);
        } else {
            self.snapshot[slot.index()].push(samples);
        }
    }

    /// How many samples this tap holds: the drain loop's own bookkeeping, and
    /// enough for a "no signal yet" line on the panel.
    pub fn held(&self, slot: Slot) -> usize {
        self.snapshot[slot.index()].len
    }

    /// Whether a length has a full window of samples. Not enough history is not
    /// measured as silence and is not zero-padded — a padded window is a
    /// smeared window, and a first frame that arrives when the window really is
    /// full — 85 ms at the default length, two thirds of a second at the longest
    /// — is honest in a way a wrong one is not.
    pub fn ready(&self, slot: Slot, size: FftSize) -> bool {
        self.snapshot[slot.index()].len >= size.len()
    }

    /// Forget both histories: a rate change, a stream restart, a paused host.
    pub fn clear(&mut self) {
        for slot in Slot::BOTH {
            self.snapshot[slot.index()].clear();
        }
    }

    /// The newest window, transformed and reduced to `columns` values in dBFS,
    /// one per pixel column, floor at [`FLOOR_DB`]. Returns false when the
    /// window has not filled, leaving `out` empty.
    ///
    /// The reduction is the picture: bin `b` of a window of N at `rate` hertz
    /// stands at `b · rate / N`, and each column takes the loudest bin inside
    /// its own slice of the log axis — so a narrow bar keeps every partial
    /// instead of averaging the interesting ones away. A column finer than the
    /// bin spacing (the 20 Hz end of a 32 k window) takes the nearest bin
    /// instead of an empty one, which is the difference between a curve and a
    /// comb at the left of the panel.
    pub fn measure(
        &mut self,
        slot: Slot,
        size: FftSize,
        rate: f32,
        columns: usize,
        out: &mut Vec<f32>,
    ) -> bool {
        out.clear();
        if columns == 0 {
            return false;
        }
        let n = size.len();
        let tail = self.snapshot[slot.index()].tail(n);
        if tail.is_empty() {
            return false;
        }
        let i = size.index();
        if !rate.is_finite() || rate <= 0.0 {
            return false;
        }
        for (target, (&x, &w)) in self.time[i]
            .iter_mut()
            .zip(tail.iter().zip(self.window[i].iter()))
        {
            *target = x * w;
        }
        if self.plan[i]
            .process_with_scratch(
                &mut self.time[i],
                &mut self.spectrum[i],
                &mut self.scratch[i],
            )
            .is_err()
        {
            return false;
        }
        self.computes += 1;
        self.reduce(size, rate, columns, out);
        true
    }

    /// Bins to columns, in dBFS. Kept apart from `measure` because it is the
    /// part worth reading on its own, and the part a test can drive with a
    /// spectrum it built itself.
    fn reduce(&self, size: FftSize, rate: f32, columns: usize, out: &mut Vec<f32>) {
        let spectrum = &self.spectrum[size.index()];
        let bin_hz = rate / size.len() as f32;
        let last = spectrum.len() - 1;
        out.clear();
        out.reserve(columns);
        for column in 0..columns {
            // The column's own slice of the log axis, in bins.
            let lo_hz = FLO * (FHI / FLO).powf(column as f32 / columns as f32);
            let hi_hz = FLO * (FHI / FLO).powf((column + 1) as f32 / columns as f32);
            let lo = (lo_hz / bin_hz).ceil() as usize;
            let hi = ((hi_hz / bin_hz).floor() as usize).min(last);
            let (from, to) = if lo > hi {
                // Finer than one bin — the low end of a long window, where the
                // axis has more pixels than the transform has bins. The nearest
                // bin stands in for the column, so the picture stays a curve
                // instead of turning into a comb of empty columns.
                let mid = (((lo_hz + hi_hz) * 0.5) / bin_hz).round() as usize;
                let mid = mid.min(last);
                (mid, mid)
            } else {
                (lo.min(last), hi)
            };
            let mut best = FLOOR_DB;
            for bin in spectrum[from..=to].iter() {
                // 4|X|/N: the two of a one-sided spectrum, over the Hann
                // window's 0.5 coherent gain, over the length. A full-scale
                // sine therefore reads 0 dBFS at its own bin, which is the
                // assertion in this file's tests.
                let amplitude = 4.0 * bin.re.hypot(bin.im) / size.len() as f32;
                let db = 20.0 * amplitude.max(1e-9).log10();
                if db > best {
                    best = db;
                }
            }
            // The floor is clipped; the top is not. A signal that arrives over
            // 0 dBFS is the host's business, and a curve that says +2 dB is
            // telling the truth about it.
            out.push(best.max(FLOOR_DB));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;
    const COLUMNS: usize = 600;

    /// Feed one window's worth of a sine whose frequency lands exactly on a
    /// bin of `size`, so the measurement has no scalloping to argue about.
    fn tone(analyser: &mut Analyser, slot: Slot, size: FftSize, bin: usize, amplitude: f32) {
        let hz = bin as f32 * RATE / size.len() as f32;
        let n = size.len();
        let samples: Vec<f32> = (0..n)
            .map(|i| amplitude * (2.0 * std::f32::consts::PI * hz * i as f32 / RATE).sin())
            .collect();
        analyser.push(slot, &samples);
    }

    /// The value of the column a frequency lands in.
    fn column_of(out: &[f32], hz: f32) -> f32 {
        let t = (hz.max(FLO) / FLO).ln() / (FHI / FLO).ln();
        let i = ((t * COLUMNS as f32) as usize).min(out.len() - 1);
        out[i]
    }

    #[test]
    fn the_labels_name_the_transform_and_the_bins_follow() {
        let labels: Vec<&str> = FftSize::ALL.iter().map(|s| s.label()).collect();
        assert_eq!(labels, ["1k", "2k", "4k", "8k", "16k", "32k"]);
        for (index, size) in FftSize::ALL.iter().enumerate() {
            assert_eq!(size.index(), index);
            assert_eq!(size.len(), 1024 << index);
            // A real FFT of N samples answers N/2 + 1 frequencies, DC through
            // Nyquist: the label is the length, never the bin count.
            assert_eq!(size.bins(), size.len() / 2 + 1);
            assert_eq!(FftSize::from_param(size.to_param()), *size);
        }
        assert_eq!(FftSize::S32768.next(), FftSize::S1024, "the cycle wraps");
        assert_eq!(FftSize::S1024.prev(), FftSize::S32768);
        // A setting file from elsewhere must not open the analyser somewhere
        // nobody offered.
        assert_eq!(FftSize::from_param(-1), FftSize::DEFAULT);
        assert_eq!(FftSize::from_param(99), FftSize::DEFAULT);
    }

    #[test]
    fn off_asks_for_nothing_top_for_one_size_and_all_for_every_size() {
        let mut sizes = [FftSize::S1024; 6];
        assert_eq!(sizes_for(Scope::Off, FftSize::S4096, &mut sizes), 0);
        assert_eq!(sizes_for(Scope::Top, FftSize::S8192, &mut sizes), 1);
        assert_eq!(sizes[0], FftSize::S8192);
        let n = sizes_for(Scope::All, FftSize::S8192, &mut sizes);
        assert_eq!(n, 6);
        // Last, so the length the selector points at is the line on top.
        assert_eq!(sizes[5], FftSize::S8192);
        let mut seen: Vec<usize> = sizes[..n].iter().map(|s| s.index()).collect();
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2, 3, 4, 5], "every length once");
    }

    #[test]
    fn a_full_scale_sine_reads_zero_decibels_at_its_own_bin() {
        // The scaling claim, measured: 4|X|/N over a Hann window puts a
        // full-scale tone at 0 dBFS, and half of it at −6.02, with no
        // adjustment factor discovered later by eye.
        let size = FftSize::S4096;
        let mut analyser = Analyser::new();
        tone(&mut analyser, Slot::In, size, 100, 1.0);
        let mut out = Vec::new();
        assert!(analyser.measure(Slot::In, size, RATE, COLUMNS, &mut out));
        assert_eq!(out.len(), COLUMNS);
        let hz = 100.0 * RATE / size.len() as f32;
        let read = column_of(&out, hz);
        assert!(read.abs() < 0.1, "a full-scale sine read {read} dBFS");

        let mut quiet = Analyser::new();
        tone(&mut quiet, Slot::In, size, 100, 0.5);
        assert!(quiet.measure(Slot::In, size, RATE, COLUMNS, &mut out));
        let read = column_of(&out, hz);
        assert!(
            (read + 6.02).abs() < 0.1,
            "half amplitude read {read} dBFS, wants −6.02"
        );
    }

    #[test]
    fn a_column_holds_the_loudest_bin_in_it_and_not_the_average() {
        // Two tones a few bins apart, 20 dB apart: the reduction takes the
        // maximum, so the loud one reads as itself whether or not the quiet one
        // shares its column. This is also why the bar and the interpolated view
        // can share one array.
        let size = FftSize::S4096;
        let mut analyser = Analyser::new();
        let bin_hz = RATE / size.len() as f32;
        let mut mixed = vec![0.0f32; size.len()];
        for (i, sample) in mixed.iter_mut().enumerate() {
            let t = 2.0 * std::f32::consts::PI * i as f32 / RATE;
            *sample = (t * (100.0 * bin_hz)).sin() + 0.1 * (t * (103.0 * bin_hz)).sin();
        }
        analyser.push(Slot::In, &mixed);
        let mut out = Vec::new();
        assert!(analyser.measure(Slot::In, size, RATE, COLUMNS, &mut out));
        let read = column_of(&out, 100.0 * bin_hz);
        assert!(read.abs() < 1.5, "the loud tone read {read} dBFS, wants 0");
    }

    #[test]
    fn nothing_is_measured_before_the_window_has_filled() {
        let mut analyser = Analyser::new();
        let mut out = Vec::new();
        analyser.push(Slot::In, &[0.1; 500]);
        assert!(!analyser.ready(Slot::In, FftSize::S1024));
        assert!(!analyser.measure(Slot::In, FftSize::S1024, RATE, COLUMNS, &mut out));
        assert!(out.is_empty(), "a refused measurement leaves nothing drawn");
        assert_eq!(analyser.computes, 0, "a refused window is not a transform");
    }

    #[test]
    fn the_history_keeps_the_newest_window_whatever_the_chunk_sizes() {
        // 0.68 s of a tone, pushed in dribs the size a host's blocks arrive in,
        // must measure the same as the same samples pushed once.
        let size = FftSize::S4096;
        let hz = 100.0 * RATE / size.len() as f32;
        let total = 3 * size.len();
        let wave: Vec<f32> = (0..total)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / RATE).sin())
            .collect();

        let mut whole = Analyser::new();
        whole.push(Slot::In, &wave);
        let mut one = Vec::new();
        assert!(whole.measure(Slot::In, size, RATE, COLUMNS, &mut one));

        let mut dribs = Analyser::new();
        for chunk in wave.chunks(1000) {
            dribs.push(Slot::In, chunk);
        }
        let mut other = Vec::new();
        assert!(dribs.measure(Slot::In, size, RATE, COLUMNS, &mut other));

        let worst = one
            .iter()
            .zip(&other)
            .fold(0.0f32, |worst, (a, b)| worst.max((a - b).abs()));
        assert!(
            worst < 1e-3,
            "chunking changed the measurement by {worst} dB"
        );
    }

    #[test]
    fn history_past_the_buffer_keeps_the_newest_samples() {
        // Fill with silence, then a tone longer than the buffer can hold twice
        // over: what is left must be the tone, or the largest setting would
        // show a picture of a moment that ended a minute ago.
        let mut analyser = Analyser::new();
        let silence = [0.0f32; 4096];
        for _ in 0..10 {
            analyser.push(Slot::Out, &silence);
        }
        let size = FftSize::S32768;
        let hz = 200.0 * RATE / size.len() as f32;
        let wave: Vec<f32> = (0..MAX_SAMPLES + 9_000)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / RATE).sin())
            .collect();
        for chunk in wave.chunks(1009) {
            analyser.push(Slot::Out, chunk);
        }
        assert_eq!(analyser.held(Slot::Out), MAX_SAMPLES);
        let mut out = Vec::new();
        assert!(analyser.measure(Slot::Out, size, RATE, COLUMNS, &mut out));
        let read = column_of(&out, hz);
        assert!(
            read.abs() < 0.5,
            "the newest window read {read} dBFS at the tone"
        );
    }

    #[test]
    fn the_two_taps_are_measured_apart() {
        // The whole point of two histories: watching the output must not be
        // answered with the input, and an end nobody fed reads as silence
        // rather than as whatever the other end is doing.
        let size = FftSize::S4096;
        let mut analyser = Analyser::new();
        tone(&mut analyser, Slot::In, size, 100, 1.0);
        analyser.push(Slot::Out, &[0.0; 4096]);
        let mut out = Vec::new();
        assert!(analyser.measure(Slot::In, size, RATE, COLUMNS, &mut out));
        let hz = 100.0 * RATE / size.len() as f32;
        assert!(column_of(&out, hz).abs() < 0.1, "the input lost its tone");
        assert!(analyser.measure(Slot::Out, size, RATE, COLUMNS, &mut out));
        assert!(
            out.iter().all(|&v| v <= FLOOR_DB + 0.01),
            "the output shows something it was never given"
        );
    }

    #[test]
    fn the_scope_decides_the_transform_count_and_the_counter_says_so() {
        // "off performs no FFT at all", proved as cost and not as pixels.
        let size = FftSize::S4096;
        for (scope, wants) in [(Scope::Off, 0u64), (Scope::Top, 3), (Scope::All, 18)] {
            let mut analyser = Analyser::new();
            tone(&mut analyser, Slot::In, FftSize::S32768, 200, 1.0);
            let mut sizes = [FftSize::S1024; 6];
            let mut out = Vec::new();
            for _frame in 0..3 {
                let n = sizes_for(scope, size, &mut sizes);
                for &each in &sizes[..n] {
                    analyser.measure(Slot::In, each, RATE, COLUMNS, &mut out);
                }
            }
            assert_eq!(
                analyser.computes, wants,
                "{scope:?} ran {} transforms in three frames",
                analyser.computes
            );
        }
    }

    #[test]
    fn a_nan_is_silence_and_not_a_poisoned_picture() {
        let mut analyser = Analyser::new();
        let mut junk = vec![0.0f32; 4096];
        for (i, sample) in junk.iter_mut().enumerate() {
            *sample = if i % 97 == 0 { f32::NAN } else { 0.25 };
        }
        analyser.push(Slot::In, &junk);
        let mut out = Vec::new();
        assert!(analyser.measure(Slot::In, FftSize::S4096, RATE, COLUMNS, &mut out));
        assert!(
            out.iter().all(|v| v.is_finite()),
            "one bad sample took the whole picture with it"
        );
    }

    #[test]
    fn the_axis_maps_both_ways_and_stops_at_its_ends() {
        for width in [320.0f32, 600.0, 1400.0] {
            let left = -width / 2.0;
            for hz in [20.0, 50.0, 440.0, 1_000.0, 6_400.0, 20_000.0] {
                let back = x_to_hz(hz_to_x(hz, left, width), left, width);
                assert!(
                    (back / hz - 1.0).abs() < 1e-3,
                    "{hz} Hz round-tripped to {back}"
                );
            }
            assert_eq!(hz_to_x(5.0, left, width), left, "below the axis");
            assert_eq!(hz_to_x(90_000.0, left, width), left + width, "above it");
        }
        // Log, not linear: an octave is the same distance everywhere.
        let left = 0.0;
        let width = 600.0;
        let low = hz_to_x(100.0, left, width) - hz_to_x(50.0, left, width);
        let high = hz_to_x(12_800.0, left, width) - hz_to_x(6_400.0, left, width);
        assert!(
            (low - high).abs() < 0.01,
            "an octave is {low} px at the bottom and {high} px at the top"
        );
    }

    #[test]
    fn every_size_measures_the_window_it_claims_to() {
        // Each length has to work end to end, including the two that bracket
        // the useful range: 1 k shows nothing below 47 Hz, and 32 k needs two
        // thirds of a second of history to say anything at all.
        let mut out = Vec::new();
        for size in FftSize::ALL {
            let mut analyser = Analyser::new();
            tone(&mut analyser, Slot::In, size, 40, 1.0);
            assert!(
                analyser.measure(Slot::In, size, RATE, COLUMNS, &mut out),
                "{:?} refused a full window",
                size
            );
            assert_eq!(out.len(), COLUMNS);
            assert!(out.iter().all(|v| v.is_finite()));
            let hz = 40.0 * RATE / size.len() as f32;
            if hz >= FLO {
                let read = column_of(&out, hz);
                assert!(
                    read.abs() < 0.5,
                    "{:?} read {read} dBFS at {hz:.1} Hz",
                    size
                );
            }
        }
    }
}
