//! Prepared off-thread; process never loads, allocates, locks, or destroys a chain.
use crate::settings::NamPreferences;
// One engine on every platform; only the name it is reached through differs,
// because cargo will not resolve one dependency name to two sources — see the
// manifest. Paths are bare for the same reason the test files can `#[path]`
// this module into their own crate.
#[cfg(not(windows))]
use nam_engine::prelude::*;
#[cfg(windows)]
use nam_windows::prelude::*;

pub const BLOCK: usize = 256;
/// NAM's gain convention, the one the engine's loader applies.
///
/// A model's calibration numbers are analog measurements, not digital ones:
/// `input_level_dbu` is what digital full scale meant at the amp input while
/// the capture ran, and `loudness` is how loud the model is against NAM's
/// standardized test input. Pulling the input to a 12 dBu full-scale convention
/// makes two captures of the same amp driven differently land in the same place,
/// and pulling the output to −18 dBFS is what keeps level from jumping when the
/// user switches models. Neither is a level follower: both are constants fixed
/// at load, so playing harder still drives the model harder.
///
/// `output_level_dbu` is absent on purpose. The loader parses it, validates it
/// and reports it, and never applies it — `tests/dsp.rs` asserts this derivation
/// agrees with the loader's numbers on the same file.
///
/// The reference converts decibels through a table of 4096 gains spanning −96 dB
/// to +30 dB, interpolated linearly between steps of 0.031 dB. Exact `10^(dB/20)`
/// agrees with it to about two parts per million, which is inaudible in a gain
/// that never moves. The table's *range* is not cosmetic and is reproduced below.
///
/// Nothing at runtime calls this any more: the chain takes the loader's own
/// multipliers, which is the same arithmetic done by the code that reads the
/// file. It stays compiled so the parity test can hold one against the other on
/// the same file, on every platform, instead of trusting that they agree.
#[allow(dead_code)]
pub(crate) fn calibration(input_level_dbu: Option<f32>, loudness: Option<f32>) -> (f32, f32) {
    const FULL_SCALE_INPUT_DB: f32 = 12.0;
    const LOUDNESS_DB: f32 = -18.0;
    // The loader stops here: it reads its gain from a table of the reference's
    // ending at these ends, so a file whose metadata passes the ±60 dB gate and
    // still asks for +42 dB gets +30 dB. Unclamped, this derivation would disagree
    // with the gain actually applied, which is the opposite of what it is for.
    const MIN_ADJUSTMENT_DB: f32 = -96.0;
    const MAX_ADJUSTMENT_DB: f32 = 30.0;
    let input_db = (FULL_SCALE_INPUT_DB - input_level_dbu.unwrap_or(FULL_SCALE_INPUT_DB))
        .clamp(MIN_ADJUSTMENT_DB, MAX_ADJUSTMENT_DB);
    let output_db =
        (LOUDNESS_DB - loudness.unwrap_or(LOUDNESS_DB)).clamp(MIN_ADJUSTMENT_DB, MAX_ADJUSTMENT_DB);
    (
        10.0f32.powf(input_db / 20.0),
        10.0f32.powf(output_db / 20.0),
    )
}
pub struct Chain {
    pub rate: u32,
    /// Applied to every input sample before the model, and after it: the two
    /// adjustment factors the loader derives from the model's calibration, read
    /// once at load.
    input_gain: f32,
    output_gain: f32,
    model: Option<LoadedModelPair>,
    cab: Option<CabSimAdapter>,
}
impl Chain {
    pub fn load(paths: &NamPreferences, rate: u32) -> Result<(Self, String), String> {
        let mut description = String::from("No model — dry mono input");
        let mut input_gain = 1.0;
        let mut output_gain = 1.0;
        let model = if let Some(path) = &paths.model_path {
            let mut pair = load_and_build_model(
                path,
                &SystemSnapshot::capture(),
                false,
                LoadOptions::default(),
            )
            .map_err(|e| format!("{}: {e}", path.display()))?;
            let info = pair.model_info(path);
            description = format!(
                "{} | {} | {} Hz | receptive field {} samples\n{}",
                pair.architecture,
                pair.topology,
                pair.sample_rate,
                info.receptive_field,
                path.display()
            );
            if pair.sample_rate != rate {
                return Err(format!(
                    "Model requires {} Hz; audio is {rate} Hz. Change the input sample rate before loading.\n{description}",
                    pair.sample_rate
                ));
            }
            let model = pair
                .model_l
                .as_mut()
                .ok_or("No mono model returned by loader")?;
            if model.num_input_channels() != 1 || model.num_output_channels() != 1 {
                return Err("Nice NAM requires a mono-input, mono-output model".into());
            }
            model
                .set_max_buffer_size(BLOCK)
                .map_err(|e| e.to_string())?;
            model.reset(rate, BLOCK).map_err(|e| e.to_string())?;
            // The loader has already done NAM's gain staging. The chain keeps
            // the two multipliers so `process` never reaches through the pair,
            // which is also what makes them testable: they are the numbers a
            // user hears as a level change when they switch models.
            input_gain = pair.input_mult_adj;
            output_gain = pair.output_mult_adj;
            let meta = pair.metadata.as_ref();
            let mut applied = Vec::new();
            if let Some(level) = meta.and_then(|m| m.input_level_dbu) {
                applied.push(format!(
                    "input {level} dBu ({:+.1} dB)",
                    20.0 * input_gain.log10()
                ));
            }
            if let Some(loudness) = meta.and_then(|m| m.loudness) {
                applied.push(format!(
                    "loudness {loudness} dBFS ({:+.1} dB)",
                    20.0 * output_gain.log10()
                ));
            }
            if !applied.is_empty() {
                description.push_str(&format!("\nCalibration applied: {}.", applied.join(", ")));
            }
            Some(pair)
        } else {
            None
        };
        let cab = if let Some(path) = &paths.cabinet_path {
            let ir =
                CabSimIr::load(path, rate, true).map_err(|e| format!("{}: {e}", path.display()))?;
            let engine =
                ConvEngine::new(&ir.samples, BLOCK).map_err(|e| format!("Cabinet: {e:?}"))?;
            Some(CabSimAdapter::new(Box::new(engine)).map_err(|e| format!("Cabinet: {e:?}"))?)
        } else {
            None
        };
        Ok((
            Self {
                rate,
                model,
                cab,
                input_gain,
                output_gain,
            },
            description,
        ))
    }
    pub fn latency(&self) -> u32 {
        self.cab
            .as_ref()
            .map_or(0, |cab| cab.latency_samples() as u32)
    }
    pub fn reset(&mut self) {
        if let Some(pair) = &mut self.model
            && let Some(model) = &mut pair.model_l
        {
            model.clear_state();
        }
        if let Some(cab) = &mut self.cab {
            cab.reset();
        }
    }
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let (input_gain, output_gain) = (self.input_gain, self.output_gain);
        if let Some(pair) = self.model.as_mut() {
            let mut scaled = [0.0; BLOCK];
            for (dest, sample) in scaled.iter_mut().zip(input) {
                *dest = *sample * input_gain;
            }
            if let Some(model) = pair.model_l.as_mut() {
                // The engine's kernels are the place denormals cost real time, so
                // the flush-to-zero window closes around the call and nothing else.
                audio_float::with_denormals_flushed(|| {
                    model.process(&scaled[..input.len()], output)
                });
                for sample in output.iter_mut() {
                    *sample *= output_gain;
                }
            } else {
                output.copy_from_slice(input);
            }
        } else {
            output.copy_from_slice(input);
        }
        if let Some(cab) = self.cab.as_mut() {
            cab.process_in_place(output, None);
        }
        for sample in output {
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::calibration;
    #[test]
    fn a_file_without_calibration_changes_nothing() {
        // The defaults are the convention itself, so a bare file computes 0 dB
        // both ways instead of something plausible and wrong.
        assert_eq!(calibration(None, None), (1.0, 1.0));
    }
    #[test]
    fn each_number_is_the_distance_from_the_convention() {
        // A capture at 18.3 dBu full scale is 6.3 dB hotter than the convention,
        // so the input comes down; a model measuring -20.5 dBFS is 2.5 dB quieter
        // than -18, so the output goes up. The signs are the half that gets
        // inverted by mistake, so they are asserted as well as the values.
        let (input, output) = calibration(Some(18.3), Some(-20.5));
        assert!((input - 10f32.powf(-6.3 / 20.0)).abs() < 1e-6, "{input}");
        assert!((output - 10f32.powf(2.5 / 20.0)).abs() < 1e-6, "{output}");
        assert!(input < 1.0 && output > 1.0, "{input} {output}");
    }
    #[test]
    fn an_adjustment_beyond_the_reference_table_stops_where_that_table_stops() {
        // ±60 dB of metadata is allowed through the gate, and a loudness that
        // low would ask for +42 dB of output. The loader reads its gain from a
        // table ending at +30 dB and clamps there, so this
        // must land on the same multiplier as a file asking for exactly +30 dB
        // rather than run 12 dB ahead of it.
        let extreme = calibration(Some(-60.0), Some(-60.0));
        let at_the_end = calibration(Some(-60.0), Some(-48.0));
        assert_eq!(extreme.1, at_the_end.1, "output must stop at +30 dB");
        assert!(
            (extreme.1 - 10f32.powf(30.0 / 20.0)).abs() < 1e-3,
            "the cap should be +30 dB, not {}",
            extreme.1
        );
        // The input side reaches the same ceiling from the other direction: 12
        // dBu minus a −60 dBu file is +72 dB unclamped.
        assert_eq!(extreme.0, at_the_end.0);
    }
}
