//! Nice Dist: TS808/TS9/OD-1/SD-1/RAT/Centaur overdrive with oversampled nonlinear feedback clipping.
#![deny(unsafe_code)]

const BLOCK: usize = 128;
pub mod controls;
pub mod dsp;
use nice_common::oversample::Oversampling;
mod editor;
mod gui;
#[cfg(feature = "standalone")]
pub mod settings;
#[cfg(feature = "standalone")]
pub mod standalone;
mod ui;

use crossbeam::atomic::AtomicCell;
use editor::DistEditor;
use nice_common::true_peak::TruePeak;
use nice_plug::editor::dpi::LogicalSize;
use std::num::NonZeroU32;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

#[cfg(feature = "standalone")]
pub use nice_common::devices;
#[cfg(feature = "standalone")]
use std::sync::Mutex;

/// Standalone owns the shared control cells; a plugin host owns the parameters.
pub static STANDALONE_MODE: AtomicBool = AtomicBool::new(false);

const MIN_SIZE: LogicalSize<f32> = LogicalSize::new(560.0, 380.0);
const DEFAULT_SIZE: LogicalSize<f32> = LogicalSize::new(860.0, 440.0);
const RESIZE_HINT: ResizeHint = ResizeHint::resizable().with_min_logical_size(MIN_SIZE);

use nice_plug::prelude::*;

pub struct DistSharedState {
    pub input_rms: AtomicCell<f32>,
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    pub output_peak: AtomicCell<f32>,
    pub meter_updated: AtomicU64,
    pub drive: AtomicCell<f32>,
    pub tone: AtomicCell<f32>,
    pub output: AtomicCell<f32>,
    pub variant: AtomicCell<i32>,
    pub oversampling: AtomicCell<i32>,
    pub buffer_frames: AtomicU32,
    pub dropped: AtomicU64,
    logical_size: AtomicCell<LogicalSize<f32>>,
    scale_factor: AtomicCell<f64>,
    open: AtomicBool,
    #[cfg(feature = "standalone")]
    window_pos: Mutex<Option<(i32, i32)>>,
    #[cfg(feature = "standalone")]
    #[cfg_attr(not(all(windows, feature = "unsafe")), allow(dead_code))]
    pending_geom: Mutex<Option<(i32, i32)>>,
}

impl DistSharedState {
    fn logical_size(&self) -> LogicalSize<f32> {
        self.logical_size.load()
    }
    fn scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }

    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            drive: self.drive.load(),
            tone: self.tone.load(),
            level: self.output.load(),
        }
    }
    fn publish(&self, params: &DistParams) {
        self.drive.store(params.drive.value());
        self.tone.store(params.tone.value());
        self.output.store(params.output.value());
        self.variant.store(params.variant.value());
        self.oversampling.store(params.oversampling.value());
    }
}

