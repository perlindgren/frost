//! CAT, inspired by the Pro Co RAT circuit; nominal LM308 macromodel, not a transistor model.
use super::*;

pub(super) struct Cat {
    amp: Network<6, 5>,
    clip: Network<4, 3>,
    output: Network<2, 1>,
    previous: f64,
    slew: f64,
}
impl Cat {
    pub(super) fn new(rate: f64, controls: [f64; 3]) -> Self {
        let mut amp = Network::new([
            Capacitor::new(S(0), N(0), 22e-9, rate),
            Capacitor::new(N(5), G, 1e-9, rate),
            Capacitor::new(N(1), N(2), 100e-12, rate),
            Capacitor::new(N(3), G, 2.2e-6, rate),
            Capacitor::new(N(4), G, 4.7e-6, rate),
        ]);
        // Backward-Euler dominant pole: dy/dt = 2π GBW (v+ - v- - y/A0).
        // 1 MHz, 0.3 V/us are nominal LM308 figures, not a fit at 9 V.
        amp.constraint_row = Some(2);
        let mut this = Self {
            amp,
            clip: Network::new([
                Capacitor::new(S(0), N(0), 4.7e-6, rate),
                Capacitor::new(N(2), G, 3.3e-9, rate),
                Capacitor::new(N(2), N(3), 22e-9, rate),
            ]),
            output: Network::new([Capacitor::new(S(0), N(0), 1e-6, rate)]),
            previous: 0.0,
            slew: 300_000.0 / rate,
        };
        this.tune_at_rate(controls, rate);
        this
    }
    fn tune_at_rate(&mut self, [drive, filter, volume]: [f64; 3], rate: f64) {
        self.amp.begin();
        self.amp.resistor(N(0), G, 1_000_000.0);
        self.amp.resistor(N(0), N(5), 1_000.0);
        self.amp.resistor(N(1), N(2), 100_000.0 * drive * drive);
        self.amp.resistor(N(1), N(3), 47.0);
        self.amp.resistor(N(1), N(4), 560.0);
        let k = std::f64::consts::TAU * 1_000_000.0 / rate;
        self.amp.matrix[2] = [0.0, k, 1.0 + k / 100_000.0, 0.0, 0.0, -k];
        self.amp.sources[2] = [0.0, 0.0];
        self.amp.finish();
        self.clip.begin();
        self.clip.resistor(N(0), N(1), 1_000.0);
        self.clip
            .resistor(N(1), N(2), 1_500.0 + 100_000.0 * filter * filter);
        self.clip.resistor(N(3), G, 1_000_000.0);
        self.clip.finish();
        self.output.begin();
        self.output
            .resistor(N(0), N(1), 100_000.0 * (1.0 - volume * volume));
        self.output.resistor(N(1), G, 100_000.0 * volume * volume);
        self.output.resistor(N(1), G, 1_000_000.0); // external load
        self.output.finish();
    }
    pub(super) fn tune(&mut self, controls: [f64; 3]) {
        self.tune_at_rate(controls, 300_000.0 / self.slew);
    }
    pub(super) fn tick(&mut self, x: f64) -> f64 {
        let mut amp = self.amp.solve([x, self.previous]);
        let y = amp[2]
            .clamp(self.previous - self.slew, self.previous + self.slew)
            .clamp(-3.0, 3.0);
        // Re-solve the passive feedback equations with the limited output.
        let correction = (y - amp[2]) / self.amp.inverse[2][2];
        for i in 0..6 {
            amp[i] += self.amp.inverse[i][2] * correction;
        }
        amp[2] = y;
        self.amp.commit(&amp, [x, self.previous]);
        self.previous = y;
        let mut nodes = self.clip.solve([y, 0.0]);
        // Exact scalar reduction of the loaded RC network at the diode node.
        let r = self.clip.inverse[1][1];
        let v = symmetric_feedback_voltage(nodes[1] / r, 1.0 / r);
        let current = (nodes[1] - v) / r;
        for i in 0..4 {
            nodes[i] -= self.clip.inverse[i][1] * current;
        }
        nodes[1] = v;
        self.clip.commit(&nodes, [y, 0.0]);
        // Ideal unity source follower; its 1M gate loading is included above.
        let output = self.output.solve([nodes[3], 0.0]);
        self.output.commit(&output, [nodes[3], 0.0]);
        output[1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn amplifier_obeys_slew_and_rails_and_recovers() {
        let mut cat = Cat::new(384000.0, [1.0, 0.5, 1.0]);
        let mut previous = 0.0;
        for i in 0..10000 {
            let y = cat.tick(if i < 5000 { 64.0 } else { 0.0 });
            assert!(y.is_finite());
            assert!(cat.previous.abs() <= 3.0);
            assert!((cat.previous - previous).abs() <= cat.slew + 1e-12);
            previous = cat.previous;
        }
        for _ in 0..384000 {
            cat.tick(0.0);
        }
        assert!(cat.previous.abs() < 1e-5);
    }
    #[test]
    fn clockwise_filter_cuts_treble_and_volume_zero_mutes() {
        let mut bright = Cat::new(384000.0, [0.5, 0.0, 0.7]);
        let mut dark = Cat::new(384000.0, [0.5, 1.0, 0.7]);
        let mut muted = Cat::new(384000.0, [0.5, 0.5, 0.0]);
        let mut energy = [0.0; 3];
        for i in 0..65536 {
            let x = 0.1 * (std::f64::consts::TAU * 6000.0 * i as f64 / 384000.0).sin();
            let y = [bright.tick(x), dark.tick(x), muted.tick(x)];
            if i > 32768 {
                for j in 0..3 {
                    energy[j] += y[j] * y[j];
                }
            }
        }
        assert!(energy[1] < energy[0] * 0.02, "{energy:?}");
        assert!(energy[2] < energy[0] * 1e-9, "{energy:?}");
    }
}
