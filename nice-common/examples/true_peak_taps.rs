//! Regenerate the true-peak estimator's branch coefficients.
//!
//! `cargo run -q -p nice-common --example true_peak_taps` prints the
//! `BRANCHES` literal used by `src/true_peak.rs`, plus the measurements that
//! justify the design. The recipe lives here and nowhere else:
//! `src/true_peak.rs` carries the printed numbers, and a test in that module
//! recomputes this recipe in `f64` and checks the literal against it, so a
//! stale table fails CI instead of silently degrading the meter.
//!
//! The design, in the words of the module it feeds: a Blackman-windowed sinc
//! interpolator for a 4x oversampled stream with cutoff at exactly the input
//! Nyquist, cut into polyphase branches. The prototype is `4·TAPS + 1` taps
//! long and centred on tap `2·TAPS`, which is a multiple of the oversampling
//! factor — that is what makes the three printed branches plus the raw input
//! sample land on one uniform 4x grid of instants (offsets 0, ¼, ½, ¾ of a
//! sample) instead of a ragged one. Branch `p` reads prototype taps
//! `p, p+4, p+8, …`, newest sample first, and reconstructs the signal a
//! quarter-sample-truncated delay of `4 − p/4` samples behind the newest
//! input. Each branch is then normalised to *its own* unity DC gain.
//!
//! Two properties fall out of choosing the cutoff at exactly Nyquist (β = 1)
//! and normalising per branch, and both are measured below:
//!
//! * Branch 0 is an exact Kronecker delta — its only nonzero tap is the
//!   prototype centre, `sinc(0)·blackman(1) = 1`. The raw-sample path *is*
//!   that branch, so it costs nothing and a constant input reads exactly the
//!   sample value.
//! * Every branch has `|G_p(f)| ≤ 1` across the band, with equality only via
//!   the flat δ branch, so the estimator can never read a steady sinusoid
//!   above its true peak. A wider cutoff (β > 1) closes the residual
//!   under-read by ~0.4 dB but overshoots up to +2.5 dB — it would light the
//!   clip indicator on material that is not clipping, which is disqualifying.
//!   The honest number is the under-read: worst −0.694 dB for `f ≤ 0.45·fs`, at
//!   f = fs/3, and −6.023 dB exactly at f = fs/2, where a quadrature sine has
//!   every sample at a zero crossing. That last one is structural to any 4×
//!   scheme and is far worse for the raw-sample meter it replaces.

/// Oversampling factor of the virtual grid the estimator reads.
const OVER: usize = 4;
/// Taps per printed branch, i.e. the span of history each branch reads.
/// T = 8 reads at worst 0.694 dB under below 0.45·fs; T = 10 measures 0.018 dB
/// better for 25% more taps, and the whole family is bounded below by the 4x
/// grid floor `20·log10(cos(π/8)) = −0.69 dB`, so the extra taps buy nothing
/// real.
const TAPS: usize = 8;
/// Prototype length. Odd, centred on a multiple of `OVER`.
const M: usize = OVER * TAPS + 1;
/// Prototype centre tap. `2·TAPS` is a multiple of `OVER` by construction.
const CENTRE: usize = 2 * TAPS;
/// Relative margin the f32 taps are scaled to sit under unity, so that f32
/// accumulation (fixed order, as the kernel runs it) of a constant input
/// stays strictly below the sample value. The δ path then supplies that value
/// exactly. 4e-6 ≈ 34 f32 ulps — orders of magnitude above the accumulation
/// error, 0.00003 dB in level.
const HEADROOM: f32 = 1.0 - 4e-6;

/// Blackman window over the prototype, `n` in `0..M`.
fn blackman(n: usize) -> f64 {
    let x = n as f64 / (M - 1) as f64;
    0.42 - 0.5 * (2.0 * std::f64::consts::PI * x).cos()
        + 0.08 * (4.0 * std::f64::consts::PI * x).cos()
}