/// Saved knob positions reserved for the future DSP implementation.
#[derive(Params)]
pub struct DistParams {
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "level"]
    pub output: FloatParam,
    #[id = "variant"]
    pub variant: IntParam,
    #[id = "oversampling"]
    pub oversampling: IntParam,
}
impl Default for DistParams {
    fn default() -> Self {
        Self {
            drive: FloatParam::new("Drive", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            tone: FloatParam::new("Tone", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            output: FloatParam::new("Level", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            oversampling: IntParam::new("Oversampling", 3, IntRange::Linear { min: 0, max: 3 })
                .with_value_to_string(Arc::new(|i| Oversampling::from_index(i).label().into()))
                .non_automatable(),
            variant: IntParam::new("Model", 0, IntRange::Linear { min: 0, max: 5 })
                .with_value_to_string(Arc::new(|index| {
                    controls::Variant::from_index(index).label().into()
                }))
                .with_string_to_value(Arc::new(|text| {
                    controls::Variant::ALL
                        .iter()
                        .find(|mode| mode.label().eq_ignore_ascii_case(text.trim()))
                        .map(|mode| mode.index())
                }))
                .non_automatable(),
        }
    }
}

fn block_stats(
    scratch: &[[f32; BLOCK]; 2],
    channels: usize,
    n: usize,
    tp: &mut [TruePeak; 2],
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

pub struct DistPlugin {
    params: Arc<DistParams>,
    shared: Arc<DistSharedState>,
    effect: dsp::Dist,
    tp_in: [TruePeak; 2],
    tp_out: [TruePeak; 2],
}

#[cfg(feature = "standalone")]
pub(crate) fn seed_standalone_params(
    shared: &DistSharedState,
    params: &DistParams,
    prefs: &settings::DistPreferences,
) {
    let seed = |cell: &AtomicCell<f32>, param: &FloatParam, saved| {
        if let Some(value) = saved {
            let t = param.range().normalize(value).clamp(0.0, 1.0);
            cell.store(param.range().unnormalize(t));
        }
    };
    seed(&shared.drive, &params.drive, prefs.drive);
    seed(&shared.tone, &params.tone, prefs.tone);
    seed(&shared.output, &params.output, prefs.output);
    if let Some(factor) = prefs.oversampling {
        shared
            .oversampling
            .store(Oversampling::from_factor(factor).index());
    }
    if let Some(factor) = prefs.variant {
        shared
            .variant
            .store(controls::Variant::from_number(factor).index());
    }
}

impl Default for DistPlugin {
    fn default() -> Self {
        let params = Arc::new(DistParams::default());
        let shared = Arc::new(DistSharedState {
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            meter_updated: AtomicU64::new(0),
            drive: AtomicCell::new(params.drive.value()),
            tone: AtomicCell::new(params.tone.value()),
            output: AtomicCell::new(params.output.value()),
            variant: AtomicCell::new(params.variant.value()),
            oversampling: AtomicCell::new(params.oversampling.value()),
            buffer_frames: AtomicU32::new(0),
            dropped: AtomicU64::new(0),
            logical_size: AtomicCell::new(DEFAULT_SIZE),
            scale_factor: AtomicCell::new(1.0),
            open: AtomicBool::new(false),
            #[cfg(feature = "standalone")]
            window_pos: Mutex::new(None),
            #[cfg(feature = "standalone")]
            pending_geom: Mutex::new(None),
        });
        #[cfg(feature = "standalone")]
        if STANDALONE_MODE.load(Ordering::Relaxed) {
            let (settings, warning) = settings::Settings::load();
            if let Some(warning) = warning {
                eprintln!("nice-dist: {warning}");
            }
            seed_standalone_params(&shared, &params, &settings.preferences);
        }
        let effect = dsp::Dist::new(
            44_100.0,
            &shared.controls(),
            controls::Variant::from_index(shared.variant.load()),
            Oversampling::from_index(shared.oversampling.load()),
        );
        Self {
            params,
            shared,
            effect,
            tp_in: [TruePeak::new(); 2],
            tp_out: [TruePeak::new(); 2],
        }
    }
}

impl Plugin for DistPlugin {
    const NAME: &'static str = "Nice Dist";
    const VENDOR: &'static str = "frost developers";
    const URL: &'static str = "https://github.com/perlindgren/frost";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    type Editor = DistEditor;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Self::Editor> {
        Some(DistEditor {
            params: self.params.clone(),
            shared: self.shared.clone(),
        })
    }

    fn activate(
        &mut self,
        _: &AudioIOLayout,
        config: &BufferConfig,
        context: &mut impl ActivateContext<Self>,
    ) -> bool {
        if !config.sample_rate.is_finite() || config.sample_rate < 8_000.0 {
            return false;
        }
        if !STANDALONE_MODE.load(Ordering::Relaxed) {
            self.shared.publish(&self.params);
        }
        self.effect = dsp::Dist::new(
            config.sample_rate,
            &self.shared.controls(),
            controls::Variant::from_index(self.shared.variant.load()),
            Oversampling::from_index(self.shared.oversampling.load()),
        );
        context.set_latency_samples(self.effect.latency());
        true
    }

    fn reset(&mut self) {
        self.effect.set_controls(
            &self.shared.controls(),
            controls::Variant::from_index(self.shared.variant.load()),
            Oversampling::from_index(self.shared.oversampling.load()),
        );
        self.effect.reset();
        for meter in self.tp_in.iter_mut().chain(self.tp_out.iter_mut()) {
            meter.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        if !STANDALONE_MODE.load(Ordering::Relaxed) {
            self.shared.publish(&self.params);
        }
        self.effect.set_controls(
            &self.shared.controls(),
            controls::Variant::from_index(self.shared.variant.load()),
            Oversampling::from_index(self.shared.oversampling.load()),
        );
        let channels = buffer.channels();
        let mut input = [[0.0f32; BLOCK]; 2];
        let mut input_energy = 0.0f64;
        let mut output_energy = 0.0f64;
        let mut input_peak = 0.0f32;
        let mut output_peak = 0.0f32;
        let mut count = 0usize;
        for (_, block) in buffer.iter_blocks(BLOCK) {
            let n = block.samples();
            for (channel, scratch) in input.iter_mut().enumerate().take(channels) {
                let samples = block.get(channel).unwrap();
                scratch[..n].copy_from_slice(&samples[..n]);
            }
            let (energy, peak) = block_stats(&input, channels, n, &mut self.tp_in);
            input_energy += energy;
            input_peak = input_peak.max(peak);
            let mut output = [[0.0f32; BLOCK]; 2];
            for frame in 0..n {
                let y = self.effect.process([
                    input[0][frame],
                    if channels == 2 { input[1][frame] } else { 0.0 },
                ]);
                output[0][frame] = y[0];
                output[1][frame] = y[1];
            }
            let (energy, peak) = block_stats(&output, channels, n, &mut self.tp_out);
            output_energy += energy;
            output_peak = output_peak.max(peak);
            count += n;
            for (channel, out) in block.into_iter().enumerate().take(channels) {
                out.copy_from_slice(&output[channel][..n]);
            }
        }
        if count > 0 {
            self.shared
                .input_rms
                .store((input_energy / (count * channels) as f64).sqrt() as f32);
            self.shared
                .output_rms
                .store((output_energy / (count * channels) as f64).sqrt() as f32);
            self.shared.input_peak.store(input_peak);
            self.shared.output_peak.store(output_peak);
            self.shared.meter_updated.store(now_ms(), Ordering::Relaxed);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for DistPlugin {
    const CLAP_ID: &'static str = "com.frost.nice-dist";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("TS808/TS9/OD-1/SD-1/RAT/Centaur component-based overdrive");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Distortion];
}

impl Vst3Plugin for DistPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"NiceDistPlugin01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}

nice_export_clap!(DistPlugin);
nice_export_vst3!(DistPlugin);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod parameter_tests {
    use super::*;
    #[test]
    fn variants_and_controls_have_distinct_saved_host_ids() {
        let params = DistParams::default();
        assert_eq!(params.variant.value(), controls::Variant::Ts808.index());
        let mut ids: Vec<_> = params
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort();
        assert_eq!(ids, ["drive", "level", "oversampling", "tone", "variant"]);
    }
}
