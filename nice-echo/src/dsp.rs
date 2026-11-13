//! The tape path: one record head, three playback heads on a free-running
//! loop, a spring tank beside it, and the mode selector that decides who
//! reaches the output.
//!
//! ## What the sources say
//!
//! Quoted from *Inside a Space Echo* (Red Bull Music Academy,
//! <http://web.archive.org/web/20190912173023/https://daily.redbullmusicacademy.com/specials/2016-inside-a-space-echo/>):
//! the mode selector is a “combination of three playback heads and the spring
//! reverb unit, achieving 12 different echo variations”, and it “includes a
//! reverb only mode”; the repeat rate works by “varying the tape speed… the
//! slower the tape, the longer the delay”; the intensity controls the repeat
//! count, and “high intensity levels allow for the echo to self-oscillate”;
//! the reverb volume works “at mode selections between 5 and 11 and reverb
//! only”; on the echo/normal switch, “echo” means the input signal is echoed
//! 100 %; and the machine runs a free “tape loop”, which “cuts down on tape
//! wear and playback speed irregularities”.
//!
//! Quoted from the RE-101/RE-201 Service Notes
//! (<http://manuals.fdiskc.com/flat/Roland%20RE-101%20&%20RE-201%20Service%20Manual.pdf>):
//! the output jack is specified at −15 dBm (H), −23 dBm (M) and −35 dBm (L).
//! That is where [`OUTPUT_TRIM_DB`] comes from: 8 dB from H to M, 20 dB from
//! H to L. The absolute dBm figures say what the jack delivers into its
//! rated load; into a modern interface travels whatever the host does with
//! it, so these enter the DSP as relative trims only.
//!
//! Neither source gives a delay time, a wow-and-flutter figure or a head
//! spacing anywhere, and nothing found so far does. Every number below that
//! is not one of those quotes is a **named assumption**, listed in
//! `README.md` under the same labels used here:
//!
//! * **A-1** — head spacing anchored to the commonly-cited “up to ~340 ms”
//!   maximum: taps at 110 / 220 / 330 ms at the slowest tape speed.
//! * **A-2** — the three heads are equally spaced.
//! * **A-3** — the repeat-rate knob spans tape speeds 1.0× to 3.25×
//!   (fastest taps ≈ 34 / 68 / 102 ms).
//! * **A-4** — the twelve-position mode table in [`mode_mask`]. The sources
//!   confirm twelve positions, three heads plus spring, and a spring-only
//!   position; the exact wiring of the other eleven is this model's, stated
//!   so it can be argued with.
//! * **A-5** — wow at 0.6 Hz ±0.25 ms, flutter at 6.1 Hz ±0.03 ms:
//!   deliberately modest, because the free-running loop (“cuts down on…
//!   playback speed irregularities”) earns a steadier transport than a
//!   capstan-pulled one.
//! * **A-6** — the spring is an input low-pass, a short pre-delay (the lag
//!   every spring reverb is known for), four parallel damped combs of
//!   unequal length — each low-passing its own feedback, so the ring dims
//!   treble-first — summed into a tail low-pass, fed from the dry input in
//!   parallel with the tape path. The damped comb is the standard block;
//!   every figure in it is this model's. (This replaces an all-pass homage
//!   whose maths came from nice-phase; TODO.md carries that history.)
//! * **A-7** — the echo/normal switch reads: NORMAL = dry and echo
//!   together; ECHO = the dry is gone and what you hear is 100 % echo.
//! * **A-8** — per-head HF loss: two-pole corners at 9 / 6 / 4 kHz (one
//!   first-order stage cascaded with itself), duller the farther the tape
//!   has travelled.
//! * **A-9** — the record path carries a deterministic ±1.5e-4 (≈ −76 dBFS)
//!   bias noise, the stand-in for tape hiss and head electronics. No source
//!   gives a hiss figure; the model needs one for a starker reason than
//!   realism: a noise-free digital loop makes exact silence a fixed point
//!   — tanh(0) = 0 and every filter passes zero — so the silence a burst
//!   leaves on the tape re-records itself forever, hole and all. The noise
//!   breathes with the wow — hiss on a real machine tracks tape speed —
//!   by ±40 % across the wow cycle, a figure as much this model's as the
//!   ±1.5e-4 it modulates.
//! * **A-10** — Intensity crosses unity loop gain at ≈ 0.74 of its travel;
//!   [`feedback_gain`] tops out at 1.35. The sources say high levels
//!   self-oscillate and give no threshold; where the crossover sits is this
//!   model's choice.
//! * **A-11** — tape wear: one minutes-long wander modulates the transport
//!   — wow depth by ±35 %, tap spacing by ±0.3 %, each head's HF-loss
//!   corner by ±8 %, the three heads placed at different points of the
//!   cycle. The walk-through's own wear list (worn oxide, stretched loops)
//!   says loops tire; nobody has measured how far, so the spans are ours.
//! * **A-12** — the record path loses top end as it is driven: the write
//!   corner slides from 18 kHz clean to 4 kHz fully driven, following a
//!   one-pole envelope (≈ 21 ms) of the pre-saturation drive. Driven tape
//!   losing HF is lore, not a quote; the corners and the time constant are
//!   this model's.
//!
//! A block touches no allocator (see `tests/no_alloc.rs`): the loop and the
//! tank's memory are allocated at activation and never again.

/// The family's block: the most samples any call below carries, matching
/// `standalone::process`'s transport plan.
pub const BLOCK: usize = 64;

/// The twelve positions of the mode selector, numbered from 0 the way the
/// parameter counts them (the panel numbers them 1..=12).
pub const MODES: usize = 12;

/// **A-1/A-2.** Seconds from the record head to each playback head when the
/// tape runs at its slowest: 110, 220, 330 ms, equal spacing, anchored to
/// the commonly-cited maximum of about 340 ms. No service document states a
/// spacing; this is stated here so the machine is at least honest about it.
const HEAD_TAP_S: [f32; 3] = [0.110, 0.220, 0.330];

/// **A-3.** The tape-speed range the repeat-rate knob spans, as multiples
/// of the slowest speed. Slower tape, longer delay — the quoted law — so
/// the speed divides the delay below.
const SPEED_MIN: f32 = 1.0;
const SPEED_MAX: f32 = 3.25;

/// **A-8.** Two-pole low-pass corner for each head's HF loss, in Hz,
/// realised by cascading one first-order stage with itself: the farther
/// the tape has travelled since it was written, the duller it reads. The
/// second pole matters under heavy feedback — with a single pole a loop
/// near Nyquist barely loses to the dull ones and wins, which is not how
/// the hardware behaves.
const HEAD_FC_HZ: [f32; 3] = [9_000.0, 6_000.0, 4_000.0];

/// **A-5.** Wow and flutter: rates in Hz and depths in seconds. The depths
/// are small on purpose — the free-running loop the sources describe is the
/// machine's own argument for a steady transport.
const WOW_HZ: f32 = 0.6;
const WOW_DEPTH_S: f32 = 0.000_25;
const FLUTTER_HZ: f32 = 6.1;
const FLUTTER_DEPTH_S: f32 = 0.000_03;

/// **A-9.** How much the hiss breathes with the wow: the noise amplitude is
/// scaled by 1 ± HISS_BREATH across the wow cycle. Hiss tracking speed is
/// the behaviour; this number is ours, like the amplitude it modulates.
const HISS_BREATH: f32 = 0.4;

/// **A-11.** Tape wear: one very slow wander — roughly one cycle every
/// four minutes — that the transport ages with: wow depth, tap spacing and
/// each head's HF-loss corner all ride it. Nobody has measured a tired
/// loop; that loops tire is the walk-through's own wear list, the amounts
/// are ours.
const WEAR_HZ: f32 = 1.0 / 227.0;
const WEAR_WOW_DEPTH: f32 = 0.35;
const WEAR_TAP_SPAN: f32 = 0.003;
const WEAR_HF_SPAN: f32 = 0.08;
/// Where each head sits in the wander cycle, radians — spread across it so
/// the three gaps drift against each other instead of in lockstep.
const WEAR_PHASE: [f32; 3] = [0.0, 2.1, 4.2];

