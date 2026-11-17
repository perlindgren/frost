//! Nice Clone plugin scaffold. Controls are stored and automated; audio is unchanged.
#![deny(unsafe_code)]

const BLOCK: usize = 128;
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

    fn publish(&self, params: &CloneParams) {
        self.gain.store(params.gain.value());
        self.treble.store(params.treble.value());
        self.output.store(params.output.value());
    }
}

/// Normalized knob positions. Their DSP mapping is intentionally deferred.
#[derive(Params)]
pub struct CloneParams {
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}
impl Default for CloneParams {
    fn default() -> Self {
        Self {
            gain: FloatParam::new("Gain", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            treble: FloatParam::new("Treble", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            output: FloatParam::new("Output", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
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
        Self {
            params,
            shared,
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
        _config: &BufferConfig,
        _: &mut impl ActivateContext<Self>,
    ) -> bool {
        true
    }

    fn reset(&mut self) {
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
            // Scaffold only: copy samples unchanged, regardless of controls.
            let output = input;
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

impl ClapPlugin for ClonePlugin {
    const CLAP_ID: &'static str = "com.frost.nice-clone";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Overdrive scaffold; audio passes through unchanged");
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
