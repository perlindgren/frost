//! One block-budget meter per rack card. Purple is time already consumed;
//! green is this card, amber is approaching the deadline, red is over budget.
use frost::{Color, Context};
use nice_rack_engine::audio::ProcessingTime;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct TimingMeter {
    value: ProcessingTime,
    live: bool,
    overload: f32,
}
impl TimingMeter {
    pub fn tick(&mut self, sample: Option<ProcessingTime>, dt: f32) {
        let dt = dt.clamp(0.0, 0.25);
        self.overload = (self.overload - dt).max(0.0);
        if let Some(value) = sample {
            if value.total() >= 1.0 {
                self.overload = 0.5;
            }
            let mix = if self.live {
                1.0 - (-f64::from(dt) / 0.08).exp()
            } else {
                1.0
            };
            self.value.incoming += (value.incoming - self.value.incoming) * mix;
            self.value.added += (value.added - self.value.added) * mix;
            self.live = true;
        } else {
            self.live = false;
            self.value = ProcessingTime::default();
            self.overload = 0.0;
        }
    }
    pub fn draw(
        &self,
        ctx: &mut Context,
        font: &Arc<[u8]>,
        [x, y]: [f32; 2],
        width: f32,
        details: bool,
    ) {
        use nice_common::knob::{AMBER, GREEN, MUTED, PANEL, RED};
        let at = y - 54.0;
        let left = x - width / 2.0;
        ctx.rectangle(x, at, width / 2.0, 2.0, PANEL, 0.23);
        let (incoming, added) = self.value.segments();
        let color = if self.value.total() >= 1.0 {
            RED
        } else if self.value.total() >= 0.85 {
            AMBER
        } else {
            GREEN
        };
        for (start, fraction, color) in [(0.0, incoming, MUTED), (incoming, added, color)] {
            if fraction > 0.0 {
                let length = fraction * width;
                ctx.rectangle(
                    left + start * width + length / 2.0,
                    at,
                    length / 2.0,
                    2.0,
                    color,
                    0.24,
                );
            }
        }
        if self.overload > 0.0 {
            ctx.rectangle(x + width / 2.0 - 1.5, at, 1.5, 3.0, RED, 0.25);
        }
        if !details {
            return;
        }
        let Some([mx, my]) = ctx.mouse_position() else {
            return;
        };
        let (w, h) = ctx.size();
        let half = (w / 2.0 - 16.0).min(250.0);
        let tx = (mx + half + 15.0).clamp(-w / 2.0 + half + 8.0, w / 2.0 - half - 8.0);
        let ty = (my + 65.0).clamp(-h / 2.0 + 60.0, h / 2.0 - 60.0);
        ctx.rectangle(
            tx,
            ty,
            half,
            52.0,
            Color {
                r: 13.0 / 255.0,
                g: 10.0 / 255.0,
                b: 22.0 / 255.0,
                a: 1.0,
            },
            0.9,
        );
        let rows = if self.live {
            [
                format!("Audio budget: {:.1}% used", self.value.total() * 100.0),
                format!(
                    "Before {:.1}%  + card {:.1}%",
                    self.value.incoming * 100.0,
                    self.value.added * 100.0
                ),
                "100% = one audio block's deadline".into(),
            ]
        } else {
            [
                "Audio budget".into(),
                "No live processing timing".into(),
                "Purple: before  |  Green: this card".into(),
            ]
        };
        for (i, row) in rows.into_iter().enumerate() {
            ctx.text(
                tx,
                ty + 26.0 - i as f32 * 24.0,
                font,
                row,
                13.0,
                600.0,
                if i == 0 { color } else { MUTED },
                0.91,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smoothing_preserves_the_sum_and_overload_holds_then_clears_on_stop() {
        let mut meter = TimingMeter::default();
        meter.tick(
            Some(ProcessingTime {
                incoming: 0.2,
                added: 0.1,
            }),
            0.016,
        );
        meter.tick(
            Some(ProcessingTime {
                incoming: 0.8,
                added: 0.3,
            }),
            0.016,
        );
        assert!(meter.value.total() > 0.3 && meter.value.total() < 1.1);
        assert_eq!(meter.overload, 0.5);
        meter.tick(None, 0.016);
        assert_eq!(meter.value.total(), 0.0);
        assert!(!meter.live);
        assert_eq!(meter.overload, 0.0);
    }
}