/// **A-12.** The record pole: driven tape loses top end, so the corner the
/// write signal passes through slides from 18 kHz (clean) to 4 kHz (fully
/// driven) as the drive envelope rises. The direction of travel is tape
/// lore; the corners and the envelope's ≈ 21 ms time constant are ours.
const RECORD_HF_CLEAN_HZ: f32 = 18_000.0;
const RECORD_HF_DRIVEN_HZ: f32 = 4_000.0;
const REC_DRIVE_SLEW: f32 = 0.001;

/// **A-6.** The spring tank: an input low-pass, then the pre-delay, then
/// four damped combs of unequal length feeding one tail low-pass. Delays in
/// seconds, the comb loop-gain, and the low-pass corners in Hz. The comb is
/// the standard block; every number here is ours, stated so it can be
/// argued with. The pre-delay is the spring's signature — energy arrives
/// *late* — and the comb dampings are unequal so the ring dims treble-first.
const SPRING_PRE_S: f32 = 0.003_5;
const SPRING_PRE_LP_HZ: f32 = 3_200.0;
const SPRING_COMB_S: [f32; 4] = [0.013_7, 0.018_1, 0.023_9, 0.029_3];
const SPRING_COMB_G: f32 = 0.84;
const SPRING_COMB_LP_HZ: [f32; 4] = [3_000.0, 2_400.0, 1_900.0, 1_500.0];
/// The spring's tail low-pass, after the comb garden.
const SPRING_TAIL_HZ: f32 = 2_400.0;

/// How fast the tape speed chases its target, as a per-sample blend: a hand
/// on the Repeat Rate knob takes about a quarter of a second to arrive, and
/// the smear on the way is the trick the rate control is *for*.
const SPEED_SLEW: f32 = 0.000_08;

/// The output-level steps, in dB relative to H, straight off the Service
/// Notes: H −15 dBm, M −23 dBm, L −35 dBm.
pub const OUTPUT_TRIM_DB: [f32; 3] = [0.0, -8.0, -20.0];

/// Spring bit of a mode mask; bits 0..=2 are the three heads.
const SPRING_BIT: u8 = 1 << 3;

/// **A-4.** The mode selector as this model ships it: which heads and
/// whether the spring reach the mixer, for each of the twelve positions.
const fn mode_table() -> [u8; MODES] {
    [
        0b0001,     // 1: head 1
        0b0010,     // 2: head 2
        0b0100,     // 3: head 3
        0b0111,     // 4: all three heads
        0b1001,     // 5: head 1 + spring
        0b1010,     // 6: head 2 + spring
        0b1100,     // 7: head 3 + spring
        0b1011,     // 8: heads 1+2 + spring
        0b1110,     // 9: heads 2+3 + spring
        0b1101,     // 10: heads 1+3 + spring
        0b1111,     // 11: everything
        SPRING_BIT, // 12: spring alone
    ]
}

/// The heads and spring a mode position selects. Positions outside the
/// selector clamp to it, because automation can arrive anywhere the host
/// thinks the parameter can go.
pub fn mode_mask(mode: i32) -> u8 {
    mode_table()[mode.clamp(0, MODES as i32 - 1) as usize]
}

/// What a panel position routes, in the words the view prints under the
/// mode knob. The sentence is built from `mode_mask`'s own bits, so a
/// caption can never name a head the DSP does not mix or a spring it does
/// not feed — the panel label and the mixer read the same table, and the
/// only way they can disagree is if the table itself changes meaning.
pub fn mode_caption(mode: i32) -> String {
    let mask = mode_mask(mode);
    let mut heads = String::new();
    for head in 0..3u8 {
        if mask & (1 << head) != 0 {
            if !heads.is_empty() {
                heads.push('+');
            }
            heads.push(char::from(b'1' + head));
        }
    }
    match (heads.is_empty(), mask & SPRING_BIT != 0) {
        (true, true) => "spring only".to_string(),
        // No mask in the shipped table routes nothing, but the clamp
        // admits every integer to this function, so the honest answer for
        // a table that stops meaning what it says is that nothing moves.
        (true, false) => "nothing routed".to_string(),
        (false, false) if heads.len() == 1 => format!("head {heads} only"),
        (false, false) => format!("heads {heads}"),
        (false, true) if heads.len() == 1 => format!("head {heads} + spring"),
        (false, true) => format!("heads {heads} + spring"),
    }
}

/// The tape speed the repeat-rate knob asks for: the knob's `0.0..=1.0`
/// across the **A-3** range, linearly.
pub fn tape_speed(rate: f32) -> f32 {
    SPEED_MIN + (SPEED_MAX - SPEED_MIN) * rate.clamp(0.0, 1.0)
}

/// Seconds from record to head `head` at the knob's rate. The quoted law
/// made arithmetic: the speed divides, so slower tape is longer delay.
pub fn tap_seconds(rate: f32, head: usize) -> f32 {
    HEAD_TAP_S[head.min(HEAD_TAP_S.len() - 1)] / tape_speed(rate)
}

/// The repeat-count knob as a loop gain: unity is crossed around
/// three-quarters of the travel (A-10), so the top quarter of the knob is
/// the howling quarter the sources describe. The honest history: the first
/// top gain, 1.06, did self-oscillate eventually — in around half a minute
/// of lap-by-lap growth, which is not how the unit behaves at all. 1.35
/// howls in seconds.
pub fn feedback_gain(intensity: f32) -> f32 {
    intensity.clamp(0.0, 1.0) * 1.35
}

/// The H/M/L output step, as a linear trim.
pub fn output_gain(level: i32) -> f32 {
    let db = OUTPUT_TRIM_DB[level.clamp(0, 2) as usize];
    (db * std::f32::consts::LN_10 / 20.0).exp()
}

/// The record head's saturation: `tanh`, the odd function that gives the
/// loop its “more drive, softer peaks” and bounds a self-oscillating echo
/// instead of letting it run to the rails.
fn saturate(x: f32) -> f32 {
    x.tanh()
}

/// One-pole low-pass coefficient for a corner at `fc` at `rate`. A corner
/// above Nyquist would ask for a coefficient over one, so it bows out of
/// the way and the pole simply passes everything.
fn lp_coeff(fc: f32, rate: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * fc.min(rate * 0.45) / rate).exp()
}

/// Every control at once, in engineering units. The plugin owns the mapping
/// from panel names to these; the DSP only ever sees this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// Mode selector, 0..=11 (the panel's 1..=12).
    pub mode: i32,
    /// Repeat Rate: tape speed across **A-3**, 0 = slowest.
    pub repeat_rate: f32,
    /// Intensity: repeat count; full self-oscillates.
    pub intensity: f32,
    /// Echo Volume: the heads' share of the output.
    pub echo_volume: f32,
    /// Reverb Volume: the spring's share.
    pub reverb_volume: f32,
    /// Treble, centred at 0.5.
    pub treble: f32,
    /// Bass, centred at 0.5.
    pub bass: f32,
    /// 0 = NORMAL (dry and echo together), 1 = ECHO (the dry is gone). **A-7.**
    pub echo_switch: i32,
    /// Output level: 0 = H, 1 = M, 2 = L.
    pub output_level: i32,
}

impl Default for Controls {
    /// The resting place: a wide mode with the spring in, a medium-long
    /// delay, a few repeats, both tone knobs centred, dry signal passing.
    fn default() -> Self {
        Self {
            mode: 10,
            repeat_rate: 0.45,
            intensity: 0.4,
            echo_volume: 0.5,
            reverb_volume: 0.3,
            treble: 0.5,
            bass: 0.5,
            echo_switch: 0,
            output_level: 0,
        }
    }
}

/// A head's HF-loss poles — the same one-pole stage twice. **A-8.**
#[derive(Clone, Copy, Default)]
struct Head {
    lp1: f32,
    lp2: f32,
}

