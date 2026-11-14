//! Nice NAM: shared hosted/standalone processing with an embedded frost editor.
#![deny(unsafe_code)]

mod dsp;
mod editor;
mod engine;
mod gui;
pub mod settings;
#[cfg(feature = "standalone")]
pub mod standalone;
mod ui;
use crossbeam::atomic::AtomicCell;
use editor::NamEditor;
#[cfg(feature = "standalone")]
use editor::NamEditorHandle;
use nice_plug::editor::dpi::LogicalSize;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
const MIN_SIZE: LogicalSize<f32> = LogicalSize::new(640.0, 620.0);
const DEFAULT_SIZE: LogicalSize<f32> = LogicalSize::new(900.0, 620.0);
const RESIZE_HINT: ResizeHint = ResizeHint::resizable().with_min_logical_size(MIN_SIZE);
pub static STANDALONE_MODE: AtomicBool = AtomicBool::new(false);
/// Gaps the audio device reported since the streams were opened, counted in the
/// standalone's stream error callback and read by the main view.
///
/// A static, like `STANDALONE_MODE`, because a stream error callback is built
/// per stream and is handed no reference to the plugin's shared state. It counts
/// what the backend reports and nothing more: a cpal backend on the platforms
/// that still have one reports a gap — WASAPI flags `DATA_DISCONTINUITY`, an
/// actual hole in the playback — and the ASIO transport reports one when it
/// cannot keep up. A backend that never reports keeps this at zero, so zero
/// means the device said nothing, which is why the label states the count
/// instead of declaring the audio healthy.
pub static DROP_OUTS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "standalone")]
pub use nice_common::devices;

use nice_plug::prelude::*;
use std::num::NonZeroU32;
use std::sync::Arc;

pub struct NamSharedState {
    pub input_rms: AtomicCell<f32>,
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    pub output_peak: AtomicCell<f32>,
    pub gate_db: AtomicCell<f32>,
    pub meter_updated: AtomicU64,
    logical_size: AtomicCell<LogicalSize<f32>>,
    scale_factor: AtomicCell<f64>,
    open: AtomicBool,
    #[cfg(feature = "standalone")]
    window_pos: Mutex<Option<(i32, i32)>>,
    /// A saved top-left, queued by the standalone host and applied once by the
    /// window handler through the platform API — baseview has no position
    /// concept. Only placement reads it, so it must not warn without placement.
    #[cfg(feature = "standalone")]
    #[cfg_attr(not(all(windows, feature = "unsafe")), allow(dead_code))]
    pending_geom: Mutex<Option<(i32, i32)>>,
    pub input_channel: AtomicCell<u32>,
    pub buffer_frames: AtomicU32,
    engine: Arc<engine::EngineState>,
}
impl NamSharedState {
    fn logical_size(&self) -> LogicalSize<f32> {
        self.logical_size.load()
    }
    fn scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }
}

pub struct NamPlugin {
    params: Arc<NamParams>,
    shared: Arc<NamSharedState>,
    incoming: rtrb::Consumer<Box<dsp::Chain>>,
    retiring: rtrb::Producer<Box<dsp::Chain>>,
    chain: Option<Box<dsp::Chain>>,
    gate_gain: f32,
}
#[derive(Default, Params)]
struct NamParams {
    #[persist = "assets"]
    paths: Arc<Mutex<settings::NamPreferences>>,
}
impl Default for NamPlugin {
    fn default() -> Self {
        let params = Arc::new(NamParams::default());
        if STANDALONE_MODE.load(Ordering::Relaxed) {
            *params.paths.lock().unwrap() = settings::Settings::load().0.preferences;
        }
        let (engine, incoming, retiring) = engine::start(Arc::clone(&params.paths));
        let gate_db = params.paths.lock().unwrap().gate_db.unwrap_or(-60.0);
        let shared = Arc::new(NamSharedState {
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            gate_db: AtomicCell::new(gate_db),
            meter_updated: AtomicU64::new(0),
            logical_size: AtomicCell::new(DEFAULT_SIZE),
            scale_factor: AtomicCell::new(1.0),
            open: AtomicBool::new(false),
            #[cfg(feature = "standalone")]
            window_pos: Mutex::new(None),
            #[cfg(feature = "standalone")]
            pending_geom: Mutex::new(None),
            input_channel: AtomicCell::new(1),
            buffer_frames: AtomicU32::new(0),
            engine,
        });
        Self {
            params,
            shared,
            incoming,
            retiring,
            chain: None,
            gate_gain: 1.0,
        }
    }
}

