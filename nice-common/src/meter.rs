//! Input-meter ballistics, as pure state over (rms, peak, clock).
//!
//! Kept free of GPU/threading so every rule the user specified is nailed
//! down by unit tests instead of by eye:
//!
//! * the bar chases RMS with a fast attack and a moderate release;
//! * the peak marker jumps to any input exceeding the CURRENTLY SHOWN
//!   marker and re-arms the 1 s stick — that is the only arming condition;
//! * after 1 s without an exceedance the marker falls at 24 dB/s (real
//!   wall-clock dt, frame-rate proof), floored at the bar so it can never
//!   display below the level it is measuring;
//! * the whole meter goes red while the shown marker is above 0 dBFS, which
//!   is the one thing the -48..0 scale cannot show by position
//!   ([`PeakMeter::clipping`]);
//! * a meter built by [`PeakMeter::latching`] holds that red after the marker
//!   has fallen back, until someone acknowledges it with
//!   [`PeakMeter::clear_clip`] — the rack's rule, not a plugin's;
//! * silence drains both layers to -120 dBFS.
//!
//! Both levels are expected to be true peak in dBTP — what
//! [`crate::true_peak`] reports — not the largest sample, which is a
//! different and always lower number.

/// Peak-hold level meter. All inputs are dBFS.
#[derive(Clone, Copy, Debug)]
pub struct PeakMeter {
    level_db: f32,
    peak_db: f32,
    stick_until_ms: u64,
    last_ms: u64,
    /// Whether a clip is remembered rather than allowed to fade away.
    latch: bool,
    /// A clip seen while `latch` was on, held until [`Self::clear_clip`].
    latched_clip: bool,
}

/// Hold time after the marker was last beaten by the input.
pub const STICK_MS: u64 = 1000;
/// Marker fall rate, dB per second.
pub const FALL_DB_PER_S: f32 = 24.0;
/// Value treated as digital silence.
pub const SILENCE_DB: f32 = -120.0;
/// The top of the scale, and the line above which the meter goes red. It is
/// the top of the drawn range too, so a true peak over it has nowhere to be
/// plotted — colour is the only channel left to say "beyond full scale".
pub const CLIP_DB: f32 = 0.0;

impl PeakMeter {
    pub fn new(now_ms: u64) -> Self {
        Self {
            level_db: SILENCE_DB,
            peak_db: SILENCE_DB,
            stick_until_ms: 0,
            last_ms: now_ms,
            latch: false,
            latched_clip: false,
        }
    }

    /// A meter that remembers its clips until they are acknowledged with
    /// [`Self::clear_clip`]. For a meter whose red state outlives the frame
    /// that caused it, because someone has to be the one to say "yes, I saw
    /// it" — the rack, where a clip two plugins upstream is worth keeping
    /// until the operator has read it. A plain [`Self::new`] forgets on its
    /// own, which is what a plugin's own meter wants.
    pub fn latching(now_ms: u64) -> Self {
        let mut meter = Self::new(now_ms);
        meter.latch = true;
        meter
    }

    /// Advance one frame. `rms_db` is this frame's RMS level (SILENCE_DB if
    /// no audio), `true_peak_db` this frame's true peak in dBTP — from
    /// [`crate::true_peak`], not the largest sample (SILENCE_DB if none) — and
    /// `now_ms` the wall clock.
    pub fn tick(&mut self, rms_db: f32, true_peak_db: f32, now_ms: u64) {
        let dt = now_ms.saturating_sub(self.last_ms).min(200) as f32 / 1000.0;
        self.last_ms = now_ms;

        // Bar: RMS with fast attack / moderate release.
        let k = if rms_db > self.level_db { 0.5 } else { 0.08 };
        self.level_db += (rms_db - self.level_db) * k;

        // Marker: exceedance of the SHOWN value sticks it; else, after the
        // stick lapses, a constant-rate fall floored at the bar.
        if true_peak_db > self.peak_db {
            self.peak_db = true_peak_db;
            self.stick_until_ms = now_ms + STICK_MS;
        } else if now_ms >= self.stick_until_ms {
            self.peak_db = (self.peak_db - FALL_DB_PER_S * dt).max(self.level_db);
        }

        // Latching is the only way this bit is ever set, so a meter built with
        // `new` sees it stay false forever and keeps the self-clearing rule.
        if self.latch && self.peak_db > CLIP_DB {
            self.latched_clip = true;
        }
    }