/// One damped comb: a delay line whose output is low-passed and fed back
/// into its own input. The block is old and standard; the numbers driving
/// it here are A-6's.
#[derive(Default)]
struct Comb {
    buf: Vec<f32>,
    i: usize,
    lp: f32,
}

impl Comb {
    fn set_delay(&mut self, samples: usize) {
        self.buf = vec![0.0; samples.max(1)];
        self.i = 0;
        self.lp = 0.0;
    }

    /// Read what is on the line, damp it into the feedback path, write the
    /// input plus the damped tail at the cursor. The low-pass inside the
    /// loop is what makes the ring dim treble-first: each lap loses top
    /// end before it loses level.
    fn run(&mut self, x_in: f32, g: f32, a: f32) -> f32 {
        let out = self.buf[self.i];
        self.lp += a * (out - self.lp);
        self.buf[self.i] = x_in + g * self.lp;
        self.i += 1;
        if self.i == self.buf.len() {
            self.i = 0;
        }
        out
    }
}

/// The tank: input low-pass, a short pre-delay — the spring's signature
/// lag, energy arriving *late* — then four damped combs of unequal length
/// summed, then the tail low-pass. **A-6.**
#[derive(Default)]
struct Spring {
    pre: Vec<f32>,
    pre_i: usize,
    pre_lp: f32,
    combs: [Comb; 4],
    tail_state: f32,
}

impl Spring {
    fn set_rate(&mut self, rate: f32) {
        self.pre = vec![0.0; (SPRING_PRE_S * rate).round() as usize];
        self.pre_i = 0;
        self.pre_lp = 0.0;
        for (i, s) in SPRING_COMB_S.iter().enumerate() {
            self.combs[i].set_delay((s * rate).round() as usize);
        }
        self.tail_state = 0.0;
    }

    fn run(&mut self, x_in: f32, coeffs: &[f32; 5], tail_a: f32) -> f32 {
        self.pre_lp += coeffs[0] * (x_in - self.pre_lp);
        // Read the pre-delay before writing: what comes out now was
        // written SPRING_PRE_S seconds ago, so the first samples into a
        // dead tank come out silent. The lag is the point.
        let d = self.pre[self.pre_i];
        self.pre[self.pre_i] = self.pre_lp;
        self.pre_i += 1;
        if self.pre_i == self.pre.len() {
            self.pre_i = 0;
        }
        let mut sum = 0.0f32;
        for (c, comb) in self.combs.iter_mut().enumerate() {
            sum += comb.run(d, SPRING_COMB_G, coeffs[1 + c]);
        }
        let mix = sum * 0.25;
        self.tail_state += tail_a * (mix - self.tail_state);
        self.tail_state
    }
}

/// One lane's tape: the loop's memory, the write cursor, the heads.
#[derive(Default)]
struct Tape {
    line: Vec<f32>,
    write: usize,
    heads: [Head; 3],
    /// A-9's hiss generator, a plain LCG — identically seeded in both
    /// lanes so the one monophonic tape stays bit-exact across them.
    rng: u32,
    /// A-12's drive envelope into the record head, and the record pole's
    /// state. Per-lane, like everything the tape itself touches.
    drive_env: f32,
    rec_lp: f32,
}

impl Tape {
    /// The next hiss sample: high bits of the common 1_664_525 /
    /// 1_013_904_223 linear congruential step, scaled to ±1.5e-4, and
    /// breathing with the wow — `breath` is the shared −1..=1 wow position
    /// and A-9's `HISS_BREATH` sets the depth. Deterministic,
    /// allocation-free, and — the part the mono-stays-mono test leans on —
    /// the same sequence and the same shared breath in every lane.
    fn hiss(&mut self, breath: f32) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.rng >> 8) as f32 / 8_388_608.0 - 1.0) * 1.5e-4 * (1.0 + HISS_BREATH * breath)
    }
}

/// A lane's tone stage: one pole each for the bass and treble shelves.
#[derive(Clone, Copy, Default)]
struct Tone {
    bass_lp: f32,
    treble_lp: f32,
}

/// The tone stage, in the open so its claims can be nailed down. The bass
/// knob rides the signal the 160 Hz pole has caught; the treble knob rides
/// what the 4.5 kHz pole has not. Both centred — both coefficients zero —
/// the stage is the input exactly, bit for bit.
fn tone(t: &mut Tone, bass_a: f32, treble_a: f32, kb: f32, kt: f32, x: f32) -> f32 {
    t.bass_lp += bass_a * (x - t.bass_lp);
    let u = x + kb * t.bass_lp;
    t.treble_lp += treble_a * (u - t.treble_lp);
    u + kt * (u - t.treble_lp)
}

/// What the block does with the controls, worked out once per block.
struct Recipe {
    mask: u8,
    feedback: f32,
    echo_volume: f32,
    reverb_volume: f32,
    dry: f32,
    out_gain: f32,
    kb: f32,
    kt: f32,
    speed_target: f32,
}

fn recipe(c: &Controls) -> Recipe {
    Recipe {
        mask: mode_mask(c.mode),
        feedback: feedback_gain(c.intensity),
        echo_volume: c.echo_volume,
        reverb_volume: c.reverb_volume,
        dry: if c.echo_switch == 0 { 1.0 } else { 0.0 },
        out_gain: output_gain(c.output_level),
        kb: (c.bass * 2.0 - 1.0) * 0.6,
        kt: (c.treble * 2.0 - 1.0) * 0.6,
        speed_target: tape_speed(c.repeat_rate),
    }
}

/// The machine: two lanes of tape and spring under one transport. The tape
/// is monophonic — one record head, heard in both outputs — and the
/// transport's wow and flutter are shared, because the motor does not
/// wobble left and right differently.
pub struct Echo {
    rate: f32,
    tapes: [Tape; 2],
    springs: [Spring; 2],
    tone: [Tone; 2],
    /// Tape speed the transport is at right now, mid-slew.
    speed: f32,
    /// Shared wow/flutter phases, in radians.
    wow_p: f32,
    flutter_p: f32,
    /// A-11's shared wander: the phase, and sin/cos of the angle, so both
    /// lanes and every head can read one slow transport truth for two
    /// multiplies each.
    wear_p: f32,
    wear_s: f32,
    wear_c: f32,
    /// A-9: the wow LFO's own position (−1..=1), what the hiss breathes on.
    wow_lfo: f32,
    /// Per-head cos/sin of the wander's phase offsets, so a head's place
    /// in the cycle costs no trig.
    wear_phase_c: [f32; 3],
    wear_phase_s: [f32; 3],
    /// Per-head pole coefficients, and the spring's, record's and tone's,
    /// refreshed with the sample rate.
    head_a: [f32; 3],
    spring_a: [f32; 5],
    spring_tail_a: f32,
    /// A-12's record pole at the clean and the driven corner.
    rec_a_clean: f32,
    rec_a_driven: f32,
    tone_bass_a: f32,
    tone_treble_a: f32,
}

impl Echo {
    pub fn new(rate: f32, controls: &Controls) -> Self {
        let mut echo = Self {
            rate: 0.0,
            tapes: [Tape::default(), Tape::default()],
            springs: [Spring::default(), Spring::default()],
            tone: [Tone::default(), Tone::default()],
            speed: tape_speed(controls.repeat_rate),
            wow_p: 0.0,
            flutter_p: 0.0,
            wear_p: 0.0,
            wear_s: 0.0,
            wear_c: 1.0,
            wow_lfo: 0.0,
            wear_phase_c: WEAR_PHASE.map(f32::cos),
            wear_phase_s: WEAR_PHASE.map(f32::sin),
            head_a: [0.0; 3],
            spring_a: [0.0; 5],
            spring_tail_a: 0.0,
            rec_a_clean: 0.0,
            rec_a_driven: 0.0,
            tone_bass_a: 0.0,
            tone_treble_a: 0.0,
        };
        echo.set_rate(rate);
        echo
    }