impl Plugin for NamPlugin {
    const NAME: &'static str = "Nice NAM";
    const VENDOR: &'static str = "frost developers";
    const URL: &'static str = "https://github.com/perlindgren/frost";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(1),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    type Editor = NamEditor;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Self::Editor> {
        Some(NamEditor {
            params: self.params.clone(),
            shared: self.shared.clone(),
        })
    }
    fn activate(
        &mut self,
        _: &AudioIOLayout,
        config: &BufferConfig,
        _: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.shared
            .gate_db
            .store(self.params.paths.lock().unwrap().gate_db.unwrap_or(-60.0));
        self.shared
            .engine
            .rate
            .store(config.sample_rate as u32, Ordering::Release);
        true
    }
    fn reset(&mut self) {
        if let Some(chain) = &mut self.chain {
            chain.reset();
        }
    }
    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Do not pop unless the old instance can be retired off the audio thread.
        if self.retiring.slots() > 0
            && let Ok(chain) = self.incoming.pop()
        {
            if let Some(old) = self.chain.replace(chain) {
                let _ = self.retiring.push(old);
            }
            context.set_latency_samples(self.chain.as_ref().map_or(0, |chain| chain.latency()));
        }
        let rate = self.shared.engine.rate.load(Ordering::Relaxed);
        let mut input_energy = 0.0f64;
        let mut output_energy = 0.0f64;
        let mut input_peak = 0.0f32;
        let mut output_peak = 0.0f32;
        let mut count = 0usize;
        let threshold = 10.0f32.powf(self.shared.gate_db.load() / 20.0);
        for (_, block) in buffer.iter_blocks(dsp::BLOCK) {
            let n = block.samples();
            let mut input = [0.0; dsp::BLOCK];
            let mut output = [0.0; dsp::BLOCK];
            input[..n].copy_from_slice(&block.get(0).unwrap()[..n]);
            let energy = input[..n]
                .iter()
                .map(|&x| f64::from(x).powi(2))
                .sum::<f64>();
            input_energy += energy;
            input_peak = input[..n]
                .iter()
                .fold(input_peak, |peak, x| peak.max(x.abs()));
            let target = if n > 0 && (energy / n as f64).sqrt() >= f64::from(threshold) {
                1.0
            } else {
                0.0
            };
            let seconds = if target > self.gate_gain { 0.005 } else { 0.05 };
            let coefficient = 1.0 - (-1.0 / (rate.max(1) as f32 * seconds)).exp();
            for sample in &mut input[..n] {
                self.gate_gain += (target - self.gate_gain) * coefficient;
                *sample *= self.gate_gain;
            }
            if let Some(chain) = self.chain.as_mut().filter(|chain| chain.rate == rate) {
                chain.process(&input[..n], &mut output[..n]);
            } else {
                output[..n].copy_from_slice(&input[..n]);
            }
            output_energy += output[..n]
                .iter()
                .map(|&x| f64::from(x).powi(2))
                .sum::<f64>();
            output_peak = output[..n]
                .iter()
                .fold(output_peak, |peak, x| peak.max(x.abs()));
            count += n;
            for channel in block.into_iter() {
                channel.copy_from_slice(&output[..n]);
            }
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

impl ClapPlugin for NamPlugin {
    const CLAP_ID: &'static str = "com.frost.nice-nam";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Neural Amp Modeler and optional cabinet convolution");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect];
}

impl Vst3Plugin for NamPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"FrostNiceNamFx01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
}

nice_export_clap!(NamPlugin);
nice_export_vst3!(NamPlugin);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
