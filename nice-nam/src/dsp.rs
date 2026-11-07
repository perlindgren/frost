//! Prepared off-thread; process never loads, allocates, locks, or destroys a chain.
use crate::settings::NamPreferences;
#[cfg(not(windows))]
use nam_engine::prelude::*;

pub const BLOCK: usize = 256;
pub struct Chain {
    pub rate: u32,
    #[cfg(not(windows))]
    model: Option<LoadedModelPair>,
    #[cfg(not(windows))]
    cab: Option<CabSimAdapter>,
}
impl Chain {
    pub fn load(paths: &NamPreferences, rate: u32) -> Result<(Self, String), String> {
        #[cfg(windows)]
        {
            let _ = paths;
            Ok((Self { rate }, "Windows DSP is not implemented".into()))
        }
        #[cfg(not(windows))]
        {
            let mut description = String::from("No model — dry mono input");
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
            Ok((Self { rate, model, cab }, description))
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
    }
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        #[cfg(not(windows))]
        {
            if let Some(pair) = self.model.as_mut() {
                let mut scaled = [0.0; BLOCK];
                for (dest, sample) in scaled.iter_mut().zip(input) {
                    *dest = *sample * pair.input_mult_adj;
                }
                if let Some(model) = pair.model_l.as_mut() {
                    audio_float::with_denormals_flushed(|| {
                        model.process(&scaled[..input.len()], output)
                    });
                    for sample in output.iter_mut() {
                        *sample *= pair.output_mult_adj;
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
        output.copy_from_slice(input);
        for sample in output {
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
    }
}
