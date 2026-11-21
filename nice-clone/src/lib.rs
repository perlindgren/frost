//! Nice Clone: a component-based Centaur-style overdrive with a frost editor.
#![deny(unsafe_code)]

const BLOCK: usize = 128;
pub mod dsp;
mod editor;
mod gui;
#[cfg(feature = "standalone")]
pub mod settings;
#[cfg(feature = "standalone")]
pub mod standalone;
mod ui;

use crossbeam::atomic::AtomicCell;
use editor::CloneEditor;
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

pub struct CloneSharedState {
    pub input_rms: AtomicCell<f32>,
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    pub output_peak: AtomicCell<f32>,
    pub meter_updated: AtomicU64,
    pub gain: AtomicCell<f32>,
    pub treble: AtomicCell<f32>,
    pub output: AtomicCell<f32>,
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

impl CloneSharedState {
    fn logical_size(&self) -> LogicalSize<f32> {
        self.logical_size.load()
    }
    fn scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }

    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            gain: self.gain.load(),
            treble: self.treble.load(),
            output: self.output.load(),
        }
    }

    fn publish(&self, params: &CloneParams) {
        self.gain.store(params.gain.value());
        self.treble.store(params.treble.value());
        self.output.store(params.output.value());
        self.oversampling.store(params.oversampling.value());
    }
}

/// Normalized physical knob positions, shared by the circuit and editor.
#[derive(Params)]
pub struct CloneParams {
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "oversampling"]
    pub oversampling: IntParam,
}
impl Default for CloneParams {
    fn default() -> Self {
        Self {
            gain: FloatParam::new("Gain", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            treble: FloatParam::new("Treble", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            output: FloatParam::new("Output", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            oversampling: IntParam::new("Oversampling", 3, IntRange::Linear { min: 0, max: 3 })
                .with_value_to_string(Arc::new(|index| {
                    dsp::Oversampling::from_index(index).label().into()
                }))
                .with_string_to_value(Arc::new(|text| {
                    dsp::Oversampling::ALL
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

pub struct ClonePlugin {
    params: Arc<CloneParams>,
    shared: Arc<CloneSharedState>,
    drive: dsp::CloneDrive,
    reported_latency: u32,
    tp_in: [TruePeak; 2],
    tp_out: [TruePeak; 2],
}

#[cfg(feature = "standalone")]
pub(crate) fn seed_standalone_params(
    shared: &CloneSharedState,
    params: &CloneParams,
    prefs: &settings::ClonePreferences,
) {
    let seed = |cell: &AtomicCell<f32>, param: &FloatParam, saved| {
        if let Some(value) = saved {
            let t = param.range().normalize(value).clamp(0.0, 1.0);
            cell.store(param.range().unnormalize(t));
        }
    };
    seed(&shared.gain, &params.gain, prefs.gain);
    seed(&shared.treble, &params.treble, prefs.treble);
    seed(&shared.output, &params.output, prefs.output);
    if let Some(factor) = prefs.oversampling {
        shared
            .oversampling
            .store(dsp::Oversampling::from_factor(factor).index());
    }
}

impl Default for ClonePlugin {
    fn default() -> Self {
        let params = Arc::new(CloneParams::default());
        let shared = Arc::new(CloneSharedState {
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            meter_updated: AtomicU64::new(0),
            gain: AtomicCell::new(params.gain.value()),
            treble: AtomicCell::new(params.treble.value()),
            output: AtomicCell::new(params.output.value()),
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
                eprintln!("nice-clone: {warning}");
            }
            seed_standalone_params(&shared, &params, &settings.preferences);
        }
        let drive = dsp::CloneDrive::with_oversampling(
            44_100.0,
            &shared.controls(),
            dsp::Oversampling::from_index(shared.oversampling.load()),
        );
        Self {
            params,
            shared,
            reported_latency: drive.latency(),
            drive,
            tp_in: [TruePeak::new(); 2],
            tp_out: [TruePeak::new(); 2],
        }
    }
}

impl Plugin for ClonePlugin {
    const NAME: &'static str = "Nice Clone";
    const VENDOR: &'static str = "frost developers";
    const URL: &'static str = "https://github.com/perlindgren/frost";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Mono in, mono out, and nothing else: the box is mono — one signal
    /// through one clone circuit — and so is this model. The plugin neither
    /// folds a stereo signal down nor widens a mono one; that width work
    /// belongs to the host and the rack, which is exactly why this is the only
    /// layout here.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(1),
        main_output_channels: NonZeroU32::new(1),
        ..AudioIOLayout::const_default()
    }];

    type Editor = CloneEditor;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Self::Editor> {
        Some(CloneEditor {
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
        self.drive
            .request_oversampling(dsp::Oversampling::from_index(
                self.shared.oversampling.load(),
            ));
        self.drive.set_rate(config.sample_rate);
        self.drive.set_controls(&self.shared.controls());
        self.drive.reset();
        self.reported_latency = self.drive.latency();
        context.set_latency_samples(self.reported_latency);
        true
    }

    fn reset(&mut self) {
        self.drive.reset();
        for meter in self.tp_in.iter_mut().chain(self.tp_out.iter_mut()) {
            meter.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        if !STANDALONE_MODE.load(Ordering::Relaxed) {
            self.shared.publish(&self.params);
        }
        self.drive.set_controls(&self.shared.controls());
        self.drive
            .request_oversampling(dsp::Oversampling::from_index(
                self.shared.oversampling.load(),
            ));
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
                // The first channel is the input: the layout guarantees one,
                // and the plugin never folds channels together; width is the
                // host's work, not the pedal's.
                output[0][frame] = self.drive.process(input[0][frame]);
            }
            let (energy, peak) = block_stats(&output, channels, n, &mut self.tp_out);
            output_energy += energy;
            output_peak = output_peak.max(peak);
            count += n;
            for (channel, out) in block.into_iter().enumerate().take(channels) {
                out.copy_from_slice(&output[channel][..n]);
            }
        }
        if self.drive.latency() != self.reported_latency {
            self.reported_latency = self.drive.latency();
            context.set_latency_samples(self.reported_latency);
        }
        if count > 0 {
            self.shared
                .input_rms
                .store((input_energy / count as f64).sqrt() as f32);
            self.shared
                .output_rms
                .store((output_energy / count as f64).sqrt() as f32);
            self.shared.input_peak.store(input_peak);
            self.shared.output_peak.store(output_peak);
            self.shared.meter_updated.store(now_ms(), Ordering::Relaxed);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for ClonePlugin {
    const CLAP_ID: &'static str = "com.frost.nice-clone";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Component-based Centaur-style germanium overdrive");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Distortion];
}

impl Vst3Plugin for ClonePlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"FrostNiceCloneFX";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}

nice_export_clap!(ClonePlugin);
nice_export_vst3!(ClonePlugin);

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
    fn oversampling_is_a_saved_non_automatable_host_parameter_with_eightfold_default() {
        let params = CloneParams::default();
        assert_eq!(params.oversampling.value(), dsp::Oversampling::X8.index());
        assert!(
            params
                .oversampling
                .flags()
                .contains(ParamFlags::NON_AUTOMATABLE)
        );
        let mut ids: Vec<_> = params
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort();
        assert_eq!(ids, ["gain", "output", "oversampling", "treble"]);
    }
}
