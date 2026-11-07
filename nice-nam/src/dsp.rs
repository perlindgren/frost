//! Prepared off-thread; process never loads, allocates, locks, or destroys a chain.
use crate::settings::NamPreferences;
#[cfg(not(windows))]
use nam_engine::prelude::*;
#[cfg(windows)]
use nam_windows::{Model, NamModel};

pub const BLOCK: usize = 256;
/// NAM's gain convention, applied identically by both engines.
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
/// `output_level_dbu` is absent on purpose. The loader macOS and Linux use
/// parses it, validates it and reports it, and never applies it — `tests/dsp.rs`
/// asserts this derivation agrees with that loader's numbers on the same file.
///
/// The reference converts decibels through a table of 4096 gains spanning −96 dB
/// to +30 dB, interpolated linearly between steps of 0.031 dB. Exact `10^(dB/20)`
/// agrees with it to about two parts per million, which is inaudible in a gain
/// that never moves. The table's *range* is not cosmetic and is reproduced below.
///
/// macOS and Linux take these numbers from the loader instead of computing them,
/// so on those platforms nothing but the parity test calls this — it stays
/// compiled there so one arithmetic can be held against the other engine's.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn calibration(input_level_dbu: Option<f32>, loudness: Option<f32>) -> (f32, f32) {
    const FULL_SCALE_INPUT_DB: f32 = 12.0;
    const LOUDNESS_DB: f32 = -18.0;
    // Both engines stop here: the reference clamps at its table's ends, so a file
    // whose metadata passes the ±60 dB gate and still asks for +42 dB gets +30 dB
    // there. Unclamped, the same file would play 12 dB louder on Windows than on
    // the engine that stops, which is the opposite of what this function is for.
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
/// What a model file is allowed to be, before any of it is built.
///
/// The Windows engine checks the network — architecture, activation, channel
/// counts, weight count — and refuses what it cannot run, but it does that while
/// building, and it takes everything outside the network on trust: a `.nam` is
/// JSON anyone can hand-edit. The loader macOS and Linux use gates the bytes and
/// the numbers as well, so this does too, and a corrupt file fails the same way
/// on both platforms instead of silencing itself on one.
#[cfg(windows)]
const MAX_MODEL_BYTES: u64 = 256 * 1024 * 1024;
/// Past this, a metadata level is not a level. The same limit the other loader
/// applies, for the reason it gives: a value like `1e39` arrives as `+∞` when
/// deserialized into an `f32`, which would otherwise become an infinite gain.
#[cfg(windows)]
const MAX_PLAUSIBLE_DB: f32 = 60.0;
/// The magic of the binary `.namb` container.
#[cfg(windows)]
const NAMB: &[u8; 4] = b"NAMB";

/// Both engines cap what a model file may weigh. Split out so the boundary is
/// testable without writing 256 MiB into a temporary directory.
#[cfg(windows)]
pub(crate) fn size_is_plausible(bytes: u64) -> bool {
    bytes <= MAX_MODEL_BYTES
}

/// Whether this is a `.namb` rather than a `.nam`.
///
/// Detected by its magic and not by its extension, because a renamed container
/// should get this message rather than "failed to parse `.nam` JSON". The
/// limitation is this build's engine: the loader the other platforms use reads
/// both formats, and the same file loads there.
#[cfg(windows)]
pub(crate) fn is_namb_container(path: &std::path::Path) -> bool {
    use std::io::Read;
    std::fs::File::open(path)
        .and_then(|mut file| {
            let mut head = [0u8; NAMB.len()];
            file.read_exact(&mut head)?;
            Ok(head == *NAMB)
        })
        .unwrap_or(false)
}