    pub fn level_db(&self) -> f32 {
        self.level_db
    }

    pub fn peak_db(&self) -> f32 {
        self.peak_db
    }

    /// Whether the meter is showing a true peak above full scale.
    ///
    /// Read off the *shown* marker rather than the last frame's input, which
    /// is what makes the red state behave like the marker it colours instead
    /// of like a second, faster meter: a peak over 0 dBTP lights the whole
    /// meter, holds for `STICK_MS`, then goes out on its own as the marker
    /// falls at `FALL_DB_PER_S` — no extra state, no reset to remember, and no
    /// way for the red to be lit while the marker it explains sits lower than
    /// the top of the scale.
    ///
    /// Strictly greater, so a signal that reaches exactly 0 dBTP without
    /// passing it reads as full scale and not as clipped.
    ///
    /// A meter built by [`Self::latching`] answers something narrower than
    /// "is it over the top right now": it answers "has it gone over, and has
    /// nobody said so yet". Once it has been over, this stays true after the
    /// marker has long fallen back under the top, and only
    /// [`Self::clear_clip`] ends it. The colour still comes from here, so a
    /// latched meter is a red meter: the marker's own fade takes the red with
    /// it on an unlatched one and leaves it standing on a latched one.
    pub fn clipping(&self) -> bool {
        self.latched_clip || self.peak_db > CLIP_DB
    }

    /// Forget a clip, ending the red that [`Self::latching`] would otherwise
    /// hold.
    ///
    /// The shown marker is brought back to the top of the scale with it.
    /// Without that, acknowledging a clip three dB over would leave the meter
    /// red for another second while the marker fell, which reads as a click
    /// that did nothing: the red is the channel for *beyond* full scale, so an
    /// acknowledgement retires exactly that much of the marker. Anything the
    /// audio is putting over the top *now* is re-read and re-latched by the
    /// next [`Self::tick`] — an ongoing clip cannot be clicked away, only a
    /// past one.
    pub fn clear_clip(&mut self) {
        self.latched_clip = false;
        self.peak_db = self.peak_db.min(CLIP_DB);
    }
}

#[cfg(feature = "widgets")]
/// Every colour [`PeakMeter::draw`] puts on screen, chosen in one place so
/// the decision is a value a test can read instead of a frame a human has to
/// look at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterColors {
    /// The rail the bar runs in.
    pub track: frost::Color,
    /// The nine scale ticks.
    pub tick: frost::Color,
    /// The dBFS numerals under them.
    pub numeral: frost::Color,
    /// The bar itself — the caller's amber→green `main`, or red.
    pub bar: frost::Color,
    /// The held peak marker.
    pub marker: frost::Color,
}

#[cfg(feature = "widgets")]
impl PeakMeter {
    /// The colours this meter will be drawn in: the house neutrals plus the
    /// caller's `main`, or red through and through while [`Self::clipping`].
    ///
    /// Red means the *whole* meter and not a lamp beside it, for the reason
    /// the caller cannot fix locally: every consumer draws the bar, marker,
    /// ticks and numerals from here, so one decision here is eight meters
    /// behaving the same way, and a red track with red numerals cannot be
    /// mistaken for the amber of a held peak.
    pub fn colors(&self, main: frost::Color) -> MeterColors {
        use frost::Color;
        if self.clipping() {
            MeterColors {
                track: Color {
                    a: 0.5,
                    ..CLIP_TRACK
                },
                tick: Color { a: 0.6, ..CLIP },
                numeral: CLIP,
                bar: CLIP,
                marker: Color {
                    a: 0.95,
                    ..CLIP_MARKER
                },
            }
        } else {
            MeterColors {
                track: Color { a: 0.5, ..TRACK },
                tick: Color { a: 0.6, ..DIM },
                numeral: DIM,
                bar: main,
                marker: Color { a: 0.9, ..FG },
            }
        }
    }

