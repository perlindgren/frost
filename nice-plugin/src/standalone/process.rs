//! No-MIDI standalone context and the mono/stereo transport-to-plugin adapter.
use nice_common::audio_io::Endpoints;
use nice_common::resample::StereoFrame;
use nice_plug::{context::process::SendEventError, prelude::*};
use std::{
    cell::Cell,
    marker::PhantomData,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub struct Context<P> {
    transport: Transport,
    latency: Cell<u32>,
    latency_reports: Cell<usize>,
    marker: PhantomData<fn() -> P>,
}
impl<P> Context<P> {
    pub fn new(rate: f32) -> Self {
        Self {
            transport: Transport::new(rate),
            latency: Cell::new(0),
            latency_reports: Cell::new(0),
            marker: PhantomData,
        }
    }
    pub fn latency_samples(&self) -> u32 {
        self.latency.get()
    }
    pub fn latency_reports(&self) -> usize {
        self.latency_reports.get()
    }
    fn report_latency(&self, samples: u32) {
        self.latency.set(samples);
        self.latency_reports.set(self.latency_reports.get() + 1);
    }
}
impl<P: Plugin<BackgroundTask = ()>> ActivateContext<P> for Context<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn execute(&self, _: ()) {}
    fn set_latency_samples(&self, value: u32) {
        self.report_latency(value);
    }
    fn set_current_voice_capacity(&self, _: u32) {}
}
impl<P: Plugin<BackgroundTask = ()>> ProcessContext<P> for Context<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn execute_background(&self, _: ()) {}
    fn execute_gui(&self, _: ()) {}
    fn transport(&self) -> &Transport {
        &self.transport
    }
    fn next_event(&mut self) -> Option<PluginNoteEvent<P>> {
        None
    }
    fn try_send_event(
        &mut self,
        event: PluginNoteEvent<P>,
    ) -> Result<(), (PluginNoteEvent<P>, SendEventError)> {
        Err((event, SendEventError::NoOutputBuffer))
    }
    fn set_latency_samples(&self, value: u32) {
        self.report_latency(value);
    }
    fn request_restart(&self) {}
    fn set_current_voice_capacity(&self, _: u32) {}
}

pub fn process_once<P: Plugin<BackgroundTask = ()>, const CAPACITY: usize>(
    buffer: &mut plugin_buffer::PlanarBuffer<CAPACITY>,
    context: &mut Context<P>,
    plugin: &Arc<Mutex<P>>,
    io: &mut Endpoints,
    max_frames: usize,
) -> usize {
    let count = io.input.slots().min(max_frames);
    if count == 0 {
        return 0;
    }
    buffer.with_buffer(count, |buffer| {
        let channels = buffer.as_slice();
        let stereo = channels.len() == 2;
        for frame in 0..count {
            let [left, right] = io.input.pop().expect("slots promised a frame");
            channels[0][frame] = if stereo { left } else { (left + right) * 0.5 };
            if stereo {
                channels[1][frame] = right;
            }
        }
        match plugin.try_lock() {
            Ok(mut instance) => {
                instance.process(
                    buffer,
                    &mut AuxiliaryBuffers {
                        inputs: &mut [],
                        outputs: &mut [],
                    },
                    context,
                );
            }
            Err(_) => {
                for channel in buffer.as_slice() {
                    channel[..count].fill(0.0);
                }
            }
        }
        for frame in 0..count {
            let channels = buffer.as_slice_immutable();
            let left = channels[0][frame];
            let right = if channels.len() == 2 {
                channels[1][frame]
            } else {
                left
            };
            let _ = io.output.push(StereoFrame([left, right]));
        }
    });
    count
}
pub(super) fn pump<
    P: Plugin<BackgroundTask = ()>,
    B: Fn(usize),
    const BLOCK: usize,
    const CAPACITY: usize,
>(
    plugin: Arc<Mutex<P>>,
    on_buffer: Arc<B>,
    mut io: Endpoints,
    stop: Arc<AtomicBool>,
    rate: f32,
) {
    let channels = P::AUDIO_IO_LAYOUTS[0].main_output_channels.unwrap().get() as usize;
    let mut buffer = plugin_buffer::PlanarBuffer::<CAPACITY>::new(channels, BLOCK);
    let mut context = Context::new(rate);
    while !stop.load(Ordering::Relaxed) {
        let count = process_once(&mut buffer, &mut context, &plugin, &mut io, BLOCK);
        if count == 0 {
            std::thread::park_timeout(Duration::from_micros(250));
        } else {
            on_buffer(count);
        }
    }
}