/// The numbers around the network, which the engine does not examine and which
/// this build depends on: weights are copied into the buffers the audio thread
/// runs, and the calibration levels set the gain in front of them.
#[cfg(windows)]
pub(crate) fn numbers_are_plausible(file: &NamModel) -> Result<(), String> {
    for (field, value) in [
        ("input_level_dbu", file.input_level_dbu()),
        ("output_level_dbu", file.output_level_dbu()),
        ("loudness", file.loudness()),
    ] {
        let Some(value) = value else { continue };
        if !value.is_finite() || value.abs() > MAX_PLAUSIBLE_DB {
            return Err(format!(
                "{field} is {value}, outside the ±{MAX_PLAUSIBLE_DB} dB an audio device produces"
            ));
        }
    }
    if let Some(index) = file.weights.iter().position(|weight| !weight.is_finite()) {
        return Err(format!(
            "weight {} of {} is not a number",
            index,
            file.weights.len()
        ));
    }
    Ok(())
}
pub struct Chain {
    pub rate: u32,
    /// Applied to every input sample before the model, and after it: the two
    /// halves of [`calibration`], read once at load.
    input_gain: f32,
    output_gain: f32,
    #[cfg(not(windows))]
    model: Option<LoadedModelPair>,
    #[cfg(not(windows))]
    cab: Option<CabSimAdapter>,
    #[cfg(windows)]
    model: Option<Model>,
}
impl Chain {
    pub fn load(paths: &NamPreferences, rate: u32) -> Result<(Self, String), String> {
        #[cfg(windows)]
        {
            let mut description = String::from("No model — dry mono input");
            let mut input_gain = 1.0;
            let mut output_gain = 1.0;
            let model = if let Some(path) = &paths.model_path {
                // Nothing is built for a file this build cannot run, so the cheap
                // refusals come first and each one says what to do about it.
                let bytes = std::fs::metadata(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?
                    .len();
                if !size_is_plausible(bytes) {
                    return Err(format!(
                        "{}: {} bytes is not a model; {MAX_MODEL_BYTES} bytes is the limit.",
                        path.display(),
                        bytes
                    ));
                }
                if is_namb_container(path) {
                    return Err(format!(
                        "{}: a binary .namb container. This build's engine reads .nam exports \
                         only; export the model as .nam, or use the macOS or Linux build.",
                        path.display()
                    ));
                }
                let file =
                    NamModel::from_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
                numbers_are_plausible(&file).map_err(|e| format!("{}: {e}", path.display()))?;
                // The rate is settled before the network is built. A model's
                // dilations and recurrence are counted in samples, not seconds,
                // so running it at another rate is not a difference of tone but
                // a wrong sound, and the engine does not resample for us.
                let wanted = file.expected_sample_rate();
                if wanted != f64::from(rate) {
                    return Err(format!(
                        "Model requires {wanted} Hz; audio is {rate} Hz. Change the input sample rate before loading."
                    ));
                }
                let mut model =
                    Model::from_nam(&file).map_err(|e| format!("{}: {e}", path.display()))?;
                let architecture = match &model {
                    Model::WaveNet(_) => "WaveNet",
                    Model::Lstm(_) => "LSTM",
                    Model::Slimmable(_) => "Slimmable container",
                    // `Model` is non-exhaustive: an architecture the pinned dependency
                    // grows into still loads, and still says what it is.
                    _ => "NAM model",
                };
                description = format!(
                    "{architecture} | {rate} Hz | receptive field {} samples\n{}",
                    model.receptive_field(),
                    path.display()
                );
                if file.sample_rate.is_none() {
                    // A file from before the field existed. Both engines assume
                    // 48 kHz, which is probably right and might not be, so the
                    // status line says which it is instead of presenting a guess
                    // as something the file stated.
                    description.push_str("\nSample rate not recorded in the file: assumed 48 kHz.");
                }
                // A WaveNet fills its dilated history from zeros, so the first
                // receptive field of real audio is a transient. Settling against
                // silence here keeps it off the speaker: this runs on the
                // preparation thread, and the chain is not published until it returns.
                let mut settle = vec![0.0_f32; BLOCK];
                let mut remaining = model.receptive_field();
                while remaining > 0 {
                    let n = remaining.min(BLOCK);
                    model.process_buffer(&mut settle[..n]);
                    remaining -= n;
                }
                // Calibration is read off the parsed file, not the built `Model`:
                // the pinned 0.4.0 has no forwarding accessors on `Model`, and they
                // would only echo these same values back.
                let (input, output) = calibration(file.input_level_dbu(), file.loudness());
                input_gain = input;
                output_gain = output;
                let mut applied = Vec::new();
                if let Some(level) = file.input_level_dbu() {
                    applied.push(format!(
                        "input {level} dBu ({:+.1} dB)",
                        20.0 * input.log10()
                    ));
                }
                if let Some(loudness) = file.loudness() {
                    applied.push(format!(
                        "loudness {loudness} dBFS ({:+.1} dB)",
                        20.0 * output.log10()
                    ));
                }
                if !applied.is_empty() {
                    description
                        .push_str(&format!("\nCalibration applied: {}.", applied.join(", ")));
                }
                Some(model)
            } else {
                None
            };
            if paths.cabinet_path.is_some() {
                description.push_str("\nCabinet IR ignored: no cabinet engine in this build.");
            }
            Ok((
                Self {
                    rate,
                    model,
                    input_gain,
                    output_gain,
                },
                description,
            ))
        }
        #[cfg(not(windows))]
        {
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
                // The loader has already done NAM's gain staging, here and on
                // every other host that uses it. The chain keeps the two
                // multipliers so `process` never reaches through the pair, and
                // the same two numbers are what the Windows engine derives for
                // itself from the same file.
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
                    description
                        .push_str(&format!("\nCalibration applied: {}.", applied.join(", ")));
                }
                Some(pair)
            } else {
                None
            };
            let cab = if let Some(path) = &paths.cabinet_path {
                let ir = CabSimIr::load(path, rate, true)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
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
    }
    pub fn latency(&self) -> u32 {
        #[cfg(not(windows))]
        {
            self.cab
                .as_ref()
                .map_or(0, |cab| cab.latency_samples() as u32)
        }
        #[cfg(windows)]
        {
            0
        }
    }
    pub fn reset(&mut self) {
        #[cfg(not(windows))]
        {
            if let Some(pair) = &mut self.model
                && let Some(model) = &mut pair.model_l
            {
                model.clear_state();
            }
            if let Some(cab) = &mut self.cab {
                cab.reset();
            }
        }
        #[cfg(windows)]
        if let Some(model) = &mut self.model {
            model.reset();
        }
    }
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let (input_gain, output_gain) = (self.input_gain, self.output_gain);
        #[cfg(not(windows))]
        {
            if let Some(pair) = self.model.as_mut() {
                let mut scaled = [0.0; BLOCK];
                for (dest, sample) in scaled.iter_mut().zip(input) {
                    *dest = *sample * input_gain;
                }
                if let Some(model) = pair.model_l.as_mut() {
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
        }
        #[cfg(windows)]
        {
            // `process_buffer` works in place, so the input travels into the
            // caller's output buffer and is transformed there: no third buffer,
            // and nothing allocated on the way. The gain staging rides along on
            // the way in and on the way out, which is where the other engine
            // applies it too.
            if let Some(model) = self.model.as_mut() {
                for (dest, sample) in output.iter_mut().zip(input) {
                    *dest = *sample * input_gain;
                }
                model.process_buffer(output);
                for sample in output.iter_mut() {
                    *sample *= output_gain;
                }
            } else {
                output.copy_from_slice(input);
            }
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
        // low would ask for +42 dB of output. The loader macOS and Linux use
        // reads its gain from a table ending at +30 dB and clamps there, so this
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