    /// Re-arm for a sample rate: allocate the loop and the tank's memory.
    /// Called at activation, never mid-block.
    pub fn set_rate(&mut self, rate: f32) {
        let rate = rate.max(8_000.0);
        self.rate = rate;
        // The longest delay the transport can ask for is the far head at
        // slowest speed, stretched by A-11's wander and riding the full
        // wow depth; the loop is that, with room to spare.
        let longest = HEAD_TAP_S[HEAD_TAP_S.len() - 1] * (1.0 + WEAR_TAP_SPAN)
            + WOW_DEPTH_S * (1.0 + WEAR_WOW_DEPTH)
            + FLUTTER_DEPTH_S;
        let len = ((longest * rate).ceil() as usize) + 16;
        for tape in &mut self.tapes {
            tape.line = vec![0.0; len];
            tape.write = 0;
            tape.heads = [Head::default(); 3];
            tape.drive_env = 0.0;
            tape.rec_lp = 0.0;
        }
        for spring in &mut self.springs {
            spring.set_rate(rate);
        }
        self.tone = [Tone::default(), Tone::default()];
        self.head_a = [
            lp_coeff(HEAD_FC_HZ[0], rate),
            lp_coeff(HEAD_FC_HZ[1], rate),
            lp_coeff(HEAD_FC_HZ[2], rate),
        ];
        self.spring_a = [
            lp_coeff(SPRING_PRE_LP_HZ, rate),
            lp_coeff(SPRING_COMB_LP_HZ[0], rate),
            lp_coeff(SPRING_COMB_LP_HZ[1], rate),
            lp_coeff(SPRING_COMB_LP_HZ[2], rate),
            lp_coeff(SPRING_COMB_LP_HZ[3], rate),
        ];
        self.spring_tail_a = lp_coeff(SPRING_TAIL_HZ, rate);
        self.rec_a_clean = lp_coeff(RECORD_HF_CLEAN_HZ, rate);
        self.rec_a_driven = lp_coeff(RECORD_HF_DRIVEN_HZ, rate);
        self.tone_bass_a = lp_coeff(160.0, rate);
        self.tone_treble_a = lp_coeff(4_500.0, rate);
    }

    /// The sample rate the transport was armed for.
    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Process one mono block — at most [`BLOCK`] frames.
    pub fn process_mono(&mut self, controls: &Controls, input: &[f32], output: &mut [f32]) {
        let r = recipe(controls);
        let n = input.len().min(output.len()).min(BLOCK);
        for i in 0..n {
            let off = self.tick(&r);
            output[i] = self.lane(0, &r, input[i], off);
        }
    }

    /// Process one stereo block. Both lanes hear one tape: lane 0 and lane
    /// 1 of a duplicated input come out identical — a stereo spread is a
    /// TODO.md debt, honestly declared.
    pub fn process_stereo(
        &mut self,
        controls: &Controls,
        inputs: [&[f32]; 2],
        outputs: [&mut [f32]; 2],
    ) {
        let r = recipe(controls);
        let n = inputs[0]
            .len()
            .min(inputs[1].len())
            .min(outputs[0].len())
            .min(outputs[1].len())
            .min(BLOCK);
        for i in 0..n {
            let off = self.tick(&r);
            outputs[0][i] = self.lane(0, &r, inputs[0][i], off);
            outputs[1][i] = self.lane(1, &r, inputs[1][i], off);
        }
    }

    /// Advance the shared transport by one sample: the speed slews toward
    /// where the knob points — the smear the Repeat Rate knob is famous for
    /// is this line doing its work — and the wow, flutter and wear phases
    /// turn. Returns the shared delay offset, in samples.
    fn tick(&mut self, r: &Recipe) -> f32 {
        self.speed += (r.speed_target - self.speed) * SPEED_SLEW;
        self.wow_p += std::f32::consts::TAU * WOW_HZ / self.rate;
        self.flutter_p += std::f32::consts::TAU * FLUTTER_HZ / self.rate;
        // A-11: the minutes-long wander, once per sample. Keeping sin and
        // cos of the same angle means every head can be placed anywhere in
        // its cycle for two multiplies.
        self.wear_p += std::f32::consts::TAU * WEAR_HZ / self.rate;
        if self.wear_p > std::f32::consts::TAU {
            self.wear_p -= std::f32::consts::TAU;
        }
        self.wear_s = self.wear_p.sin();
        self.wear_c = self.wear_p.cos();
        if self.wow_p > std::f32::consts::TAU {
            self.wow_p -= std::f32::consts::TAU;
        }
        if self.flutter_p > std::f32::consts::TAU {
            self.flutter_p -= std::f32::consts::TAU;
        }
        let wow = self.wow_p.sin();
        // A-9: the hiss breathes on the raw wow position.
        self.wow_lfo = wow;
        // A-11: and the wow's own depth is deeper or shallower depending
        // on where the wear has the transport today.
        (wow * WOW_DEPTH_S * (1.0 + WEAR_WOW_DEPTH * self.wear_s)
            + self.flutter_p.sin() * FLUTTER_DEPTH_S)
            * self.rate
    }

    /// One sample of one lane, with the transport's shared offset applied.
    fn lane(&mut self, ch: usize, r: &Recipe, x: f32, wow_off: f32) -> f32 {
        // A-11: where each head sits in the wear wander right now.
        let (ws, wc) = (self.wear_s, self.wear_c);
        let breath = self.wow_lfo;
        let tape = &mut self.tapes[ch];
        let len = tape.line.len() as f32;
        let write = tape.write;
        let mut echo_sum = 0.0f32;
        for (h, &tap) in HEAD_TAP_S.iter().enumerate() {
            if r.mask & (1u8 << h) == 0 {
                continue;
            }
            // Read the heads before writing: they see the tape as it was
            // before this sample, which keeps every delay at least one
            // sample long.
            let w = ws * self.wear_phase_c[h] + wc * self.wear_phase_s[h];
            let d = tap * (1.0 + WEAR_TAP_SPAN * w) * self.rate / self.speed + wow_off;
            let v = read_line(&tape.line, write, d, len);
            let head = &mut tape.heads[h];
            // A-11 again: the head's HF-loss corner wanders with the tape,
            // clamped short of instability the way lp_coeff already is.
            let a = (self.head_a[h] * (1.0 + WEAR_HF_SPAN * w)).min(1.0);
            head.lp1 += a * (v - head.lp1);
            head.lp2 += a * (head.lp1 - head.lp2);
            echo_sum += head.lp2;
        }

        // Record: input plus however much echo the Intensity knob sends
        // back around, plus a thread of hiss (A-9) breathing with the wow,
        // through the saturation that bounds it. The feedback is the sum of
        // the selected heads — the mixer model the mode table describes.
        let drive = x + r.feedback * echo_sum;
        let fed = saturate(drive + tape.hiss(breath));
        // A-12: pushed tape loses top end. The record pole's coefficient
        // slides between the clean and the driven corner with a slow
        // envelope of the pre-saturation drive; interpolating in
        // coefficient space keeps the slide monotone and costs one fma.
        let env = tape.drive_env;
        tape.drive_env += REC_DRIVE_SLEW * (drive.abs() - env);
        let rec_a = self.rec_a_clean + env.min(1.0) * (self.rec_a_driven - self.rec_a_clean);
        tape.rec_lp += rec_a * (fed - tape.rec_lp);
        tape.line[write] = tape.rec_lp;
        tape.write = (write + 1) % tape.line.len();

        // The tank always runs its state — a mode switch should not unfreeze
        // a stale spring — but only its selected share reaches the output.
        let wet = self.springs[ch].run(x, &self.spring_a, self.spring_tail_a);
        let rev = if r.mask & SPRING_BIT != 0 {
            r.reverb_volume
        } else {
            0.0
        };

        let mix = r.dry * x + echo_sum * r.echo_volume + wet * rev;
        let shaped = tone(
            &mut self.tone[ch],
            self.tone_bass_a,
            self.tone_treble_a,
            r.kb,
            r.kt,
            mix,
        );
        shaped * r.out_gain
    }
}