    /// Draw the input RMS bar, peak hold and -48..0 dBFS scale. Returns whether
    /// the pointer is over the bar or its scale, for a caller's gate control.
    pub fn draw(
        &self,
        ctx: &mut frost::Context,
        font: &std::sync::Arc<[u8]>,
        center: [f32; 2],
        half_width: f32,
        fit: f32,
        main: frost::Color,
    ) -> bool {
        let c = self.colors(main);
        let [cx, y] = center;
        let half = half_width;
        ctx.rectangle(cx, y, half, 2.5 * fit, c.track, 0.05);
        // dBFS scale: 0 at the right end, -48 at the left, every 6 dB.
        // Short ticks (±4) with numerals well below — digit tops must
        // not crowd the ticks.
        for i in 0..=8u32 {
            let xv = cx + half - i as f32 * (half / 4.0);
            ctx.rectangle(xv, y, 1.0 * fit, 4.0 * fit, c.tick, 0.06);
            ctx.text(
                xv,
                y - 26.0 * fit,
                font,
                format!("{}", -(6 * i as i32)),
                16.0 * fit,
                550.0,
                c.numeral,
                0.06,
            );
        }
        // Bar: current level.
        let m = self.level_db().clamp(-48.0, 0.0);
        let xv = half * (1.0 + m / 24.0);
        if xv > -half + 0.5 {
            ctx.rectangle(
                cx + (-half + xv) / 2.0,
                y,
                (xv + half) / 2.0,
                2.5 * fit,
                // Same amber→green state as note and needle (main lerps),
                // overridden by red the moment the peak is over full scale.
                c.bar,
                0.07,
            );
        }
        // Peak marker: a thin bright line at the held peak, above the
        // bar in z; sits at the bar while rising, sticks after it falls.
        let p = self.peak_db().clamp(-48.0, 0.0);
        let px = half * (1.0 + p / 24.0);
        if p > -47.5 {
            ctx.rectangle(cx + px, y, 1.5 * fit, 6.0 * fit, c.marker, 0.08);
        }

        ctx.mouse_position().is_some_and(|[mx, my]| {
            (mx - cx).abs() <= half && (my - (y - 13.0 * fit)).abs() <= 17.0 * fit
        })
    }
}
#[cfg(feature = "widgets")]
const TRACK: frost::Color = frost::Color {
    r: 42.0 / 255.0,
    g: 36.0 / 255.0,
    b: 61.0 / 255.0,
    a: 1.0,
};
#[cfg(feature = "widgets")]
const DIM: frost::Color = frost::Color {
    r: 107.0 / 255.0,
    g: 114.0 / 255.0,
    b: 128.0 / 255.0,
    a: 1.0,
};
#[cfg(feature = "widgets")]
const FG: frost::Color = frost::Color {
    r: 230.0 / 255.0,
    g: 230.0 / 255.0,
    b: 234.0 / 255.0,
    a: 1.0,
};
/// The house red, so a meter that is over full scale is the same red as
/// anything else in the family that says so.
#[cfg(feature = "widgets")]
const CLIP: frost::Color = crate::knob::RED;
/// `TRACK` turned red: the rail still has to sit under the bar, and a bright
/// track would leave the bar nothing to be darker than.
#[cfg(feature = "widgets")]
const CLIP_TRACK: frost::Color = frost::Color {
    r: 78.0 / 255.0,
    g: 28.0 / 255.0,
    b: 36.0 / 255.0,
    a: 1.0,
};
/// `FG` turned hot: the marker is the one element carrying the number, so it
/// keeps its brightness and takes only the hue.
#[cfg(feature = "widgets")]
const CLIP_MARKER: frost::Color = frost::Color {
    r: 1.0,
    g: 154.0 / 255.0,
    b: 146.0 / 255.0,
    a: 1.0,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive `n` frames of `step_ms` apart at constant levels.
    fn hold(m: &mut PeakMeter, rms: f32, peak: f32, n: u32, step_ms: u64, t: &mut u64) {
        for _ in 0..n {
            *t += step_ms;
            m.tick(rms, peak, *t);
        }
    }

    #[test]
    fn first_exceedance_jumps_and_sticks() {
        let mut t = 0u64;
        let mut m = PeakMeter::new(t);
        hold(&mut m, -30.0, -12.0, 1, 16, &mut t);
        assert_eq!(m.peak_db(), -12.0, "instant rise");
        hold(&mut m, -30.0, -40.0, 30, 16, &mut t); // ~480 ms of quieter input
        assert_eq!(m.peak_db(), -12.0, "sticks through the whole second");
    }

    #[test]
    fn falls_after_exactly_one_second_of_no_exceedance() {
        let mut m = PeakMeter::new(0);
        let t = 16u64;
        m.tick(-30.0, -12.0, t);
        // Still within the stick: frozen.
        for dt_ms in [300, 600, 900] {
            m.tick(-30.0, -40.0, 16 + dt_ms);
            assert_eq!(m.peak_db(), -12.0, "frozen at {dt_ms} ms");
        }
        // Past the stick: falling.
        m.tick(-30.0, -40.0, 16 + 1100);
        assert!(m.peak_db() < -12.0, "must have started falling");
    }

    #[test]
    fn fall_rate_is_24_db_per_second_and_floors_at_bar() {
        let mut t = 0u64;
        let mut m = PeakMeter::new(t);
        // Loud strum (peak -12) over a quiet sustain (RMS -50, so the bar
        // floor cannot truncate the one-second fall measurement).
        m.tick(-50.0, -12.0, 16);
        assert_eq!(m.peak_db(), -12.0);
        // Let the bar settle to the sustain level while the stick holds.
        hold(&mut m, -50.0, -80.0, 40, 16, &mut t); // 640 ms < stick
        assert_eq!(m.peak_db(), -12.0, "still stuck during settle");
        // Past the stick, fall for ~1 s in small steps: 24 dB. The stick
        // expires at t = 1016 ms; frames 63..=125 (t 1024..2016) fall for
        // 1008 ms ⇒ 24.2 dB. (Settle ran 40 frames, so 85 more.)
        hold(&mut m, -50.0, -80.0, 85, 16, &mut t);
        let drop = -12.0 - m.peak_db();
        assert!(
            (drop - FALL_DB_PER_S).abs() < 0.6,
            "one second should drop ~24 dB, dropped {drop}"
        );
        // Keep falling: pinned at the bar, never below.
        hold(&mut m, -50.0, -80.0, 400, 16, &mut t);
        assert!(
            (m.peak_db() - m.level_db()).abs() < 0.5,
            "marker floors at the bar: peak {}, level {}",
            m.peak_db(),
            m.level_db()
        );
    }

    #[test]
    fn exceedance_mid_fall_jumps_and_rearms() {
        let mut t = 0u64;
        let mut m = PeakMeter::new(t);
        m.tick(-40.0, -10.0, 16);
        hold(&mut m, -40.0, -70.0, 130, 16, &mut t); // past stick: falling
        let shown = m.peak_db();
        assert!(shown < -10.0, "should be mid-fall, got {shown}");
        // A strum BELOW the original peak but ABOVE the shown marker.
        t += 16;
        m.tick(-30.0, -18.0, t);
        assert_eq!(m.peak_db(), -18.0, "jumps to the new, lower max");
        // Fresh stick: frozen a full second...
        m.tick(-30.0, -70.0, t + 900);
        assert_eq!(m.peak_db(), -18.0, "re-armed: still frozen at +900 ms");
        // ...then falling again.
        m.tick(-30.0, -70.0, t + 1100);
        assert!(m.peak_db() < -18.0, "falling again after the fresh second");
    }

    #[test]
    fn red_means_the_marker_is_past_the_top_of_the_scale() {
        let mut m = PeakMeter::new(0);
        m.tick(-20.0, CLIP_DB, 16);
        assert!(
            !m.clipping(),
            "exactly 0 dBTP reaches full scale, it does not pass it"
        );
        m.tick(-20.0, CLIP_DB + 0.2, 32);
        assert!(m.clipping(), "0.2 dB over has nowhere left to be plotted");
    }

    #[test]
    fn the_red_holds_with_the_marker_and_clears_itself() {
        let mut t = 0u64;
        let mut m = PeakMeter::new(t);
        hold(&mut m, -30.0, CLIP_DB + 1.6, 1, 16, &mut t);
        assert!(m.clipping(), "a 1.6 dBTP crest lights it at once");
        // 960 ms of quiet input: inside the stick, so still red...
        hold(&mut m, -30.0, -30.0, 60, 16, &mut t);
        assert!(m.clipping(), "the stick holds the red with the marker");
        // ...and gone once the marker has fallen back under the top, with no
        // reset anywhere: the ballistics are the whole clear path.
        hold(&mut m, -30.0, -30.0, 200, 16, &mut t);
        assert!(
            !m.clipping(),
            "the meter cleared itself; marker still at {}",
            m.peak_db()
        );
        assert!(m.peak_db() <= CLIP_DB);
    }

    #[test]
    fn silence_drains_to_the_floor() {
        let mut t = 0u64;
        let mut m = PeakMeter::new(t);
        m.tick(-20.0, -6.0, 16);
        hold(&mut m, SILENCE_DB, SILENCE_DB, 400, 16, &mut t);
        assert!(m.level_db() < -100.0, "bar drains, got {}", m.level_db());
        assert!(m.peak_db() < -100.0, "marker drains, got {}", m.peak_db());
    }

    // --- the latch, which is the rack's rule and nobody else's ------------

    /// The same clip fed to a plain meter and a latching one, one shared
    /// clock, so the only difference between them is the latch itself.
    fn clip_into(plain: &mut PeakMeter, latched: &mut PeakMeter, t: &mut u64) {
        for i in 0..251 {
            *t += 16;
            let peak = if i == 0 { CLIP_DB + 1.6 } else { -30.0 };
            plain.tick(-30.0, peak, *t);
            latched.tick(-30.0, peak, *t);
        }
    }

    #[test]
    fn a_latched_meter_holds_the_red_the_other_one_drops() {
        let mut t = 0u64;
        let mut plain = PeakMeter::new(t);
        let mut latched = PeakMeter::latching(t);
        clip_into(&mut plain, &mut latched, &mut t);

        assert!(
            latched.peak_db() < CLIP_DB,
            "the marker fell back under the top on its own, as always"
        );
        assert!(
            (plain.peak_db() - latched.peak_db()).abs() < 1e-5,
            "latching changed the ballistics: {} vs {}",
            plain.peak_db(),
            latched.peak_db()
        );
        assert!(!plain.clipping(), "the plain one cleared itself");
        assert!(
            latched.clipping(),
            "the latched one still says over full scale, marker at {}",
            latched.peak_db()
        );
    }

    #[test]
    fn a_latch_survives_the_meter_falling_to_silence() {
        let mut t = 0u64;
        let mut m = PeakMeter::latching(t);
        hold(&mut m, -30.0, CLIP_DB + 0.4, 1, 16, &mut t);
        hold(&mut m, SILENCE_DB, SILENCE_DB, 600, 16, &mut t);
        assert!(
            m.clipping(),
            "the clip happened, whatever the meter is showing now"
        );
        m.clear_clip();
        assert!(!m.clipping(), "and the click is what ends it");
    }

    #[test]
    fn acknowledging_returns_the_colour_at_once() {
        let mut t = 0u64;
        let mut m = PeakMeter::latching(t);
        hold(&mut m, -30.0, CLIP_DB + 3.0, 1, 16, &mut t);
        assert!(m.clipping());
        // Inside the stick, so the marker itself is still 3 dB over.
        hold(&mut m, -30.0, -30.0, 10, 16, &mut t);
        m.clear_clip();
        assert!(
            !m.clipping(),
            "a click that left the meter red for another second would read \
             as a click that did nothing"
        );
        assert_eq!(
            m.peak_db(),
            CLIP_DB,
            "the marker keeps the top, loses the overhang"
        );
    }

    #[test]
    fn a_clip_that_is_still_happening_cannot_be_clicked_away() {
        let mut t = 0u64;
        let mut m = PeakMeter::latching(t);
        hold(&mut m, -20.0, CLIP_DB + 2.0, 2, 16, &mut t);
        m.clear_clip();
        hold(&mut m, -20.0, CLIP_DB + 2.0, 1, 16, &mut t);
        assert!(
            m.clipping(),
            "the next frame re-reads the same overage and re-latches it"
        );
    }

    #[test]
    fn nothing_is_remembered_that_was_never_asked_for() {
        // A plain meter acknowledges nothing because it never held anything:
        // `clear_clip` is a no-op on it, not a way to silence a live clip.
        let mut m = PeakMeter::new(0);
        m.tick(-20.0, CLIP_DB + 1.0, 16);
        m.clear_clip();
        assert!(m.peak_db() <= CLIP_DB);
        m.tick(-20.0, CLIP_DB + 1.0, 32);
        assert!(m.clipping(), "still live, so still red");
    }
}