/// Prototype with cutoff at `beta` × the input Nyquist — `sin(βπx)/(πx)`
/// windowed by Blackman — in the ×4-gain form:
/// each polyphase branch has unity DC gain, so a branch's output is the
/// signal's value at its own instant, in the input's own units. A unity-DC
/// prototype would read 12 dB low.
///
/// β = 1 is what makes branch 0 an exact delta. Its taps sit at integer
/// multiples of the input period, and `sinc` vanishes at every nonzero
/// integer; any β > 1 moves the zeros off the integers, so the raw-sample
/// path stops being one of the branches. Measured below.
fn prototype_at(beta: f64) -> [f64; M] {
    let mut h = [0.0f64; M];
    for (i, tap) in h.iter_mut().enumerate() {
        let x = (i as isize - CENTRE as isize) as f64 / OVER as f64;
        let s = if x == 0.0 {
            beta
        } else {
            (beta * std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
        };
        *tap = s * blackman(i);
    }
    h
}

fn prototype() -> [f64; M] {
    prototype_at(1.0)
}

/// Raw branch `p`, newest sample first: `t[k]` multiplies the sample `k`
/// inputs back. Branch 0 has `TAPS + 1` entries (`p + OVER·k ≤ 2·TAPS` for
/// `k ≤ TAPS`); every other branch has exactly `TAPS`.
fn branch(h: &[f64; M], p: usize) -> Vec<f64> {
    (0..)
        .map(|k| p + OVER * k)
        .take_while(|&i| i < M)
        .map(|i| h[i])
        .collect()
}

/// |response| of a branch at frequency `f` in cycles per input sample.
fn gain(t: &[f64], f: f64) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (k, tv) in t.iter().enumerate() {
        let w = -2.0 * std::f64::consts::PI * f * k as f64;
        re += tv * w.cos();
        im += tv * w.sin();
    }
    (re * re + im * im).sqrt()
}

fn gcd(a: u64, b: u64) -> u64 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The shipped table: branches 1..3 as `f32`, each normalised to its own
/// unity DC and then scaled down to `HEADROOM` so that f32 accumulation of a
/// constant input stays under the sample value (verified by `dc_check`).
fn shipped_branches() -> [[f32; TAPS]; OVER - 1] {
    let h = prototype();
    let mut out = [[0.0f32; TAPS]; OVER - 1];
    for (i, row) in out.iter_mut().enumerate() {
        let mut t = branch(&h, i + 1);
        let dc = t.iter().sum::<f64>();
        for tv in t.iter_mut() {
            *tv /= dc;
        }
        // Pick the scale that lands the *f32-cast* tap sum on HEADROOM: the
        // f64 target is off by up to a half-ulp per tap once cast.
        let mut scale = HEADROOM as f64;
        for _ in 0..4 {
            let sum: f64 = t.iter().map(|tv| (*tv * scale) as f32 as f64).sum();
            scale *= HEADROOM as f64 / sum;
        }
        for (k, tv) in t.iter().enumerate() {
            row[k] = (*tv * scale) as f32;
        }
    }
    out
}

/// Fold exactly as the kernel does — newest tap first, plain `f32` adds —
/// the output of every branch on a constant input of `c`.
fn dc_read(branches: &[[f32; TAPS]; OVER - 1], c: f32) -> f32 {
    let mut worst = c.abs(); // the δ branch, bit-exact
    for t in branches {
        let mut acc = 0.0f32;
        for &tap in t.iter() {
            acc += tap * c;
        }
        worst = worst.max(acc.abs());
    }
    worst
}

/// Peak the estimator reads for `input`, running exactly the kernel
/// `src/true_peak.rs` ships: δ branch first, then the three printed branches
/// in order, history newest-first.
fn read_of(branches: &[[f32; TAPS]; OVER - 1], input: &[f32], warmup: usize) -> f32 {
    let mut hist = [0.0f32; TAPS - 1];
    let mut peak = 0.0f32;
    for (n, &x) in input.iter().enumerate() {
        let mut m = x.abs();
        for t in branches {
            let mut acc = t[0] * x;
            for (i, &h) in hist.iter().enumerate() {
                acc += t[i + 1] * h;
            }
            m = m.max(acc.abs());
        }
        if n >= warmup {
            peak = peak.max(m);
        }
        for k in (1..TAPS - 1).rev() {
            hist[k] = hist[k - 1];
        }
        hist[0] = x;
    }
    peak
}