/// Linear-interpolated read `delay` samples behind the write cursor of a
/// circular loop of length `len`, wow offset already folded in.
fn read_line(line: &[f32], write: usize, delay: f32, len: f32) -> f32 {
    let d = delay.clamp(1.0, len - 2.0);
    let mut p = write as f32 - d;
    if p < 0.0 {
        p += len;
    }
    let i0 = p as usize % line.len();
    let i1 = (i0 + 1) % line.len();
    let frac = p - p.floor();
    line[i0] * (1.0 - frac) + line[i1] * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Measurement defaults: a bare head mode, no repeats, wet-only output
    /// so the dry does not hide the echo being measured.
    fn controls_with(f: impl Fn(&mut Controls)) -> Controls {
        let mut c = Controls {
            mode: 0,
            repeat_rate: 0.0,
            intensity: 0.0,
            echo_volume: 0.8,
            reverb_volume: 0.0,
            treble: 0.5,
            bass: 0.5,
            echo_switch: 1, // ECHO: 100 % echo, per the source and A-7
            output_level: 0,
        };
        f(&mut c);
        c
    }

    const RATE: f32 = 48_000.0;

    fn run_silence(e: &mut Echo, c: &Controls, n: usize) {
        let buf = [0.0f32; BLOCK];
        let mut out = [0.0f32; BLOCK];
        for _ in 0..(n / BLOCK) {
            e.process_mono(c, &buf, &mut out);
        }
    }

    /// A short burst — bright enough that the heads' HF loss cannot eat it —
    /// then silence, returning the output. A burst, not a single-sample
    /// impulse, because a one-sample impulse is exactly what the dulling
    /// filters exist to remove.
    fn burst_response(e: &mut Echo, c: &Controls, n: usize) -> Vec<f32> {
        run_silence(e, c, BLOCK * 4);
        let mut out = Vec::with_capacity(n + BLOCK);
        let mut done = 0;
        let mut sent = 0usize;
        while done < n {
            let mut buf = [0.0f32; BLOCK];
            for s in buf.iter_mut() {
                if sent < 192 {
                    *s = 0.9;
                    sent += 1;
                }
            }
            let mut o = [0.0f32; BLOCK];
            e.process_mono(c, &buf, &mut o);
            out.extend_from_slice(&o);
            done += BLOCK;
        }
        out
    }

    /// The loudest sample inside a window around `at_s` seconds.
    fn peak_at(out: &[f32], at_s: f32, tol_s: f32) -> f32 {
        let c = (at_s * RATE) as usize;
        let w = (tol_s * RATE) as usize;
        let hi = (c + w).min(out.len());
        let lo = c.saturating_sub(w).min(hi);
        out[lo..hi].iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    fn rms_in(out: &[f32], from_s: f32, to_s: f32) -> f32 {
        let a = (from_s * RATE) as usize;
        let b = (to_s * RATE) as usize;
        (out[a..b].iter().map(|s| (s * s) as f64).sum::<f64>() / (b - a) as f64).sqrt() as f32
    }

    #[test]
    fn slower_tape_is_longer_delay() {
        // The quoted law, for every head.
        for (h, &tap) in HEAD_TAP_S.iter().enumerate() {
            assert!(
                tap_seconds(0.0, h) > tap_seconds(0.5, h),
                "head {h}: slower tape must mean longer delay"
            );
            assert!(tap_seconds(0.5, h) > tap_seconds(1.0, h));
            // Slowest is the A-1 anchor; the knob spans the A-3 ratio.
            assert!((tap_seconds(0.0, h) - tap).abs() < 1e-6);
            assert!(
                (tap_seconds(0.0, h) / tap_seconds(1.0, h) - SPEED_MAX).abs() < 1e-4,
                "the knob's span is the stated A-3 ratio"
            );
        }
    }

    #[test]
    fn the_mode_table_is_the_table_the_readme_ships() {
        let want: [u8; MODES] = [
            0b0001, 0b0010, 0b0100, 0b0111, 0b1001, 0b1010, 0b1100, 0b1011, 0b1110, 0b1101, 0b1111,
            SPRING_BIT,
        ];
        for m in 0..MODES as i32 {
            assert_eq!(mode_mask(m), want[m as usize], "mode {}", m + 1);
        }
        // Automation wandering outside the selector clamps to it.
        assert_eq!(mode_mask(-7), want[0]);
        assert_eq!(mode_mask(99), want[MODES - 1]);
        // The two facts the sources do give: twelve positions, one of them
        // the spring alone with no head in it.
        assert_eq!(MODES, 12);
        assert_eq!(mode_mask(11), SPRING_BIT);
        // The spring's own volume is quoted to work from mode 5 up: every
        // position from there on admits the spring.
        for m in 4..MODES as i32 {
            assert_ne!(
                mode_mask(m) & SPRING_BIT,
                0,
                "mode {} carries the spring",
                m + 1
            );
        }
        for m in 0..4 {
            assert_eq!(mode_mask(m) & SPRING_BIT, 0, "mode {} has no spring", m + 1);
        }
    }

    #[test]
    fn a_mode_caption_names_exactly_what_its_mask_routes() {
        // The caption is built from the mask's bits; the test reads the
        // caption back as English and demands it agree with those bits —
        // and pins the twelve sentences the face shows, so that editing
        // the README's panel table has to come with editing them.
        let want: [&str; MODES] = [
            "head 1 only",
            "head 2 only",
            "head 3 only",
            "heads 1+2+3",
            "head 1 + spring",
            "head 2 + spring",
            "head 3 + spring",
            "heads 1+2 + spring",
            "heads 2+3 + spring",
            "heads 1+3 + spring",
            "heads 1+2+3 + spring",
            "spring only",
        ];
        for m in 0..MODES as i32 {
            let caption = mode_caption(m);
            assert_eq!(caption, want[m as usize], "mode {}", m + 1);
            let mask = mode_mask(m);
            for head in 0..3u8 {
                assert_eq!(
                    caption.contains(char::from(b'1' + head)),
                    mask & (1 << head) != 0,
                    "mode {}: caption \"{caption}\" against mask {mask:#06b}",
                    m + 1
                );
            }
            assert_eq!(
                caption.contains("spring"),
                mask & SPRING_BIT != 0,
                "mode {}: caption \"{caption}\" against mask {mask:#06b}",
                m + 1
            );
        }
    }

    #[test]
    fn a_burst_walks_to_the_selected_head_and_back() {
        let c = controls_with(|c| {
            c.mode = 2; // head 3 only
            c.intensity = 0.2;
        });
        let mut e = Echo::new(RATE, &c);
        let out = burst_response(&mut e, &c, (0.5 * RATE) as usize);
        // Head 3's echo arrives at ~330 ms, near the recorded level times
        // Echo Volume, and the saturation keeps it inside full scale.
        let echo = peak_at(&out, tap_seconds(0.0, 2), 0.008);
        assert!(echo > 0.4, "head 3 echo should arrive, measured {echo}");
        assert!(echo <= 1.0, "saturation keeps it inside full scale");
        // Heads 1 and 2 are switched out; their times stay quiet. A
        // repeating head-3 echo lands at multiples of 330 ms, never at 110
        // or 220.
        for h in 0..2 {
            let other = peak_at(&out, tap_seconds(0.0, h), 0.008);
            assert!(
                other < 0.02,
                "head {} is out of the mode, heard {other}",
                h + 1
            );
        }
    }

    #[test]
    fn all_three_heads_hear_three_arrivals() {
        let c = controls_with(|c| {
            c.mode = 3; // all three heads, no spring
            c.intensity = 0.0;
        });
        let mut e = Echo::new(RATE, &c);
        let out = burst_response(&mut e, &c, (0.45 * RATE) as usize);
        for h in 0..3 {
            let p = peak_at(&out, tap_seconds(0.0, h), 0.006);
            assert!(p > 0.4, "head {} must arrive in mode 4", h + 1);
        }
        // Between arrivals with no feedback there is nothing but A-9's hiss
        // to fill the gaps: the delay line, not a diffuse pad, is doing the
        // work. The threshold sits a little over the hiss floor and a
        // world under the arrivals' level.
        let between = rms_in(&out, 0.145, 0.190);
        assert!(between < 1e-3, "gaps stay quiet with no repeats: {between}");
    }

    #[test]
    fn intensity_adds_repeats() {
        let quiet = controls_with(|c| {
            c.mode = 2;
            c.intensity = 0.2;
        });
        let loud = controls_with(|c| {
            c.mode = 2;
            c.intensity = 0.85;
        });
        // Long enough to hold the third repeat (≈0.99 s) and its window.
        let mut e = Echo::new(RATE, &quiet);
        let out = burst_response(&mut e, &quiet, (1.1 * RATE) as usize);
        let first = peak_at(&out, tap_seconds(0.0, 2), 0.006);
        let second = peak_at(&out, 2.0 * tap_seconds(0.0, 2), 0.006);
        let third = peak_at(&out, 3.0 * tap_seconds(0.0, 2), 0.006);
        let mut e = Echo::new(RATE, &loud);
        let out = burst_response(&mut e, &loud, (1.1 * RATE) as usize);
        let second_loud = peak_at(&out, 2.0 * tap_seconds(0.0, 2), 0.006);
        let third_loud = peak_at(&out, 3.0 * tap_seconds(0.0, 2), 0.006);
        assert!(
            second > 0.05 && second < first,
            "low intensity repeats, quieter each time (first {first}, second {second})"
        );
        assert!(
            second_loud > second && third_loud > third,
            "a higher knob means more echo on every later repeat"
        );
    }

    #[test]
    fn full_intensity_outlives_the_input() {
        // The quoted self-oscillation: at full intensity the loop is still
        // going long after the burst died.
        let c = controls_with(|c| {
            c.mode = 2;
            c.intensity = 1.0;
        });
        let mut e = Echo::new(RATE, &c);
        burst_response(&mut e, &c, BLOCK * 4);
        // Five seconds of nothing, then listen for a full second — three
        // trips around the loop. The pause has a reason: a noise-free
        // digital loop keeps the silence a burst leaves behind circulating
        // with everything else — the heads read the hole and re-record
        // silence over it, lap after lap — and only A-9's hiss, multiplied
        // by 1.35 each lap, fills it in. The second, not a snapshot, has a
        // reason too: the loop still dips once a lap where the hole is
        // thinnest, and the dip erodes 1.35× each trip, so the honest test
        // is the average over three laps, not one lucky window.
        run_silence(&mut e, &c, (5.0 * RATE) as usize);
        let mut tail = Vec::new();
        for _ in 0..((1.0 * RATE) as usize / BLOCK) {
            let mut o = [0.0f32; BLOCK];
            e.process_mono(&c, &[0.0f32; BLOCK], &mut o);
            tail.extend_from_slice(&o);
        }
        let alive = rms_in(&tail, 0.0, 1.0);
        assert!(
            alive > 0.05,
            "full intensity self-oscillates, heard {alive}"
        );
        // And it stays bounded — the record head's saturation is the ceiling.
        assert!(
            tail.iter().all(|s| s.abs() <= 2.0),
            "oscillation runs the tanh ceiling, not the rails"
        );
        // Below the crossover, the same loop decays — down to the hiss
        // floor, which is the quietest this machine can be.
        let c = controls_with(|c| {
            c.mode = 2;
            c.intensity = 0.3;
        });
        let mut e = Echo::new(RATE, &c);
        burst_response(&mut e, &c, BLOCK * 4);
        run_silence(&mut e, &c, (2.0 * RATE) as usize);
        let mut tail = Vec::new();
        for _ in 0..((0.2 * RATE) as usize / BLOCK) {
            let mut o = [0.0f32; BLOCK];
            e.process_mono(&c, &[0.0f32; BLOCK], &mut o);
            tail.extend_from_slice(&o);
        }
        assert!(
            rms_in(&tail, 0.0, 0.2) < 1e-3,
            "modest intensity dies back to the hiss floor"
        );
    }

    #[test]
    fn full_intensity_is_more_than_unity_around_the_loop() {
        // The claim behind self-oscillation, stated in the open: the knob
        // crosses unity loop gain near three-quarters of its travel (A-10),
        // so the last quarter of it is the howling quarter.
        assert!(feedback_gain(0.6) < 1.0, "two-thirds decays");
        assert!(feedback_gain(0.8) > 1.0, "the top quarter self-oscillates");
        assert_eq!(feedback_gain(0.0), 0.0);
    }

    #[test]
    fn saturation_is_odd_and_bounded() {
        // Odd: the machine does not favour one polarity.
        for x in [0.2f32, 1.0, 7.0, 400.0] {
            assert!((saturate(x) + saturate(-x)).abs() < 1e-7, "odd at {x}");
        }
        // Bounded: no input, however absurd, escapes full scale — and at
        // inputs far past the knee tanh lands exactly on 1.0, so the bound
        // is inclusive.
        for x in [0.0f32, 0.5, 3.0, 1e6] {
            assert!(saturate(x).abs() <= 1.0, "bounded at {x}");
        }
        // Gentle while there is headroom, soft once there is not.
        assert!((saturate(0.05) - 0.05).abs() < 0.000_5);
    }

    #[test]
    fn heads_get_duller_the_farther_the_tape_travels() {
        // A-8, as behaviour: run the same bright noise through each head's
        // pole and measure the wiggle left in it.
        let mut state = [0.0f32; 3];
        let mut hf = [0.0f64; 3];
        let mut prev = [0.0f32; 3];
        let mut seed = 0x1234_5678u32;
        for _ in 0..20_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
            for h in 0..3 {
                state[h] += lp_coeff(HEAD_FC_HZ[h], RATE) * (x - state[h]);
                let d = (state[h] - prev[h]).abs() as f64;
                hf[h] += d * d;
            }
            prev.copy_from_slice(&state);
        }
        assert!(hf[0] > hf[1], "head 1 brighter than head 2");
        assert!(hf[1] > hf[2], "head 2 brighter than head 3");
        // A corner above Nyquist bows out instead of going unstable.
        assert!(lp_coeff(100_000.0, 8_000.0) <= 1.0);
    }

    #[test]
    fn the_tone_stage_centred_is_the_wire_through() {
        let mut t = Tone::default();
        let mut seed = 0xabcd_ef01u32;
        for _ in 0..5_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
            // Both knobs centred means both coefficients are zero: exactly
            // the input, not approximately.
            assert_eq!(tone(&mut t, 0.1, 0.2, 0.0, 0.0, x), x);
        }
    }

    #[test]
    fn treble_rides_the_highs_and_bass_the_lows() {
        let level_at = |freq: f32, kb: f32, kt: f32| -> f64 {
            let mut t = Tone::default();
            let mut amp = 0.0f64;
            for i in 0..8_000 {
                let x = (std::f32::consts::TAU * freq * i as f32 / RATE).sin();
                let y = tone(
                    &mut t,
                    lp_coeff(160.0, RATE),
                    lp_coeff(4_500.0, RATE),
                    kb,
                    kt,
                    x,
                );
                if i > 4_000 {
                    amp = amp.max(y.abs() as f64);
                }
            }
            amp
        };
        // 8 kHz: the treble knob moves it, the bass knob barely does.
        let hi_flat = level_at(8_000.0, 0.0, 0.0);
        let hi_up = level_at(8_000.0, 0.0, 0.6);
        let hi_down = level_at(8_000.0, 0.0, -0.6);
        assert!(
            hi_up > hi_flat * 1.1,
            "treble up lifts 8k, {hi_up} vs {hi_flat}"
        );
        assert!(hi_down < hi_flat * 0.9, "treble down shaves 8k");
        assert!(
            (level_at(8_000.0, 0.6, 0.0) - hi_flat).abs() < 0.02 * hi_flat,
            "bass leaves 8k alone"
        );
        // 100 Hz: the bass knob moves it, the treble knob barely does.
        let lo_flat = level_at(100.0, 0.0, 0.0);
        assert!(
            level_at(100.0, 0.6, 0.0) > lo_flat * 1.1,
            "bass up lifts 100"
        );
        assert!(level_at(100.0, -0.6, 0.0) < lo_flat, "bass down cuts 100");
        assert!(
            (level_at(100.0, 0.0, 0.6) - lo_flat).abs() < 0.02 * lo_flat,
            "treble leaves 100 Hz alone"
        );
    }

    #[test]
    fn output_steps_are_the_service_notes_numbers() {
        // −15 / −23 / −35 dBm: eight dB from H to M, twenty from H to L.
        assert_eq!(OUTPUT_TRIM_DB, [0.0, -8.0, -20.0]);
        let (h, m, l) = (output_gain(0), output_gain(1), output_gain(2));
        assert!((20.0 * (h / m).log10() - 8.0).abs() < 1e-3, "H→M is 8 dB");
        assert!((20.0 * (h / l).log10() - 20.0).abs() < 1e-3, "H→L is 20 dB");
        assert!(h > m && m > l && h > 0.99 && h < 1.01, "H is unity");
        // End to end: the same echo heard through M sits eight dB under H.
        let out_of = |level: i32| -> f32 {
            let c = controls_with(|c| {
                c.mode = 2;
                c.intensity = 0.5;
                c.output_level = level;
            });
            let mut e = Echo::new(RATE, &c);
            let o = burst_response(&mut e, &c, (0.42 * RATE) as usize);
            peak_at(&o, tap_seconds(0.0, 2), 0.004)
        };
        let (eh, em) = (out_of(0), out_of(1));
        assert!(
            (20.0 * (eh / em).log10() - 8.0).abs() < 0.5,
            "heard 8 dB apart"
        );
    }

    #[test]
    fn reverb_only_mode_is_the_spring_alone() {
        // Spring muted, Echo Volume up: nothing is left in the clear,
        // because no head is selected — the source's “reverb only mode”.
        let mute = controls_with(|c| {
            c.mode = 11;
            c.reverb_volume = 0.0;
            c.intensity = 0.5;
        });
        let mut e = Echo::new(RATE, &mute);
        let out = burst_response(&mut e, &mute, (0.5 * RATE) as usize);
        for h in 0..3 {
            let p = peak_at(&out, tap_seconds(0.0, h), 0.01);
            assert!(
                p < 1e-5,
                "with the spring muted mode 12 is silent, head {h}: {p}"
            );
        }
        // Spring up, same mode: the burst comes back smeared, not as a
        // discrete echo, and it rings on past the nearest head's time.
        // The window starts after the tank's own latency — the 3.5 ms
        // pre-delay plus the shortest comb is ≈ 17 ms (A-6) — because the
        // tank genuinely is silent before that now.
        let alive = controls_with(|c| {
            c.mode = 11;
            c.reverb_volume = 0.9;
            c.echo_volume = 0.0;
            c.intensity = 0.0;
        });
        let mut e = Echo::new(RATE, &alive);
        let out = burst_response(&mut e, &alive, (0.3 * RATE) as usize);
        let head = rms_in(&out, 0.022, 0.060);
        let tail = rms_in(&out, 0.060, 0.100);
        assert!(head > 0.01, "the tank answers after its own lag: {head}");
        assert!(tail > 0.000_1, "and rings on: {tail}");
        // No head-window check here on purpose: the spring tail rings
        // through every head's time, so silence at those windows would be
        // a false claim. The muted run above is what proves no head is in
        // the mode.
    }

    #[test]
    fn the_spring_smears_without_escaping_its_bound() {
        let c = controls_with(|c| {
            c.mode = 11;
            c.reverb_volume = 1.0;
        });
        let mut e = Echo::new(RATE, &c);
        let out = burst_response(&mut e, &c, (0.5 * RATE) as usize);
        let bound = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            bound <= 1.2,
            "damped comb energy stays bounded, saw {bound}"
        );
        assert!(bound > 0.3, "and the burst is not swallowed, saw {bound}");
    }

    #[test]
    fn the_transport_wobbles_a_little_and_no_more() {
        // A-5 in numbers, widened by the A-11 wear so that the wow can
        // breathe by its stated ±35 %: the modulation exists, it moves
        // over the wow cycle, and it never exceeds the stated depths.
        let c = Controls::default();
        let mut e = Echo::new(RATE, &c);
        let r = recipe(&c);
        let mut seen = Vec::with_capacity(48_000);
        for _ in 0..48_000 {
            seen.push(e.tick(&r));
        }
        let mx = seen.iter().fold(-1e9f32, |m, s| m.max(*s));
        let mn = seen.iter().fold(1e9f32, |m, s| m.min(*s));
        let depth = (WOW_DEPTH_S * (1.0 + WEAR_WOW_DEPTH) + FLUTTER_DEPTH_S) * RATE;
        assert!(
            mx <= depth + 1e-3 && mn >= -depth - 1e-3,
            "never exceeds the stated depths"
        );
        assert!(mx - mn > 3.0, "it really moves, saw {}", mx - mn);
    }

    #[test]
    fn the_transport_wanders_slowly_and_stays_bounded() {
        // A-11: one minutes-long wander, three heads seeing three phases
        // of it. The nominal cycle is 227 s; under plain f32 phase
        // accumulation — the increment is only a few ULPs of the phase —
        // the effective cycle stretches to about 258 s, and the 280 s
        // window here covers a full one of those. Every head's share must
        // sweep nearly the full ±1, yet no single step may jump: the
        // machine ages, it does not shudder.
        let c = Controls::default();
        let mut e = Echo::new(RATE, &c);
        let r = recipe(&c);
        let mut w_min = [1e9f32; 3];
        let mut w_max = [-1e9f32; 3];
        let mut w_at = [0usize; 3];
        let mut step = 0.0f32;
        let mut prev_s = e.wear_s;
        let pc = WEAR_PHASE.map(f32::cos);
        let ps = WEAR_PHASE.map(f32::sin);
        let mut offset = 0.0f32;
        let ticks = (280.0 * RATE) as usize;
        for t in 0..ticks {
            let tick = e.tick(&r);
            offset = offset.max(tick.abs());
            let d = (e.wear_s - prev_s).abs();
            step = step.max(d);
            prev_s = e.wear_s;
            for h in 0..3 {
                let w = e.wear_s * pc[h] + e.wear_c * ps[h];
                if w > w_max[h] {
                    w_max[h] = w;
                    w_at[h] = t;
                }
                w_min[h] = w_min[h].min(w);
            }
        }
        // tick() answers in samples, so the seconds of A-5's widened bound
        // meet the rate before they meet the offset.
        let bound = (WOW_DEPTH_S * (1.0 + WEAR_WOW_DEPTH) + FLUTTER_DEPTH_S) * RATE;
        assert!(
            offset <= bound + 1e-4,
            "the wander stays inside A-5's widened bound"
        );
        assert!(step < 1e-3, "the tape ages, it does not shudder: {step}");
        for h in 0..3 {
            assert!(
                w_max[h] > 0.95 && w_min[h] < -0.95,
                "head {h} saw the whole cycle"
            );
        }
        for a in 0..3 {
            for b in a + 1..3 {
                let gap = w_at[a].abs_diff(w_at[b]) as f32 / RATE;
                assert!(
                    gap > 10.0,
                    "heads peak at different decades, saw {gap} s apart"
                );
            }
        }
    }

    #[test]
    fn the_hiss_breathes_with_the_wow() {
        // A-9 as amended: hiss tracks tape speed, so the noise floor of
        // the echo swell peaks a quarter into the wow cycle and troughs
        // three quarters in. Measured at the head's own delay so we hear
        // the hiss as it was recorded, tap-delayed, nothing else added.
        let c = controls_with(|_| {});
        let mut e = Echo::new(RATE, &c);
        let mut out = Vec::with_capacity((2.0 * RATE) as usize);
        let buf = [0.0f32; BLOCK];
        let mut o = [0.0f32; BLOCK];
        for _ in 0..(2.0 * RATE) as usize / BLOCK {
            e.process_mono(&c, &buf, &mut o);
            out.extend_from_slice(&o);
        }
        let tap = tap_seconds(0.0, 0);
        let rms = |t: f32| rms_in(&out, t - 0.08 + tap, t + 0.08 + tap);
        let peak = rms(0.25 / WOW_HZ);
        let trough = rms(0.75 / WOW_HZ);
        assert!(trough > 0.0, "the floor is there at all");
        // ±40 % breath says 2.33:1; the claim fails well before that.
        assert!(
            peak > 1.5 * trough,
            "the hiss breathes: peak {peak}, trough {trough}"
        );
    }

    #[test]
    fn the_record_path_loses_top_when_driven() {
        // A-12: the same noise through the record path, quiet and pushed.
        // Pushed tape loses top end, so the high-to-low energy ratio of
        // what comes back off the head must fall when the input is loud.
        let c = controls_with(|c| {
            c.echo_volume = 1.0;
        });
        let measure = |amp: f32| -> f64 {
            let mut e = Echo::new(RATE, &c);
            let buf_len = (0.5 * RATE) as usize;
            let mut out = Vec::with_capacity(buf_len);
            let mut seed = 0xf00d_u32;
            let mut buf = [0.0f32; BLOCK];
            let mut o = [0.0f32; BLOCK];
            for _ in 0..buf_len / BLOCK {
                for s in buf.iter_mut() {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904);
                    *s = ((seed >> 8) as f32 / 8_388_608.0 - 1.0) * amp;
                }
                e.process_mono(&c, &buf, &mut o);
                out.extend_from_slice(&o);
            }
            // Skip the fill; the drive envelope's τ is ≈ 21 ms.
            let seen = &out[(0.25 * RATE) as usize..];
            let band_energy = || -> f64 {
                let mut lp6 = 0.0f32;
                let mut lp8 = 0.0f32;
                let a6 = lp_coeff(600.0, RATE);
                let a8 = lp_coeff(8_000.0, RATE);
                let mut lf = 0.0f64;
                let mut hf = 0.0f64;
                for &x in seen {
                    lp6 += a6 * (x - lp6);
                    lp8 += a8 * (x - lp8);
                    lf += (lp6 * lp6) as f64;
                    hf += ((x - lp8) * (x - lp8)) as f64;
                }
                hf / lf
            };
            band_energy()
        };
        let quiet = measure(0.12);
        // Pushed means into the red: mean |drive| ≈ 0.75, so the record
        // corner sits close to A-12's fully-driven 4 kHz. Measured drift
        // at this level is about −3 dB of treble against the clean run.
        let loud = measure(1.5);
        let drift = 10.0 * (loud / quiet).log10();
        assert!(
            drift < -1.5,
            "driven tape is duller, saw {drift:.2} dB of treble drift"
        );
    }

    #[test]
    fn the_spring_lags_before_it_rings() {
        // A-6 as amended: the tank has a pre-delay in front of the combs.
        // Right after the burst arrives the output is genuinely empty —
        // a smear that starts instantly is a reverb nobody hears on a
        // spring line.
        let c = controls_with(|c| {
            c.mode = 11;
            c.reverb_volume = 1.0;
            c.echo_volume = 0.0;
        });
        let mut e = Echo::new(RATE, &c);
        let out = burst_response(&mut e, &c, (0.2 * RATE) as usize);
        let t0 = 4.0 * BLOCK as f32 / RATE; // when the burst hits
        let early = rms_in(&out, t0 + 0.000_4, t0 + 0.002_4);
        let onset = rms_in(&out, t0 + 0.006, t0 + 0.040);
        assert!(onset > 0.01, "the window really contains the ring: {onset}");
        assert!(
            early < 0.02 * onset,
            "the spring waits for its pre-delay: {early} vs {onset}"
        );
    }

    #[test]
    fn the_spring_rings_down_bright_first() {
        // A-6 as amended: each comb low-passes its own feedback, so the
        // ring loses treble as it decays. Compare the 4 kHz bin against
        // the 200 Hz bin early and late; late, the treble bin must have
        // collapsed toward the bass bin.
        fn goertzel(x: &[f32], k: usize) -> f64 {
            let n = x.len() as f32;
            let c2 = 2.0 * (std::f32::consts::TAU * k as f32 / n).cos() as f64;
            let (mut s1, mut s2) = (0.0f64, 0.0f64);
            for &v in x {
                let s0 = v as f64 + c2 * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            s1 * s1 + s2 * s2 - c2 * s1 * s2
        }
        let c = controls_with(|c| {
            c.mode = 11;
            c.reverb_volume = 1.0;
            c.echo_volume = 0.0;
        });
        let mut e = Echo::new(RATE, &c);
        let out = burst_response(&mut e, &c, (0.45 * RATE) as usize);
        let t0 = 4.0 * BLOCK as f32 / RATE;
        let window = |from: f32| {
            let a = ((t0 + from) * RATE) as usize;
            &out[a..a + 2400]
        };
        let ratio = |from: f32| goertzel(window(from), 200) / goertzel(window(from), 10);
        let early = ratio(0.02);
        let late = ratio(0.30);
        assert!(early > 1e-6, "the early window rings: {early}");
        assert!(
            late < early * 0.2,
            "the ring darkens as it dies: early {early:.3}, late {late:.6}"
        );
    }

    #[test]
    fn turning_the_repeat_rate_smears_the_delay_on_the_way() {
        let slow = controls_with(|c| {
            c.mode = 0;
            c.repeat_rate = 0.0;
            c.intensity = 0.0;
        });
        let fast = controls_with(|c| {
            c.mode = 0;
            c.repeat_rate = 1.0;
            c.intensity = 0.0;
        });
        let mut e = Echo::new(RATE, &slow);
        run_silence(&mut e, &slow, (0.5 * RATE) as usize);
        // Mid-twist — a quarter-second in, while the speed is still
        // travelling — a burst must land between the two delays.
        run_silence(&mut e, &fast, (0.25 * RATE) as usize);
        let out = burst_response(&mut e, &fast, (0.17 * RATE) as usize);
        let t_old = tap_seconds(0.0, 0);
        let t_new = tap_seconds(1.0, 0);
        let mut at = 0.010;
        let mut found = None;
        while at < 0.155 && found.is_none() {
            if peak_at(&out, at, 0.004) > 0.4 {
                found = Some(at);
            }
            at += 0.002;
        }
        let heard = found.expect("the echo arrives while the speed is mid-slew");
        assert!(
            heard < t_old * 0.999 && heard > t_new * 1.05,
            "mid-twist delay sits between old ({t_old}) and new ({t_new}), heard {heard}"
        );
    }

    #[test]
    fn one_tape_is_heard_in_both_lanes() {
        // The stereo run and the mono run are the same machine: lane 0 of
        // the stereo pass equals the mono pass sample for sample, and both
        // lanes hear the same echo (mono tape — TODO.md owns the spread).
        let c = Controls {
            mode: 10,
            intensity: 0.4,
            echo_switch: 0,
            ..Default::default()
        };
        let mut mono = Echo::new(RATE, &c);
        let mut st = Echo::new(RATE, &c);
        let mut src = [0.0f32; BLOCK];
        let mut seed = 7u32;
        for v in src.iter_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *v = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
        }
        let mut m_out = [0.0f32; BLOCK];
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        mono.process_mono(&c, &src, &mut m_out);
        st.process_stereo(&c, [&src, &src], [&mut l, &mut r]);
        for i in 0..BLOCK {
            assert_eq!(m_out[i], l[i], "mono lane is lane 0 of the stereo run");
            assert_eq!(l[i], r[i], "both lanes hear one tape");
        }
    }
}