/// The colour choice is the user's rule, so it is tested rather than looked
/// at. Gated on `widgets` because the palette needs the GPU colour type.
#[cfg(all(test, feature = "widgets"))]
mod colour {
    use super::*;

    #[test]
    fn a_meter_with_headroom_keeps_the_house_neutrals() {
        let m = PeakMeter::new(0);
        let c = m.colors(crate::knob::GREEN);
        assert_eq!(c.bar, crate::knob::GREEN, "the caller's main is the bar");
        assert_eq!(c.numeral, DIM);
        assert_eq!(c.tick, frost::Color { a: 0.6, ..DIM });
        assert_eq!(c.track, frost::Color { a: 0.5, ..TRACK });
        assert_eq!(c.marker, frost::Color { a: 0.9, ..FG });
    }

    #[test]
    fn past_full_scale_every_element_turns_red() {
        let mut m = PeakMeter::new(0);
        m.tick(-20.0, CLIP_DB + 0.5, 16);
        let c = m.colors(crate::knob::GREEN);
        assert_eq!(c.bar, CLIP, "bar");
        assert_eq!(c.numeral, CLIP, "numerals");
        assert_eq!(c.tick, frost::Color { a: 0.6, ..CLIP }, "ticks");
        assert_eq!(
            c.track,
            frost::Color {
                a: 0.5,
                ..CLIP_TRACK
            },
            "track"
        );
        assert_eq!(
            c.marker,
            frost::Color {
                a: 0.95,
                ..CLIP_MARKER
            },
            "marker"
        );
        // The whole meter, not a lamp beside it: nothing of the resting
        // palette survives the crossing, whatever colour the caller passed.
        assert_ne!(c, PeakMeter::new(0).colors(crate::knob::GREEN));
        assert_ne!(c, PeakMeter::new(0).colors(crate::knob::AMBER));
        // And it is the house red, which is not the amber of a held peak.
        assert_eq!(CLIP, crate::knob::RED);
        assert_ne!(CLIP, crate::knob::AMBER);
    }