fn main() {
    let h = prototype();
    let branches = shipped_branches();

    // Branch 0 is printed nowhere: verify the claim that replaces it.
    let b0 = branch(&h, 0);
    let dc0 = b0.iter().sum::<f64>();
    let norm: Vec<f64> = b0.iter().map(|t| t / dc0).collect();
    let centre = norm.iter().position(|t| t.abs() > 0.5).expect("centre tap");
    let off: f64 = norm
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != centre)
        .map(|(_, t)| t.abs())
        .fold(0.0, f64::max);
    assert_eq!(centre, TAPS / 2, "delta must sit at the prototype centre");
    assert!(
        off < 1e-12,
        "branch 0 must be a delta, found off-centre tap {off:e}"
    );
    println!("branch 0: delta at k={centre}, off-centre taps < {off:e}  → raw max|x|");

    // Why β = 1 and not a touch more: the same measurement at β > 1.
    for beta in [1.05, 1.10] {
        let hb = prototype_at(beta);
        let b = branch(&hb, 0);
        let d = b.iter().sum::<f64>();
        let worst = b
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != centre)
            .map(|(_, t)| (t / d).abs())
            .fold(0.0f64, f64::max);
        println!(
            "  at β = {beta:.2} branch 0 is NOT a delta: worst off-centre tap {worst:.4}, \
             raw max|x| stops being one of the branches"
        );
    }

    // Shipped-table facts.
    let mut l1 = 0.0f64;
    for (i, t) in branches.iter().enumerate() {
        let sum: f64 = t.iter().map(|v| *v as f64).sum();
        let norm1: f64 = t.iter().map(|v| v.abs() as f64).sum();
        l1 = l1.max(norm1);
        println!("branch {}: f32 tap sum {sum:.9}  Σ|taps| {norm1:.4}", i + 1);
        for (k, v) in t.iter().enumerate() {
            println!("    t[{k}] = {}f32,", v);
        }
    }
    println!(
        "max Σ|taps| over branches: {l1:.4}  ({:.2} dB transient overshoot bound)",
        20.0 * l1.log10()
    );

    // The |G| bound above is steady-state. An abrupt onset from silence is not
    // a band-limited signal, and the branches ring on it exactly as a real
    // converter's reconstruction filter does. Measured, not assumed: the crude
    // Σ|taps| bound never occurs, the step response is the real figure.
    let mut rise = [0.0f32; 32];
    rise[8..].fill(1.0);
    let mut fall = [1.0f32; 32];
    fall[24..].fill(0.0);
    for (name, input) in [("0→1 step", &rise), ("1→0 step", &fall)] {
        let read = read_of(&branches, input, 0);
        println!(
            "worst transient: a full-scale {name} reads {read:.4} = {:+.2} dB over 0 dBFS \
             (raw sample peak reads 1.0000)",
            20.0 * f64::from(read).log10()
        );
    }

    // The everyday case the meter exists for: a full-scale tone placed so its
    // samples miss the crest of the wave entirely — at a quarter of the rate,
    // half a sample of offset (π/8 of phase) puts every sample as far off the
    // crest as it can be: a quarter cycle (π/4 of phase) leaves every sample
    // at cos(π/4) while the wave itself is at full scale.
    for (name, phase) in [
        ("worst phase", std::f64::consts::FRAC_PI_4),
        ("half a sample off", std::f64::consts::FRAC_PI_8),
    ] {
        let mut q = [0.0f32; 96];
        for (n, s) in q.iter_mut().enumerate() {
            *s = (2.0 * std::f64::consts::PI * 0.25 * n as f64 + phase).sin() as f32;
        }
        let raw = q.iter().fold(0.0f32, |a, &x| a.max(x.abs()));
        let read = read_of(&branches, &q, 0);
        println!(
            "tone at 0.25·fs, {name}: samples peak {raw:.4}, the estimator reads {read:.4} \
             ({:+.2} dB of inter-sample overshoot recovered)",
            20.0 * (f64::from(read) / f64::from(raw)).log10()
        );
    }

    // A constant input must read exactly the sample value (the δ path), for
    // the levels the crate's tests pin.
    for c in [1.0f32, 0.999_999_9, 0.75, 0.5, 0.123_456_78] {
        let read = dc_read(&branches, c);
        assert_eq!(read, c.abs(), "constant {c} must read exactly |c|");
        assert_eq!(dc_read(&branches, -c), c.abs());
    }
    println!("constant input: reads exactly |x| for every checked level (δ path)");

    // |G_p(f)| of the *shipped* f32 table over a dense grid, plus the union
    // max, which the meter actually takes (via the δ branch, |G_0| ≡ 1).
    let g: Vec<[f64; 3]> = (0..=20_000)
        .map(|i| {
            let f = i as f64 / 40_000.0;
            std::array::from_fn(|p| gain(&branches[p].map(f64::from), f))
        })
        .collect();
    let max_g = g
        .iter()
        .flat_map(|row| row.iter().copied())
        .fold(0.0f64, f64::max);
    println!("max |G_p(f)| over branches 1..3, dense grid: {max_g:.9}");
    assert!(max_g <= 1.0 + 1e-6, "a branch overshoots unity: {max_g}");
    for f in [0.125f64, 0.2, 0.25, 1.0 / 3.0, 0.4, 0.49] {
        let i = (f * 40_000.0).round() as usize;
        let row = g[i.min(20_000)];
        println!(
            "f = {f:.4}: |G| = {:.4} {:.4} {:.4}",
            row[0], row[1], row[2]
        );
    }

    // Empirical counterpart of the |G| bound, through the real kernel: a
    // full-scale steady sine never reads above full scale, at any frequency or
    // phase. Warm-up samples are excluded, so this measures the steady state.
    let mut over = (0.0f64, 0.0f64); // (worst read, f)
    for i in 1..=200 {
        let f = i as f64 / 400.0;
        for s in 0..16 {
            let phi = std::f64::consts::TAU * s as f64 / 16.0;
            let w: Vec<f32> = (0..512)
                .map(|n| (std::f64::consts::TAU * f * n as f64 + phi).sin() as f32)
                .collect();
            let read = f64::from(read_of(&branches, &w, 128));
            if read > over.0 {
                over = (read, f);
            }
        }
    }
    println!(
        "full-scale steady sine, 200 freqs × 16 phases through the kernel: worst read {:.5} \
         ({:+.4} dB) at f = {:.3}·fs — never above full scale",
        over.0,
        20.0 * over.0.log10(),
        over.1
    );
    assert!(
        over.0 <= 1.0,
        "a steady sine read over full scale: {over:?}"
    );

    // Exact worst-case read of a steady sinusoid, over every rational
    // f = a/b (coprime, b ≤ 48) and every phase. The estimator samples the
    // continuous signal on the uniform grid τ = q/4, weighting grid point q
    // by |G_{q mod 4}(f)| (branch 0 → 1). For f = a/b the phase values
    // 2π·f·q/4 close on a (4b/g)-point orbit, g = gcd(a, 4b), so the read
    // for a phase φ is  max_j w_j·|cos(2πj/(4b) + φ)|  — no simulation, no
    // block-length or windowing artefacts.
    let mut worst = (1.0f64, 0.0f64); // (read, f)
    let mut worst_band = (1.0f64, 0.0f64); // same, for f ≤ 0.45·fs
    const MAX_B: u64 = 48;
    const PHASES: usize = 192;
    for b in 1..=MAX_B {
        for a in 1..=b / 2 {
            if gcd(a, b) != 1 {
                continue;
            }
            let f = a as f64 / b as f64;
            let w = [
                1.0f64,
                gain(&branches[0].map(f64::from), f),
                gain(&branches[1].map(f64::from), f),
                gain(&branches[2].map(f64::from), f),
            ];
            let n = 4 * b;
            // w_j: weight of orbit point j = a·q mod 4b, q mod 4 = branch.
            // (a = 0 is DC: read exactly |x| via the δ path, checked above.)
            let mut wj = vec![0.0f64; n as usize];
            for q in 0..n {
                let j = (a * q) % n;
                let v = w[(q % 4) as usize];
                if v > wj[j as usize] {
                    wj[j as usize] = v;
                }
            }
            let mut worst_phase = 1.0f64;
            for s in 0..PHASES {
                let phi = std::f64::consts::PI * s as f64 / PHASES as f64;
                let mut read = 0.0f64;
                for j in 0..n {
                    if wj[j as usize] == 0.0 {
                        continue;
                    }
                    let c = (std::f64::consts::TAU * j as f64 / n as f64 + phi).cos();
                    read = read.max(wj[j as usize] * c.abs());
                }
                worst_phase = worst_phase.min(read);
            }
            if worst_phase < worst.0 {
                worst = (worst_phase, f);
            }
            if f <= 0.45 && worst_phase < worst_band.0 {
                worst_band = (worst_phase, f);
            }
        }
    }
    println!(
        "worst steady-sinusoid read: {:.3} dB at f = {:.4}·fs (max read 0.000 dB, see max |G|)",
        20.0 * worst.0.log10(),
        worst.1
    );
    println!(
        "worst read for f ≤ 0.45·fs: {:.3} dB at f = {:.4}·fs",
        20.0 * worst_band.0.log10(),
        worst_band.1
    );
    println!(
        "f = fs/2 is blind for any 4x scheme AND for the current raw peak: an exact-Nyquist sine at \
         quadrature phase has near-zero samples; the estimator still reads 0.5·true peak (−6 dB)."
    );
    println!(
        "4x grid floor 20·log10(cos(π/8)) = {:.3} dB; current raw-sample peak errs up to {:.2} dB at f = fs/4",
        20.0 * (std::f64::consts::PI / 8.0).cos().log10(),
        20.0 * (std::f64::consts::PI / 4.0).cos().log10()
    );

    // Cost, for the docs. The runtime kernel is (OVER−1) branches × TAPS MAC
    // plus one abs/max (the free δ branch) per input sample per channel.
    println!(
        "cost: {} MAC + 1 abs + 1 max per sample per channel; {} MAC per stereo channel-pair per 128-frame block",
        (OVER - 1) * TAPS,
        2 * 128 * (OVER - 1) * TAPS
    );
}