    #[test]
    fn a_latch_keeps_the_meter_red_after_the_marker_falls() {
        // The rack's whole point: the marker is back inside the scale and the
        // meter still says "this went over", because nobody acknowledged it.
        let mut t = 0u64;
        let mut m = PeakMeter::latching(t);
        m.tick(-20.0, CLIP_DB + 2.0, 16);
        for _ in 0..250 {
            t += 16;
            m.tick(-20.0, -60.0, t);
        }
        assert!(m.peak_db() < CLIP_DB, "marker back in the scale");

        let mut live = PeakMeter::new(0);
        live.tick(-20.0, CLIP_DB + 0.5, 16);
        assert_eq!(
            m.colors(crate::knob::GREEN),
            live.colors(crate::knob::GREEN),
            "a remembered clip and a live one look identical, which is the \
             point of remembering"
        );

        m.clear_clip();
        assert_eq!(
            m.colors(crate::knob::GREEN),
            PeakMeter::new(0).colors(crate::knob::GREEN),
            "and the click puts the resting palette back"
        );
    }

    #[test]
    fn the_colour_follows_the_state_and_nothing_else() {
        // Same crossing reached two ways — straight over, or up to it after a
        // louder hit fell back — must look the same, because both are red.
        let mut direct = PeakMeter::new(0);
        direct.tick(-20.0, 0.5, 16);
        let mut rearmed = PeakMeter::new(0);
        rearmed.tick(-20.0, 6.0, 16);
        // Fall back under 0 with 16 ms steps — `tick` caps dt at 200 ms, so a
        // single far-future tick would fall one cap's worth, not the whole way.
        for t in (1_032..=1_400).step_by(16) {
            rearmed.tick(-20.0, -60.0, t);
        }
        assert!(rearmed.peak_db() < 0.0, "must be back under full scale");
        // A smaller crest, still over the top, beats the shown marker.
        rearmed.tick(-20.0, 0.5, 1_416);
        assert!(direct.clipping() && rearmed.clipping());
        assert_eq!(direct.peak_db(), rearmed.peak_db());
        assert_eq!(
            direct.colors(crate::knob::GREEN),
            rearmed.colors(crate::knob::GREEN)
        );
    }
}

/// Energy and inter-sample peak of a bounded planar block. Channel order and
/// estimator history are preserved across calls; RMS normalization is the
/// caller's policy. Shared by the mono/stereo plugin processing adapters.
pub fn block_stats<const N: usize>(
    scratch: &[[f32; N]; 2],
    channels: usize,
    n: usize,
    tp: &mut [crate::true_peak::TruePeak; 2],
) -> (f64, f32) {
    let mut energy = 0.0f64;
    let mut peak = 0.0f32;
    for (channel, meter) in scratch[..channels].iter().zip(&mut tp[..channels]) {
        meter.process(&channel[..n]);
        peak = peak.max(meter.take());
        energy += channel[..n]
            .iter()
            .map(|&x| f64::from(x).powi(2))
            .sum::<f64>();
    }
    (energy, peak)
}
