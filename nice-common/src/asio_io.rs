//! The Windows transport: one ASIO driver, or two, driven through `azo`.
//!
//! This module replaces the transport that rode on cpal's ASIO host. The reason
//! is not speed but shape. cpal reaches ASIO through `asio-sys`, which compiles
//! Steinberg's headers: every Windows build needed the SDK, a C compiler and
//! `CPAL_ASIO_DIR`, and the gate — on the platform this engine ships on — could
//! not compile the Windows path at all. `.github/workflows/gate.yml` says so in
//! its own header, and refuses `--features standalone` for the same reason.
//! `azo` declares the same COM interfaces by hand (`azo-sys-0.4.1/src/lib.rs`),
//! so there is nothing to compile and the Windows path can be built, linted and
//! tested where the gate runs.
//!
//! The second reason is that ASIO is not a two-stream API, and cpal's host is
//! built as two. cpal does load the driver once per process and share it
//! (`cpal-0.18.2/src/host/asio/mod.rs:31`, `asio-sys-0.4.0/src/bindings/mod.rs:442-458`),
//! but it then builds input and output as two `Stream`s meeting in a shared
//! registry, and that seam is where its real bugs are: a second stream at another
//! buffer size is validated and then quietly ignored (`…/asio/stream.rs:875-876`),
//! a second stream at another rate re-rates the driver under the first one
//! (`…/stream.rs:1149-1165`, and `asio-sys …:605-652` answers a deferred rate by
//! tearing the driver down and reloading it while the other stream still holds its
//! buffers), `kAsioBufferSizeChange` assigns a new size to buffers allocated at
//! the old one (`…/stream.rs:1054-1069`), and dropping one direction leaves the
//! driver running with buffers nobody reads (`…/stream.rs:1095-1101`). None of it
//! is reachable here: one entry point, one driver per stream, one `createBuffers`
//! for both directions of each driver, and the combinations ASIO cannot express are
//! refused before the driver is asked anything.
//!
//! What the standard asks of a host, and where this file does it:
//!
//! * One stream per driver. [`open`] claims a slot in the process-global states and
//!   holds it until the returned [`Transport`] is dropped, and a second request for
//!   a driver this process already holds says so in words: two `createBuffers` on
//!   one instance is exactly the seam cpal's bugs live on. What this module does
//!   allow is a second *driver* — the same output block, copied onto another
//!   driver's channels, which is what a settings file's second device asks for.
//!   That stream is opened, started and closed by the same code as the first one,
//!   in a slot of its own.
//! * Both directions in one `createBuffers`, at a size the driver itself said it
//!   takes ([`crate::devices::Ladder::accepts`]), with the rate asked through
//!   `canSampleRate` before `setSampleRate` and read back afterwards. A copy's
//!   driver is asked for the same rate — a copy at another rate is a different
//!   signal — and a period of its own, because a ring sits between the two and
//!   does not care what it is.
//! * Callbacks that allocate nothing, lock nothing, never panic, and never call
//!   the driver back. The last one is what `azo`'s `Host` makes fatal: every
//!   driver method is marshalled to its pump thread with a blocking `SendMessageW`,
//!   so a driver call from a callback would be a deadlock with the driver standing
//!   inside our own callback. It is also why [`Transport::drop`] does its stopping
//!   from the host thread, where blocking is allowed.
//! * Teardown in the order the SDK uses — `stop`, then `disposeBuffers`
//!   (`asio-sys-0.4.0/src/bindings/mod.rs:987-999`) — with the errors reported
//!   instead of swallowed (`…:1040` ends in `.ok()`). One side at a time, so a
//!   driver that will not stop is met by itself and not in company.
//!
//! The callbacks are why there is more machinery here than driver calls. An
//! `ASIOCallbacks` is four bare function pointers with no user-data field
//! (`azo-sys-0.4.1/src/lib.rs:249-256`) and `createBuffers` forwards the pointer it
//! was given verbatim (`azo-0.6.1/src/driver.rs:404`), so a driver cannot be told
//! which stream it is answering: which stream a callback belongs to has to be in
//! the address of the function. Hence [`callbacks`] building one set per slot and
//! [`STATES`] being an array where one pointer used to be. `azo` itself keeps one
//! process-wide thing, the window class its host registers
//! (`azo-0.6.1/src/host/worker.rs:15-23`), which two pumps share without either
//! noticing; each `Host` is its own pump thread, its own apartment and its own
//! `CoInitialize`, which is why each driver gets one rather than sharing one.
//!
//! Two drivers are two clocks, so the copy is not a `memcpy`. The block the first
//! driver was handed goes into a ring and comes out the other side resampled to the
//! second driver's clock by [`Monitor`](crate::resample::Monitor) — the same
//! machinery an independent-rate monitor output runs on. What that buys and what it
//! costs, one sentence each: the live path never waits for the copy, so a copy that
//! falls behind drops frames and clicks while the rack goes on playing; and the
//! copy is a couple of the second driver's buffers late, because that is what
//! crossing a clock costs.
//!
//! One deliberate difference from the SDK path, stated because it is a behaviour
//! change and not an oversight: `outputReady` is never called. It exists so a
//! driver with hardware DSP learns the output is filled earlier in the block, and
//! calling it through `azo` means a `SendMessageW` to the pump thread from the
//! real-time thread — the deadlock above. cpal could do it because it calls the C
//! function directly, and probes once at `createBuffers` whether the driver
//! implements it at all (`asio-sys …:708-711`, `:1243-1245`). For every driver
//! measured on this machine it changes nothing; a machine whose driver needs it
//! would run one buffer late on output, and would hear about it from this
//! paragraph rather than from a hum.

// The whole module is a driver interface: buffers the driver owns, function
// pointers it calls, one global it reaches through. `nice-common` says
// `#![deny(unsafe_code)]` because nothing else in it should need any of that; this
// is the one place that does, and it says so at the door instead of leaving the
// exception to be inferred from scattered `unsafe`.
#![allow(unsafe_code)]

use crate::devices::{self, AsioStreamGuard, Ladder};
use crate::pcm::Pcm;
use crate::resample::{Monitor, StereoFrame};
use azo::Host;
use azo::driver::Driver as _;
use azo::dto::{ChannelId, ChannelInfoResponse};
use azo::sys::{Bool, Callbacks, MessageSelector, SampleType, Time};
use crossbeam::channel::{Receiver, Sender, bounded};
use rtrb::{Consumer, Producer, RingBuffer};
use std::cell::UnsafeCell;
use std::ffi::{c_long, c_void};
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::thread::yield_now;
use std::time::{Duration, Instant};

/// How many drivers one process may hold open: the live one, and the one carrying a
/// copy of its output. See [`Claim`].
const SLOTS: usize = 2;

/// ASIO hands the driver two halves per channel and names one of them per
/// callback (`buffers: [*mut c_void; 2]`, `azo-sys-0.4.1/src/lib.rs:364-368`).
const HALVES: usize = 2;

/// `phase` while the stream is being built: callbacks may run (they have nothing
/// to read yet) but a reset or resync request is not news — cpal learned that
/// ASIO4ALL and Focusrite-class drivers raise one during `start()`
/// (`cpal-0.18.2/src/host/asio/stream.rs:1013-1035`).
const STARTING: u8 = 0;
/// Streaming.
const RUNNING: u8 = 1;
/// Being taken away from the driver: a callback arriving now must touch nothing,
/// because the memory it points at is on its way back to the allocator.
const DETACHED: u8 = 2;

/// What the driver's own thread has to say about a stream that is already up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// Written for the person reading the panel, not for a log.
    pub message: String,
    /// True when the stream can no longer be trusted and the audio has to be
    /// reopened. False means one bad block: the stream carries on, and a host that
    /// counts dropouts may count this.
    pub invalidated: bool,
}

/// The two directions of one ASIO stream — and, if the settings asked for one, a
/// second driver carrying a copy of the output — as a host that owns settings wants
/// them.
///
/// ASIO has no second clock, no second buffer list and no second slot inside a
/// driver, so a request naming two of any of them for one driver is not a stream
/// this module can open. cpal answered that situation by using the first stream's
/// size and re-rating the driver for the second stream's rate; this answers it with
/// a sentence, because a user who chose 64 in and 128 out chose something that does
/// not exist.
pub struct Wanted<'a> {
    /// The driver for both directions, as the settings saved its name.
    pub input: &'a str,
    /// The same driver. Checked, not assumed.
    pub output: &'a str,
    /// Sample rate for both directions, as chosen.
    pub input_rate: u32,
    /// Checked against [`Wanted::input_rate`].
    pub output_rate: u32,
    /// Period for both directions, in frames.
    pub input_frames: u32,
    /// Checked against [`Wanted::input_frames`].
    pub output_frames: u32,
    /// How many input channels to open, from the driver's first.
    pub inputs: u32,
    /// How many output channels to open, from the driver's first.
    pub outputs: u32,
    /// First channel of the copy, in the 1-based spelling the settings page uses:
    /// `Some(3)` with a stereo route copies channels 1–2 onto 3–4. The copy is as
    /// wide as [`Wanted::outputs`], and `None` is one output group and no copy at
    /// all, whichever way [`Wanted::second_device`] is spelled.
    ///
    /// On the driver named by [`Wanted::output`] the copy has to sit above the first
    /// group, and both groups are asked for in the one `ASIOCreateBuffers` that
    /// driver gets, because that call takes the whole buffer list and there is no
    /// way to append a group later (`azo-0.6.1/src/driver.rs:92-103`, `:412-415`). On
    /// another driver the copy may start anywhere, and it is that driver's own
    /// `createBuffers` that carries it.
    pub second_output: Option<u32>,
    /// The driver carrying the copy. `None`, or the name in [`Wanted::output`],
    /// means the first driver with one more group on it; anything else is a second
    /// driver, live in this process beside the first and running on its own clock.
    pub second_device: Option<&'a str>,
}

impl<'a> Wanted<'a> {
    /// The second driver, when there is one. A copy naming the driver it is copied
    /// from is not a second driver: it is one driver with two groups, a different
    /// question with different rules, and the name is the whole of the difference as
    /// far as a settings file is concerned.
    fn copy_driver(&self) -> Option<&'a str> {
        match (self.second_output, self.second_device) {
            (Some(_), Some(driver)) if driver != self.output => Some(driver),
            _ => None,
        }
    }
}

/// What a host does with each block. Both runs are interleaved — `frames` frames
/// of [`Transport::inputs`] and [`Transport::outputs`] channels — so a host that
/// wants one physical input reads every `inputs`-th sample, and a host that owns
/// the driver's first two outputs writes them in order.
///
/// A copy does not widen either block: a host writes one signal and hears it on the
/// first group, on the channels above it ([`Wanted::second_output`]), or on a second
/// driver as well. The second driver runs a [`Pump`] of its own — [`Copier`] — and
/// the host's pump is never asked for its block twice.
///
/// Called on the driver's thread: allocate nothing, lock nothing, and do not call
/// back into the transport. The module documentation says why.
pub trait Pump: Send {
    /// One block the driver just captured.
    fn input(&mut self, interleaved: &[f32]);
    /// Fill the next block. It arrives silent, so a host with nothing to play
    /// writes nothing and hears silence.
    fn output(&mut self, interleaved: &mut [f32]);
}

/// One channel pair's way across the gap between two drivers' clocks.
///
/// A copy onto the same driver is a write of the block the pump just filled, and
/// needs nothing from here. A copy onto another driver is the same block read back
/// at a clock this process does not own: `rtrb` for the lock-free handover, and
/// [`Monitor`] for what is left of the two clocks disagreeing, because two drivers
/// both set to 48 kHz are not at the same rate and one-sample-in-one-out would run
/// one of them out of buffer within a minute.
///
/// Stereo is the shape because a pair then shares one ring, one interpolation phase
/// and one clock correction: a copy whose left ear is corrected a frame away from
/// its right is not the signal it was copied from. An odd last channel crosses
/// alone, because there is nobody to pair it with.
enum Feed {
    Stereo(Producer<StereoFrame>),
    Mono(Producer<f32>),
}

/// The other end, before it knows the rate and the period it will be read at —
/// which is the second driver's answer, and its state is built after the first
/// driver's.
enum Drain {
    Stereo(Consumer<StereoFrame>),
    Mono(Consumer<f32>),
}

/// A drain with its resampler, at the rate and period of the driver that reads it.
///
/// Both variants are boxed, and not because either is too big to hold: a `Monitor`
/// carries `TAPS` frames of history, which measures 1128 bytes for a pair and 616
/// for a lone channel, and the 512 bytes between them is
/// `clippy::large_enum_variant` — which `-D warnings` turns into a build failure —
/// for the same reason `nice-nam`'s `Streams` boxes the variant that outgrew its
/// neighbours (`nice-nam/src/standalone/audio.rs:46-53`). Boxing only the pair would
/// leave the same gap standing the other way round. A `Vec<Pull>` is built once,
/// when the second driver's buffers are cut, and the box costs nothing per frame.
enum Pull {
    Stereo(Box<Monitor<StereoFrame>>),
    Mono(Box<Monitor<f32>>),
}

/// The two ends of a crossing for a block `channels` wide, each holding room for
/// `capacity` frames.
fn cross(channels: usize, capacity: usize) -> (Vec<Feed>, Vec<Drain>) {
    let pairs = channels.div_ceil(2);
    let mut feeds = Vec::with_capacity(pairs);
    let mut drains = Vec::with_capacity(pairs);
    // A capacity of zero is a ring that cannot hold a frame, and a `Monitor`
    // waiting for a target it could never reach is silence forever.
    let capacity = capacity.max(1);
    for pair in 0..pairs {
        if pair * 2 + 1 < channels {
            let (sender, receiver) = RingBuffer::<StereoFrame>::new(capacity);
            feeds.push(Feed::Stereo(sender));
            drains.push(Drain::Stereo(receiver));
        } else {
            let (sender, receiver) = RingBuffer::<f32>::new(capacity);
            feeds.push(Feed::Mono(sender));
            drains.push(Drain::Mono(receiver));
        }
    }
    (feeds, drains)
}

impl Drain {
    /// This end, with its resampler, at the two clocks and the period of the driver
    /// that reads it. The crossing holds twice that period, so the reader does not
    /// come around to an empty ring because the writer was a block late, and does
    /// not outrun what it was given.
    fn pull(self, in_rate: f64, out_rate: f64, frames: usize) -> Pull {
        let frames = frames.max(1);
        let target = frames.saturating_mul(2);
        match self {
            Self::Stereo(receiver) => Pull::Stereo(Box::new(Monitor::new(
                receiver, in_rate, out_rate, target, frames,
            ))),
            Self::Mono(receiver) => Pull::Mono(Box::new(Monitor::new(
                receiver, in_rate, out_rate, target, frames,
            ))),
        }
    }
}

impl Pull {
    /// Once per block of the reading driver's clock: look at the ring and adjust
    /// for the two clocks having drifted apart since the last look.
    fn begin(&mut self) {
        match self {
            Self::Stereo(monitor) => monitor.begin_block(),
            Self::Mono(monitor) => monitor.begin_block(),
        }
    }

    /// The next frame, laid down at the front of `slot`. A pair takes two channels
    /// and a lone channel takes one, which is the whole reason this is a method on
    /// the pull and not a line in [`Copier::output`]: writing a pair's second
    /// channel where there is no pair writes the next frame's first channel.
    fn write(&mut self, slot: &mut [f32]) {
        match self {
            Self::Stereo(monitor) => {
                let StereoFrame([left, right]) = monitor.next_sample();
                if let Some((first, rest)) = slot.split_first_mut() {
                    *first = left;
                    if let Some(second) = rest.first_mut() {
                        *second = right;
                    }
                }
            }
            Self::Mono(monitor) => {
                if let Some(first) = slot.first_mut() {
                    *first = monitor.next_sample();
                }
            }
        }
    }
}

/// Hand one block to the driver on the far side of the crossing. `feed[i]` carries
/// the block's channels `2i` and `2i + 1` — or the odd last one alone — which is the
/// same pairing [`Copier`] reads back, and between them the only two places that
/// say so.
///
/// A full ring means the second driver has stopped taking frames: unplugged,
/// stopped from its own control panel, or a process that lost its CPU for a hundred
/// milliseconds. The live path does not wait for any of that, so the newest frame is
/// the one thrown away and `lost` says it happened.
fn push_block(feed: &mut [Feed], block: &[f32], channels: usize, lost: &mut bool) {
    let Some(frames) = block.len().checked_div(channels.max(1)) else {
        return;
    };
    for (pair, feed) in feed.iter_mut().enumerate() {
        let left = pair * 2;
        // Past the end of the block is a pairing built wrong, which is not a thing a
        // driver's thread gets to panic about; silence is, and silence is also what
        // the far end would have heard.
        let sample = |frame: usize, channel: usize| {
            block
                .get(frame * channels + channel)
                .copied()
                .unwrap_or_default()
        };
        match feed {
            Feed::Stereo(producer) => {
                for frame in 0..frames {
                    let pair = StereoFrame([sample(frame, left), sample(frame, left + 1)]);
                    if producer.push(pair).is_err() {
                        *lost = true;
                    }
                }
            }
            Feed::Mono(producer) => {
                for frame in 0..frames {
                    if producer.push(sample(frame, left)).is_err() {
                        *lost = true;
                    }
                }
            }
        }
    }
}

/// The pump the first driver runs: the host's own, with a copy of every block it
/// fills handed across the crossing on the way out.
struct Repeater<P> {
    inner: P,
    feed: Vec<Feed>,
    channels: usize,
    notify: Sender<Failure>,
    /// The driver the copy is late for, in the one sentence a host hears about it.
    name: String,
    /// Once, because a driver that is behind is behind every block, and a panel
    /// that says it sixty times a second says nothing.
    warned: bool,
}

impl<P: Pump> Pump for Repeater<P> {
    fn input(&mut self, interleaved: &[f32]) {
        self.inner.input(interleaved);
    }

    fn output(&mut self, interleaved: &mut [f32]) {
        // The host first, always: the crossing is the last thing the live block
        // touches, and a copy is never a reason for a rack to be late.
        self.inner.output(interleaved);
        let mut lost = false;
        push_block(&mut self.feed, interleaved, self.channels, &mut lost);
        if lost && !self.warned {
            self.warned = true;
            let _ = self.notify.try_send(Failure {
                message: format!(
                    "'{}' fell behind and lost part of the copy; the first driver is unaffected",
                    self.name
                ),
                invalidated: false,
            });
        }
    }
}

/// The pump the second driver runs: the block the first driver was handed, read back
/// at this driver's own clock. No input side — a copy is an output, and this module
/// never opens a second driver's inputs at all.
struct Copier {
    pulls: Vec<Pull>,
    channels: usize,
}

impl Pump for Copier {
    fn input(&mut self, _interleaved: &[f32]) {}

    fn output(&mut self, interleaved: &mut [f32]) {
        if self.channels == 0 {
            return;
        }
        for pull in &mut self.pulls {
            pull.begin();
        }
        // What is not written stays as it was, which is silent: the block arrives
        // zeroed and an empty ring at the start of a stream is normal, not a fault.
        for frame in interleaved.chunks_exact_mut(self.channels) {
            for (pair, pull) in self.pulls.iter_mut().enumerate() {
                if let Some(slot) = frame.get_mut(pair * 2..) {
                    pull.write(slot);
                }
            }
        }
    }
}

/// One channel's two halves, and the bytes the driver said they hold.
struct Channel {
    halves: [*mut c_void; HALVES],
    pcm: Pcm,
}

impl Channel {
    /// One half as the driver's own bytes. A null half is a driver that allocated
    /// less than it promised, which is an answer rather than a crash.
    ///
    /// The `&mut` comes out of a `&self` because the bytes are not borrowed from
    /// the `Channel`: the driver owns them, and this is the handle to them. Two
    /// calls could hand out the same bytes twice, which is why the only route
    /// here is [`State::exchange`] — reached from the driver's own thread, the one
    /// thread ASIO runs its callbacks on — and why no `&State` leaves this module:
    /// [`Transport`] keeps the address, not a borrow, and reads only its own
    /// copies of the numbers the driver gave back at open.
    #[allow(clippy::mut_from_ref)] // a handle to driver memory, not a borrow of self
    fn plane(&self, half: usize, frames: usize) -> Option<&mut [u8]> {
        let base = *self.halves.get(half)?;
        (!base.is_null()).then(|| unsafe {
            std::slice::from_raw_parts_mut(base.cast::<u8>(), self.pcm.block_bytes(frames))
        })
    }
}

/// Everything the driver's thread needs, reached through [`STATES`] because ASIO
/// hands a host no reference of any kind: `bufferSwitch` is a bare function pointer
/// with two arguments (`azo-sys-0.4.1/src/lib.rs:220-224`), so the only way a
/// callback can find out which of the two streams it belongs to is which entry of
/// [`STATES`] it was compiled to read.
struct State {
    /// The driver this state belongs to, in the sentences the driver's complaints
    /// are turned into. With two of them open, "the ASIO driver changed its buffer"
    /// is not an answer unless it says which.
    name: String,
    inputs: Vec<Channel>,
    outputs: Vec<Channel>,
    /// The copy this driver was asked to carry on channels of its own: the same
    /// block laid down again where the group above does not reach, each channel in
    /// its own format. Empty unless [`Wanted::second_output`] named a start on this
    /// same driver — a copy on a second driver is a second state, reading a
    /// [`Copier`].
    mirror: Vec<Channel>,
    frames: usize,
    /// Interleaved scratch, allocated at open so the callback allocates nothing.
    /// cpal's callback resized a `Vec` per block on the way to its own
    /// interleaving (`cpal-0.18.2/src/host/asio/stream.rs:243-248`).
    capture: UnsafeCell<Vec<f32>>,
    playback: UnsafeCell<Vec<f32>>,
    /// One owner, handed to the driver's thread and touched by nobody else while
    /// [`State::phase`] is not [`DETACHED`] — which is what makes the `&mut` this
    /// cell gives out legitimate.
    pump: UnsafeCell<Box<dyn Pump>>,
    errors: Sender<Failure>,
    phase: AtomicU8,
    /// How many callbacks are inside right now. See [`DETACHED`].
    inside: AtomicUsize,
    /// The half the last callback was about, for the drivers that fire twice for
    /// one half — Focusrite USB ASIO and ReaRoute do
    /// (`cpal-0.18.2/src/host/asio/stream.rs:227-230`).
    last_half: AtomicUsize,
    /// The rate this stream was opened at, which is what `sampleRateDidChange` is
    /// compared against. asio-sys keeps the same baseline in a process-global
    /// (`asio-sys-0.4.0/src/bindings/mod.rs:374`) so one stream can silence
    /// another's news; each stream here has its own state and its own number, which
    /// is the difference, and it matters now that two can be open at once.
    rate: f64,
    /// `kAsioOverload` reports, so a host that shows dropouts has a number.
    overloads: AtomicU64,
}

impl State {
    /// One word to the host, on a bounded channel: a driver that repeats itself
    /// cannot outrun a panel that reads it once a frame.
    fn fail(&self, message: String, invalidated: bool) {
        let _ = self.errors.try_send(Failure {
            message,
            invalidated,
        });
    }

    /// One block in from one half: gather each channel's own bytes into
    /// interleaved frames. False means a channel has no buffer, and then nothing
    /// is handed on — a half-filled block would be a click.
    unsafe fn capture(&self, half: usize) -> bool {
        let channels = self.inputs.len();
        if channels == 0 {
            return true;
        }
        let scratch = unsafe { &mut *self.capture.get() };
        for (index, channel) in self.inputs.iter().enumerate() {
            let Some(plane) = channel.plane(half, self.frames) else {
                return false;
            };
            let width = channel.pcm.bytes_per_frame();
            for frame in 0..self.frames {
                let source = &plane[frame * width..][..width];
                let target = &mut scratch[frame * channels + index..][..1];
                channel.pcm.read(source, target);
            }
        }
        unsafe { (*self.pump.get()).input(scratch) };
        true
    }

    /// One block out into one half: fill the interleaved scratch, then lay each
    /// channel's bytes down in its own buffer — and lay the same block down again
    /// on the copy's channels, which is the whole of what a second output group
    /// is.
    unsafe fn playback(&self, half: usize) {
        let channels = self.outputs.len();
        if channels == 0 {
            return;
        }
        let scratch = unsafe { &mut *self.playback.get() };
        // Silence first: a host with nothing to say says nothing, and a driver
        // whose buffers were never written would otherwise play whatever they
        // held. cpal filled with equilibrium only from inside its own callback,
        // so a stream that was started but not yet played could play a driver's
        // garbage (`cpal-0.18.2/src/host/asio/stream.rs:442-451`, `:648`).
        scratch.fill(0.0);
        unsafe { (*self.pump.get()).output(scratch) };
        let block = &*scratch;
        // Both groups read the same frames at the same stride: they were opened
        // the same width, and the copy is the same signal, not a second one.
        // `take(channels)` is the belt on that — a channel past the block's width
        // could only come from a state built wrong, and here it is silence rather
        // than a panic on the driver's own thread.
        for group in [&self.outputs, &self.mirror] {
            for (index, channel) in group.iter().enumerate().take(channels) {
                let Some(plane) = channel.plane(half, self.frames) else {
                    continue;
                };
                let width = channel.pcm.bytes_per_frame();
                for frame in 0..self.frames {
                    let source = &block[frame * channels + index..][..1];
                    let target = &mut plane[frame * width..][..width];
                    channel.pcm.write(source, target);
                }
            }
        }
    }

    /// The body of both buffer callbacks.
    fn exchange(&self, half: usize, direct: bool) {
        // Twice for the same half is a driver doing something we have measured it
        // doing. With `directProcess` there is no other half to wait for, so the
        // same index twice really is two blocks — and the SDK path drops the flag
        // on the floor (`asio-sys-0.4.0/src/bindings/mod.rs:1226`) while cpal's
        // duplicate guard then swallows the second call
        // (`cpal-0.18.2/src/host/asio/stream.rs:227-230`), which is how a driver in
        // that mode loses every other block.
        if !direct && self.last_half.load(Ordering::Relaxed) == half {
            return;
        }
        self.last_half.store(half, Ordering::Relaxed);
        // Input first, so a capture and the playback that answers it are as close
        // as this API allows.
        unsafe {
            if !self.capture(half) {
                return;
            }
            self.playback(half);
        }
    }
}

/// A callback that has announced itself, so the owner knows nothing may be freed.
struct Inside<'a>(&'a State);

impl<'a> Inside<'a> {
    /// Announce first, then look: the owner sets [`DETACHED`] before it reads this
    /// count and both sides use `SeqCst`, so a callback can never be told it may
    /// proceed and then be freed under itself.
    fn enter(state: &'a State) -> Option<Self> {
        state.inside.fetch_add(1, Ordering::SeqCst);
        if state.phase.load(Ordering::SeqCst) == DETACHED {
            state.inside.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Self(state))
    }

    /// Whether the stream is up, which is the only time a driver's complaints are
    /// news.
    fn running(&self) -> bool {
        self.0.phase.load(Ordering::Relaxed) == RUNNING
    }
}

impl Drop for Inside<'_> {
    fn drop(&mut self) {
        self.0.inside.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The states a driver's callbacks may reach, one per [`SLOTS`] slot: the live
/// stream in the first, the driver carrying a copy in the second. A callback finds
/// its own because it was compiled to read that entry — see [`callbacks`].
static STATES: [AtomicPtr<State>; SLOTS] = [const { AtomicPtr::new(null_mut()) }; SLOTS];

/// Whether a slot is spoken for, including the stretch of an open where its state is
/// still null. Held by the live [`Transport`] and released when it is dropped.
static CLAIMED: [AtomicBool; SLOTS] = [const { AtomicBool::new(false) }; SLOTS];

/// The claim on the slot, or the two slots, this process is about to open drivers
/// in, released by being dropped — so every early return in [`open`] gives the
/// process its ASIO slots back without repeating itself.
///
/// ASIO permits one loaded driver per process, and cpal and asio-sys hold
/// themselves to it with a process-global `Asio` that refuses a second driver by
/// name (`cpal-0.18.2/src/host/asio/mod.rs:27-44`,
/// `asio-sys-0.4.0/src/bindings/mod.rs:442-458`). Two instances of one CLSID is not
/// that rule: `azo` makes a fresh COM instance per handle
/// (`azo-0.6.1/src/driver.rs:218-234`) and keeps nothing between them but the host
/// window class, and every driver here that can be picked as a second device is a
/// separate registration with a separate CLSID and private state. What is
/// unmeasured is a driver's own reaction to a second instance of *itself*, which is
/// why a second slot is claimed for a second driver and never for a second stream
/// on one.
struct Claim {
    /// How many slots from the front this claim holds: one, or both.
    held: usize,
}

impl Claim {
    /// The claim on one slot, and on the copy's slot too when there is a copy to
    /// carry. Both or neither: half of a two-driver stream is not a stream.
    fn take(second: bool) -> Option<Self> {
        let want = if second { SLOTS } else { 1 };
        let mut held = 0;
        while held < want {
            if CLAIMED[held]
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                for claimed in &CLAIMED[..held] {
                    claimed.store(false, Ordering::Release);
                }
                return None;
            }
            held += 1;
        }
        Some(Self { held })
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        for claimed in &CLAIMED[..self.held] {
            claimed.store(false, Ordering::Release);
        }
    }
}

/// The four functions a driver is allowed to call, for the stream in slot `SLOT`.
/// All four are real: `azo`'s own example leaves two of them `unimplemented!()`
/// (`azo-0.6.1/examples/sine.rs`), which is a panic crossing an `extern "system"`
/// boundary into a driver's thread — a driver that happens to use either one
/// would take the process down with it.
///
/// The parameter is not decoration, and it cannot be an argument: an `ASIOCallbacks`
/// is four bare function pointers with no user-data field
/// (`azo-sys-0.4.1/src/lib.rs:249-256`) and `createBuffers` forwards the pointer it
/// is given verbatim (`azo-0.6.1/src/driver.rs:404`), so a driver cannot be told
/// which stream it is answering. Which stream a callback belongs to therefore has to
/// be in the address of the function, which is what a const parameter is and what
/// nothing else in the language is.
const fn callbacks<const SLOT: usize>() -> Callbacks {
    Callbacks {
        buffer_switch: buffer_switch::<SLOT>,
        sample_rate_did_change: sample_rate_did_change::<SLOT>,
        asio_message: asio_message::<SLOT>,
        buffer_switch_time_info: buffer_switch_time_info::<SLOT>,
    }
}

/// One set per slot, and a `static` rather than a `const` because the address is the
/// thing the driver is given and it has to go on meaning the same thing after
/// [`open`] returns. A third [`SLOTS`] is a third entry here.
static CALLBACKS: [Callbacks; SLOTS] = [callbacks::<0>(), callbacks::<1>()];

/// The selectors this host answers. Spelled as numbers because the `value` of a
/// `SELECTOR_SUPPORTED` query is a selector the driver is asking about, and the
/// answer has to be about numbers, not about a type.
const WANT_SUPPORTED: c_long = MessageSelector::SELECTOR_SUPPORTED.0;
const WANT_ENGINE: c_long = MessageSelector::ENGINE_VERSION.0;
const WANT_RESET: c_long = MessageSelector::RESET_REQUEST.0;
const WANT_SIZE: c_long = MessageSelector::BUFFER_SIZE_CHANGE.0;
const WANT_RESYNC: c_long = MessageSelector::RESYNC_REQUEST.0;

/// `ASIOGetChannelInfo`'s type code, as a format this module can convert.
///
/// The numbering is the header's own — `azo`'s names and Steinberg's `ASIOST…`
/// codes line up one for one, checked against the mirror `asio-sys` carries
/// (`azo-sys-0.4.1/src/lib.rs:88-113` against
/// `asio-sys-0.4.0/src/bindings/mod.rs:158-183`). A code with no arm here is
/// refused by name; `asio-sys` panics on one (`…:1097`), which is a thing a driver
/// could do to a host on the way into a stream.
fn pcm_of(sample_type: SampleType) -> Option<Pcm> {
    let format = match sample_type {
        SampleType::PCM_I16_MSB => Pcm::Int16 { big_endian: true },
        SampleType::PCM_I24_MSB => Pcm::Int24 { big_endian: true },
        SampleType::PCM_I32_MSB => Pcm::Int32 { big_endian: true },
        SampleType::PCM_F32_MSB => Pcm::Float32 { big_endian: true },
        SampleType::PCM_F64_MSB => Pcm::Float64 { big_endian: true },
        SampleType::PCM_I32_MSB_16 => Pcm::Padded32 {
            bits: 16,
            big_endian: true,
        },
        SampleType::PCM_I32_MSB_18 => Pcm::Padded32 {
            bits: 18,
            big_endian: true,
        },
        SampleType::PCM_I32_MSB_20 => Pcm::Padded32 {
            bits: 20,
            big_endian: true,
        },
        SampleType::PCM_I32_MSB_24 => Pcm::Padded32 {
            bits: 24,
            big_endian: true,
        },
        SampleType::PCM_I16_LSB => Pcm::Int16 { big_endian: false },
        SampleType::PCM_I24_LSB => Pcm::Int24 { big_endian: false },
        SampleType::PCM_I32_LSB => Pcm::Int32 { big_endian: false },
        SampleType::PCM_F32_LSB => Pcm::Float32 { big_endian: false },
        SampleType::PCM_F64_LSB => Pcm::Float64 { big_endian: false },
        SampleType::PCM_I32_LSB_16 => Pcm::Padded32 {
            bits: 16,
            big_endian: false,
        },
        SampleType::PCM_I32_LSB_18 => Pcm::Padded32 {
            bits: 18,
            big_endian: false,
        },
        SampleType::PCM_I32_LSB_20 => Pcm::Padded32 {
            bits: 20,
            big_endian: false,
        },
        SampleType::PCM_I32_LSB_24 => Pcm::Padded32 {
            bits: 24,
            big_endian: false,
        },
        _ => return None,
    };
    Some(format)
}

/// The driver's own last message, for the errors a user reads.
///
/// `ASIOGetErrorMessage` exists for exactly this, and the SDK path threw it away
/// with the uninitialized `ASIODriverInfo` it handed `ASIOInit`
/// (`asio-sys-0.4.0/src/bindings/mod.rs:463-471`). Most drivers answer this with an
/// empty string; when one answers with a sentence, it is the best sentence there
/// is. `azo`'s handle type has no name to write, hence the parameter in full.
fn complaint(driver: &impl azo::driver::Driver, problem: &str) -> String {
    let said = driver.last_error();
    match said
        .as_c_str()
        .to_str()
        .ok()
        .filter(|text| !text.is_empty())
    {
        Some(detail) => format!("{problem}: {detail}"),
        None => problem.to_owned(),
    }
}

/// A driver's refusal, in the two parts a reader needs: what the driver said and
/// which code it said it with.
fn refusal(problem: String, error: azo::Error) -> String {
    format!("{problem} ({error})")
}

/// `bufferSwitch`: one half of every buffer is ready, in the stream this copy of
/// the function was compiled for.
unsafe extern "system" fn buffer_switch<const SLOT: usize>(half: c_long, direct: Bool) {
    let Some(state) = (unsafe { STATES[SLOT].load(Ordering::Acquire).as_ref() }) else {
        return;
    };
    let Some(guard) = Inside::enter(state) else {
        return;
    };
    let Ok(half) = usize::try_from(half) else {
        return;
    };
    if half >= HALVES {
        return;
    }
    // An undefined `directProcess` is read as the double-buffered case, which is
    // the one the timing of this API assumes.
    let direct = bool::try_from(direct).unwrap_or(false);
    guard.0.exchange(half, direct);
}

/// `sampleRateDidChange`: someone moved the clock — the driver's own control
/// panel, another application, or a cable.
unsafe extern "system" fn sample_rate_did_change<const SLOT: usize>(rate: f64) {
    let Some(state) = (unsafe { STATES[SLOT].load(Ordering::Acquire).as_ref() }) else {
        return;
    };
    let Some(guard) = Inside::enter(state) else {
        return;
    };
    // One hertz, because a driver asked for 48000 that answers 47999.99 does not
    // have a different clock; it has the same one and a coarse converter.
    if (rate - state.rate).abs() < 1.0 || !guard.running() {
        return;
    }
    state.fail(
        format!("Sample rate changed to {rate} Hz by '{}'", state.name),
        true,
    );
}

/// `asioMessage`: the driver's out-of-band requests, on the driver's thread.
unsafe extern "system" fn asio_message<const SLOT: usize>(
    selector: MessageSelector,
    value: c_long,
    _message: *const c_void,
    _opt: *const f64,
) -> c_long {
    let Some(state) = (unsafe { STATES[SLOT].load(Ordering::Acquire).as_ref() }) else {
        return Bool::FALSE.0;
    };
    match selector {
        // Yes only to what is answered below. The numbering is the header's own,
        // checked the same way as `pcm_of`'s.
        MessageSelector::SELECTOR_SUPPORTED => match value {
            WANT_SUPPORTED | WANT_ENGINE | WANT_RESET | WANT_SIZE | WANT_RESYNC => Bool::TRUE.0,
            _ => Bool::FALSE.0,
        },
        // Two, as every host since: it is what makes a driver send
        // `kAsioBufferSizeChange` and `kAsioOverload` at all.
        MessageSelector::ENGINE_VERSION => 2,
        MessageSelector::RESET_REQUEST | MessageSelector::RESYNC_REQUEST => {
            // A request during startup is a driver reconfiguring itself on the way
            // up, not a broken stream: cpal's comment names ASIO4ALL and Focusrite
            // for it (`cpal-0.18.2/src/host/asio/stream.rs:1013-1035`).
            if Inside::enter(state).as_ref().is_some_and(|g| g.running()) {
                let what = if selector == MessageSelector::RESET_REQUEST {
                    "reset"
                } else {
                    "resync"
                };
                state.fail(
                    format!(
                        "'{}' asked to {what}; the audio needs to be reopened",
                        state.name
                    ),
                    true,
                );
            }
            Bool::TRUE.0
        }
        MessageSelector::BUFFER_SIZE_CHANGE => {
            // The honest answer to a driver that resized underneath us: we cannot
            // follow, because the rings, the monitor and the plugin's block were all
            // sized for the old one. cpal assigned the new number to buffers
            // allocated at the old size
            // (`cpal-0.18.2/src/host/asio/stream.rs:1054-1069`).
            if Inside::enter(state).as_ref().is_some_and(|g| g.running()) {
                state.fail(
                    format!("'{}' changed its buffer to {value} frames", state.name),
                    true,
                );
            }
            Bool::TRUE.0
        }
        MessageSelector::OVERLOAD => {
            if let Some(guard) = Inside::enter(state) {
                guard.0.overloads.fetch_add(1, Ordering::Relaxed);
                let message = format!("'{}' reported an overload", guard.0.name);
                guard.0.fail(message, false);
            }
            Bool::TRUE.0
        }
        // Latencies we do not use, time info we declined, and everything else we do
        // not claim. No is an answer, and the only safe one for a selector nobody
        // has read this driver's documentation for.
        _ => Bool::FALSE.0,
    }
}

/// `bufferSwitchTimeInfo`, for the drivers that use it whether we asked or not.
///
/// The time is handed straight back and never read: nothing in frost's audio path
/// carries a driver timestamp, and not dereferencing it is also how this module
/// stays clear of `asio-sys`'s mirror of `ASIOTime`, whose field order its own
/// tests never checked (`asio-sys-0.4.0/src/bindings/mod.rs:310-328`, `:1296-1314`).
unsafe extern "system" fn buffer_switch_time_info<const SLOT: usize>(
    time: *mut Time,
    half: c_long,
    direct: Bool,
) -> *mut Time {
    unsafe { buffer_switch::<SLOT>(half, direct) };
    time
}

/// What one driver is asked to carry, in the shape [`Side::open`] works from.
///
/// Both drivers an [`open`] makes get one of these: the live stream's comes out of
/// the [`Wanted`] the caller passed, a copy's out of the parts of that request a
/// second clock has to answer for itself. One type for both is what lets one
/// function open either, and what keeps the second driver out of the first one's
/// error paths.
struct Asking {
    /// Registry name of the driver, as the settings spell it and as every sentence
    /// this module returns quotes.
    driver: String,
    /// Where it was found in the registry. Carried rather than looked up again so
    /// that one [`open`] can tell two names meaning one driver apart from two names
    /// meaning two clocks.
    meta: azo::driver::Metadata,
    /// Which of [`STATES`] and [`CALLBACKS`] this driver's callbacks read. Nothing
    /// else can tell two live drivers apart: `ASIOCallbacks` is four function
    /// pointers with no user-data field (`azo-sys-0.4.1/src/lib.rs:249-256`).
    slot: usize,
    inputs: u32,
    outputs: u32,
    /// Zero-based index of the driver's own first output channel for this block.
    /// Zero for the live stream, whose channels the host's route picks inside; a
    /// copy's is the channel the settings said the copy starts on, minus one.
    first_output: u32,
    /// A second group of channels on *this* driver repeating the first group's
    /// block, 1-based as the settings spell it. Only a live stream can have one: a
    /// copy riding a second driver does not need a second group to be a copy.
    mirror: Option<u32>,
    /// The period to ask for.
    frames: u32,
    /// Whether this driver has to take exactly that period. The live stream's does
    /// — the host's rings, its monitor and every plugin's block were sized from it
    /// — and a copy's does not, because the crossing in front of it absorbs
    /// whatever the second clock hands out.
    exact_period: bool,
    /// The rate to ask for. Both drivers are asked for the same one: a copy at
    /// another rate is a copy of a different signal.
    rate: u32,
}

impl Asking {
    /// The same request in the shape [`Wanted`] has. [`Side::open`] asks a driver
    /// the questions [`Wanted`] is a list of, and reading the same names whether the
    /// driver is the host's or a copy's is the difference between one open and two
    /// that drift apart. No driver carries a copy of itself, which is why
    /// [`Wanted::copy_driver`] answers `None` for one of these, correctly.
    fn wanted(&self) -> Wanted<'_> {
        Wanted {
            input: &self.driver,
            output: &self.driver,
            input_rate: self.rate,
            output_rate: self.rate,
            input_frames: self.frames,
            output_frames: self.frames,
            inputs: self.inputs,
            outputs: self.outputs,
            second_output: self.mirror,
            second_device: None,
        }
    }
}

/// One driver a [`Transport`] holds open: the state its callbacks reach while it is
/// up, and the way to say goodbye to it.
///
/// Dropping one stops the driver, gives its buffers back and reports anything that
/// went wrong through the transport's failure channel, in the order the SDK uses
/// (`asio-sys-0.4.0/src/bindings/mod.rs:987-999`). It is a `Drop` and not a list of
/// calls because an [`open`] that fails on its way to the second driver has to tear
/// the first one down, and a second vendor's driver is exactly where an open starts
/// failing for reasons nobody predicted from a header file.
struct Side {
    /// Which of [`STATES`] and [`CALLBACKS`] this driver was given.
    slot: usize,
    state: *mut State,
    /// The driver, as a boxed farewell. `azo` keeps its `host` module private and
    /// re-exports only `Host` (`azo-0.6.1/src/lib.rs:6-17`), so the handle
    /// `create_driver` hands back has no name to put in a field. What is still
    /// needed from it after the open is `stop` and `dispose_buffers`, and that
    /// travels as this closure, which captured the handle while its type was still
    /// inferable. It reports what it could not do.
    close: Option<Box<dyn FnOnce() -> Vec<String> + Send>>,
    /// The transport's failure channel. A driver that would not stop is the host's
    /// news, not the news of whichever thread happened to drop the transport.
    notify: Sender<Failure>,
}

/// An open ASIO stream: both directions of one driver, and a copy of the output on
/// a second driver when the settings asked for one.
///
/// Dropping it stops each driver, gives its buffers back and reports anything that
/// went wrong through [`Transport::failure`].
pub struct Transport {
    /// Dropped first, which is what its position in this list means: a copy still
    /// running when the live driver is stopped keeps calling its own `bufferSwitch`
    /// into a ring nobody is filling any more, and its teardown is the one with new
    /// material in it. Whatever a second vendor's driver does on the way out, it
    /// should not do it while the first one is being taken apart. Read by nothing
    /// else, hence the name.
    _copy: Option<Side>,
    /// The driver the host's [`Pump`] runs on. Every number below describes this
    /// one; the copy has its own period and does not care about these.
    first: Side,
    frames: u32,
    rate: u32,
    inputs: u32,
    outputs: u32,
    errors: Receiver<Failure>,
    /// So the panel goes on answering `None` to ASIO capability questions while
    /// this stream is up (`crate::devices::asio_stream_live`).
    _guard: AsioStreamGuard,
    /// The slot, or both of them, this stream holds. Released with this value,
    /// which is after both sides are gone.
    _claim: Claim,
}

// The handle travels inside `close`, and `azo`'s proxy is `Send`; the state is
// reached through one of the process-global slots, guarded by the atomics above and
// by the wait in `Drop`; the pump the state owns is `Send` by the trait's own bound.
// So a side may be moved, and dropped, on any thread.
unsafe impl Send for Side {}

// A transport is `Send` because both of its sides are. It is deliberately not
// `Sync`: `failure` only reads a channel, but two threads holding one transport
// would be two owners of the drivers behind it.
unsafe impl Send for Transport {}

impl Drop for Side {
    fn drop(&mut self) {
        // The SDK's order, and the SDK's error reporting which it did not have.
        // Both calls are `SendMessageW` to this driver's pump thread, which is why
        // they happen here and never from a callback.
        let problems = self.close.take().map_or_else(Vec::new, |close| close());
        // Then take the state away from the driver's thread and wait out a callback
        // that was already inside. `stop` is meant to return only after the last
        // callback has run; a driver that breaks that promise is met by the wait
        // below, and by a leak rather than a free if it is still inside after it.
        unsafe { (*self.state).phase.store(DETACHED, Ordering::SeqCst) };
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let inside = unsafe { (*self.state).inside.load(Ordering::SeqCst) };
            if inside == 0 {
                break;
            }
            if Instant::now() >= deadline {
                // Leaked on purpose: one block of scratch, against a callback
                // writing into memory that was handed back.
                break;
            }
            yield_now();
        }
        let still_inside = unsafe { (*self.state).inside.load(Ordering::SeqCst) } != 0;
        unsafe {
            STATES[self.slot].store(null_mut(), Ordering::Release);
            if !still_inside {
                drop(Box::from_raw(self.state));
            }
        }
        // A driver that refused to stop is worth hearing about: it is the case
        // where the next open may fail, and the SDK path reported it by doing
        // nothing at all (`asio-sys-0.4.0/src/bindings/mod.rs:1040`).
        for message in problems {
            let _ = self.notify.try_send(Failure {
                message,
                invalidated: false,
            });
        }
        // A slot comes free only after this body has run: the claim is held by the
        // transport, which drops its sides before it drops the claim, and a host
        // still holding the other end of the channel sees the messages sent above.
    }
}

impl Transport {
    /// Frames per block — the number the caller asked for, or nothing. This is the
    /// live driver's number: a copy rides a period of its own behind a ring that
    /// absorbs the difference, and the host's blocks are this wide.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The rate the stream was opened at. Both drivers were asked for this one.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// How many interleaved channels [`Pump::input`] carries.
    pub fn inputs(&self) -> u32 {
        self.inputs
    }

    /// How many interleaved channels [`Pump::output`] carries. A copy on a second
    /// driver carries the same block again, and is not part of this number.
    pub fn outputs(&self) -> u32 {
        self.outputs
    }

    /// `kAsioOverload` reports so far, for a host that counts dropouts. Counted on
    /// the live driver, because that is the one whose blocks the host is out of.
    pub fn overloads(&self) -> u64 {
        unsafe { &*self.first.state }
            .overloads
            .load(Ordering::Relaxed)
    }

    /// The next thing a driver said, if either of them said anything. Nothing here
    /// reopens or repairs anything by itself: `invalidated` is the part that says
    /// the host has to. Both drivers report through this one channel, and every
    /// message says which of them it came from.
    pub fn failure(&self) -> Option<Failure> {
        self.errors.try_recv().ok()
    }
}

/// Open the driver, or the two drivers, `wanted` names, and start them.
///
/// Either the live driver takes exactly the rate and the period asked for, or this
/// returns a sentence about which of the two it refused and what it would have
/// taken. There is no "give me the nearest size" for it: a caller that sized its
/// rings, its monitor and its plugin's block around 64 frames has no use for a
/// stream that came up at 96. A second driver carrying a copy is asked for the same
/// rate and for a period it may round up to its own ladder, because the crossing in
/// front of it turns one block into another without asking the host.
///
/// The pump is taken by value and must own everything it touches (`'static`):
/// it lives in the state the driver's callback reaches through, which outlives
/// this function and — in the one case where a callback is still inside when the
/// transport is dropped — outlives the transport too.
pub fn open(wanted: Wanted<'_>, pump: impl Pump + 'static) -> Result<Transport, String> {
    if wanted.input != wanted.output {
        return Err(format!(
            "ASIO carries input and output on one driver: '{}' and '{}' cannot share a stream. \
             Pick the same driver on both sides.",
            wanted.input, wanted.output
        ));
    }
    if wanted.input_rate != wanted.output_rate {
        return Err(format!(
            "An ASIO driver runs one clock: {} Hz in and {} Hz out cannot share a stream. \
             Pick the same rate on both sides.",
            wanted.input_rate, wanted.output_rate
        ));
    }
    if wanted.input_frames != wanted.output_frames {
        return Err(format!(
            "An ASIO driver allocates one buffer for both directions: {} frames in and {} out \
             cannot share a stream. Pick the same buffer size on both sides.",
            wanted.input_frames, wanted.output_frames
        ));
    }
    if wanted.inputs == 0 && wanted.outputs == 0 {
        return Err("An ASIO stream needs at least one input or one output channel".into());
    }
    // A copy needs something to copy, and where it may start is arithmetic rather
    // than a question for the hardware. Which arithmetic depends on whose channels
    // it lands on: on the driver it was copied from it has to start beyond the
    // group it copies, and on a driver of its own channel 1 is as good a place as
    // any, since that speaker is plugged into that interface.
    let copy = wanted.copy_driver();
    if wanted.second_output.is_some() && wanted.outputs == 0 {
        return Err(
            "A second output pair copies the first one, and this stream has no output channels \
             to copy"
                .into(),
        );
    }
    if let Some(start) = wanted
        .second_output
        .filter(|start| copy.is_none() && *start <= wanted.outputs)
    {
        return Err(format!(
            "The second output pair starts at output {start}, which the first {} already \
             use: pick {} or later",
            wanted.outputs,
            wanted.outputs + 1
        ));
    }
    let rate = wanted.input_rate;
    let frames = wanted.input_frames;
    let driver_name = wanted.input;

    // Both names are looked up in the registry before either driver is loaded, so a
    // settings file naming one that has been uninstalled since says so without a
    // second interface having been initialized and torn down first. The class id
    // comes along for the pair that is not two drivers: two registry names for one
    // COM class would mean loading one driver twice into one process, which is not
    // a thing any driver here has been observed surviving.
    let found = |name: &str| {
        devices::asio_driver(name)
            .ok_or_else(|| format!("'{name}' is not registered as an ASIO driver any more"))
    };
    let meta = found(driver_name)?;
    let copy_meta = copy.map(found).transpose()?;
    let two_names_one_driver = copy
        .zip(copy_meta.as_ref())
        .filter(|(_, found)| *found == &meta);
    if let Some((copy, _)) = two_names_one_driver {
        return Err(format!(
            "'{copy}' and '{driver_name}' are the same ASIO driver under two names, and one \
             driver is not loaded twice into one process"
        ));
    }

    // The process's ASIO slots, before anything is loaded, so a second attempt fails
    // here rather than after a driver has been initialized twice. Two drivers need
    // both slots, and a stream that cannot have both takes neither.
    let claim = Claim::take(copy.is_some()).ok_or_else(|| match copy {
        Some(copy) => format!(
            "This process already has an ASIO stream open, and a second driver needs a slot of \
             its own: '{driver_name}' and '{copy}' cannot be opened on top of it"
        ),
        None => format!(
            "This process already has an ASIO stream open, so '{driver_name}' cannot be opened \
             again"
        ),
    })?;
    let guard = devices::asio_stream_guard();

    // One failure channel for both drivers, because the host reopens the whole
    // stream whichever of them moved; every message says whose it is.
    let (sender, errors) = bounded(16);
    let width = wanted.outputs as usize;
    let first_asking = Asking {
        driver: driver_name.to_owned(),
        meta,
        slot: 0,
        inputs: wanted.inputs,
        outputs: wanted.outputs,
        first_output: 0,
        // A second group of channels on the live driver is only what was asked for
        // while the copy lives there too; a copy on a driver of its own is already
        // a second group, on the other interface.
        mirror: copy.is_none().then_some(wanted.second_output).flatten(),
        frames,
        exact_period: true,
        rate,
    };

    let mut feed = None;
    let copy_side = match (copy, copy_meta) {
        // The copy's driver opens first, for two reasons that point the same way:
        // the period it answers with is what the crossing is sized from, and a
        // second vendor's driver that refuses to load, initialize or start should
        // refuse before the live one has been touched, so that a settings file with
        // a typo in the second name leaves the machine as it found it.
        (Some(name), Some(meta)) => Some(Side::open(
            &Asking {
                driver: name.to_owned(),
                meta,
                slot: 1,
                // Output channels only: a copy has no inputs, and asking a driver
                // for an input is asking it to start a converter nobody reads.
                inputs: 0,
                outputs: wanted.outputs,
                // `copy_driver` only names a driver for a request that has a second
                // output group, so there is a start to read; the 1-based channel of
                // the settings becomes the driver's 0-based index.
                first_output: wanted.second_output.unwrap_or(1) - 1,
                mirror: None,
                frames,
                exact_period: false,
                rate,
            },
            &sender,
            |at| {
                // Room for the target the pull waits for, plus a couple of blocks
                // from either side, with a floor: a driver comfortable at 32-frame
                // blocks should still have a crossing that rides out a scheduler
                // that disagrees about when this process runs, rather than clicking
                // every time one does.
                let capacity = (at as usize * 8 + frames as usize * 4).max(4096);
                let (feeds, drains) = cross(width, capacity);
                feed = Some(feeds);
                // Both ends at the nominal rate, because both drivers were asked for
                // that one rate and a copy at another rate would be a copy of a
                // different signal. What the two clocks actually differ by is
                // handled by the correction, not by an interpolation table.
                let clock = f64::from(rate);
                Box::new(Copier {
                    pulls: drains
                        .into_iter()
                        .map(|drain| drain.pull(clock, clock, at as usize))
                        .collect(),
                    channels: width,
                }) as Box<dyn Pump>
            },
        )?),
        // Both halves of the pair come from the same `copy_driver`, so a request
        // either has a second driver and its metadata, or has neither.
        _ => None,
    };

    let name = copy.unwrap_or_default().to_owned();
    let notify = sender.clone();
    let first = Side::open(&first_asking, &sender, move |_| match feed {
        Some(feed) => Box::new(Repeater {
            inner: pump,
            feed,
            channels: width,
            notify,
            name,
            warned: false,
        }) as Box<dyn Pump>,
        None => Box::new(pump) as Box<dyn Pump>,
    })?;

    Ok(Transport {
        _copy: copy_side,
        first,
        // The number the user chose, which is the one the rings were sized from;
        // what the driver answered is within a hertz of it.
        frames,
        rate,
        inputs: wanted.inputs,
        outputs: wanted.outputs,
        errors,
        _guard: guard,
        _claim: claim,
    })
}

impl Side {
    /// Load one driver, agree a rate and a period with it, cut its buffers, and
    /// start it.
    ///
    /// `make_pump` is called with the period this driver answered with, after it has
    /// accepted the size and before its buffers are started. The live stream's pump
    /// is the host's own, handed the block it was already sized for; a copy's pump
    /// needs the period it will be drained at to size the crossing between two
    /// clocks, which makes that period the one number the second driver gets to
    /// choose for itself.
    ///
    /// The returned side is not a driver that has been politely asked: by the time
    /// this answers `Ok`, `bufferSwitch` is running on the driver's own thread and
    /// the state published below is what it reaches.
    fn open(
        asking: &Asking,
        errors: &Sender<Failure>,
        make_pump: impl FnOnce(u32) -> Box<dyn Pump>,
    ) -> Result<Self, String> {
        let driver_name = asking.driver.as_str();
        let rate = asking.rate;
        let wanted = asking.wanted();
        let meta = asking.meta.clone();

        // The thread this driver's apartment lives on. Its own thread is the point,
        // twice over now: the rack's UI thread is not where a driver's COM apartment
        // belongs, and two drivers that are not the same driver have no business
        // sharing one apartment. `Host::new` gives each one its own pump thread, and
        // the instance `create_driver` makes is initialized on that thread, which is
        // where its `CoInitializeEx` happens (`azo-0.6.1/src/host/worker.rs:29-46`,
        // `azo-0.6.1/src/driver.rs:156-170`).
        let host: Arc<Host> = Host::new();
        let driver = host
            .create_driver(&meta.clsid)
            .map_err(|error| format!("Cannot load '{driver_name}': {error}"))?;
        // No window handle, which is what this panel has always done with a driver:
        // the control panel stays unparented. What the driver gets is the handle
        // itself, where the SDK path handed it uninitialized memory
        // (`asio-sys-0.4.0/src/bindings/mod.rs:463-471`).
        if !driver.init(None) {
            return Err(complaint(
                &driver,
                &format!("'{driver_name}' refused to initialize"),
            ));
        }

        let channels = driver.channel_counts().map_err(|error| {
            refusal(
                complaint(
                    &driver,
                    &format!("'{driver_name}' would not say how many channels it has"),
                ),
                error,
            )
        })?;
        let enough = |have: c_long, want: u32| u32::try_from(have).unwrap_or(0) >= want;
        if !enough(channels.in_, wanted.inputs) {
            return Err(format!(
                "'{driver_name}' has {} input channels and this needs {}",
                channels.in_, wanted.inputs
            ));
        }
        // Where this block sits on this driver: its channel 1 for the live stream, the
        // channel the settings named for a copy. `checked_add` because a hand-edited
        // settings file can name a start so high that the width runs off the end of the
        // number type; that is a refusal, not an overflow.
        let want_out = asking
            .first_output
            .checked_add(wanted.outputs)
            .ok_or_else(|| {
                format!(
                    "The output channels asked for on '{driver_name}' start past the end of any \
                 driver: it has {}",
                    channels.out
                )
            })?;
        if !enough(channels.out, want_out) {
            return Err(format!(
                "'{driver_name}' has {} output channels and this needs one at channel {want_out}",
                channels.out
            ));
        }
        // The copy sits on the driver's own channels, so its span is measured the same
        // way as the first group's.
        if let Some(start) = wanted.second_output {
            let Some(end) = start.checked_add(wanted.outputs - 1) else {
                return Err(format!(
                    "The second output pair starts at output {start}, which is past the end of any \
                 driver: '{driver_name}' has {}",
                    channels.out
                ));
            };
            if !enough(channels.out, end) {
                return Err(format!(
                    "'{driver_name}' has {} output channels and the second output pair needs {}–{}",
                    channels.out, start, end
                ));
            }
        }

        // The size, asked of the driver and judged by the rule the panel's list is
        // built from. This is where a saved setting naming 24 stops being a surprise:
        // the panel's own check validates only the min..max span, so such a setting
        // reached the driver and was refused there, after a load.
        //
        // For the live stream that rule is absolute. A copy is asked for the same period
        // and takes the nearest this driver's ladder offers at or above it, because the
        // crossing in front of it turns one block into another and the host never
        // learns the difference: a second interface whose ladder starts at 512 frames
        // does not get to veto a 64-frame live path.
        let mut ladder = driver
            .buffer_size()
            .map(|size| Ladder::from_size(&size))
            .unwrap_or(Ladder::UNKNOWN);
        let mut frames = period(&ladder, driver_name, asking.frames, asking.exact_period)?;

        // The clock, asked of the driver before it is touched and read back after,
        // because a driver is allowed to answer a nearby rate and call it done. cpal
        // does the same two steps (`cpal-0.18.2/src/host/asio/stream.rs:1149-1165`).
        let asked = f64::from(rate);
        let mut applied = driver.get_sample_rate().map_err(|error| {
            refusal(
                complaint(&driver, "The driver would not say its rate"),
                error,
            )
        })?;
        // Half a hertz is "the same clock": a driver that answers 48000.000001 for
        // 48000 should not be walked through a hardware rate change to arrive where it
        // already was.
        if (applied - asked).abs() >= 0.5 {
            driver.can_sample_rate(asked).map_err(|error| {
                refusal(
                    complaint(&driver, &format!("'{driver_name}' cannot run at {rate} Hz")),
                    error,
                )
            })?;
            driver.set_sample_rate(asked).map_err(|error| {
                refusal(
                    complaint(
                        &driver,
                        &format!("'{driver_name}' refused to change to {rate} Hz"),
                    ),
                    error,
                )
            })?;
            applied = driver.get_sample_rate().map_err(|error| {
                refusal(
                    complaint(&driver, "The driver would not say its rate"),
                    error,
                )
            })?;
            if (applied - asked).abs() >= 1.0 {
                return Err(format!(
                    "'{driver_name}' stayed at {applied} Hz when asked for {rate} Hz"
                ));
            }
            // A driver may answer a different buffer ladder on a different clock, and
            // the buffers are about to be cut to this size.
            ladder = driver
                .buffer_size()
                .map(|size| Ladder::from_size(&size))
                .unwrap_or(Ladder::UNKNOWN);
            frames = period(&ladder, driver_name, asking.frames, asking.exact_period)?;
        }

        // One `channelInfo` per opened channel, both directions. The format is
        // per-channel in ASIO, and the binding this workspace used before `azo`
        // asked channel 0 and assumed the rest agree; here a driver that mixes
        // formats is refused by the channel that differs, in the words the driver
        // itself used to name it.
        let formats = |input: bool, first: u32, count: u32| -> Result<Vec<Pcm>, String> {
            (first..first + count)
            .map(|index| -> Result<Pcm, String> {
                let id = ChannelId {
                    input,
                    index: index as c_long,
                };
                let info: ChannelInfoResponse = driver.channel_info(id).map_err(|error| {
                    refusal(
                        complaint(
                            &driver,
                            &format!("The driver would not describe channel {}", index + 1),
                        ),
                        error,
                    )
                })?;
                pcm_of(info.sample_type).ok_or_else(|| {
                    format!(
                        "Channel {} of '{driver_name}' answers {}, which this host cannot read or write",
                        index + 1, info.sample_type
                    )
                })
            })
            .collect()
        };
        let input_formats = formats(true, 0, wanted.inputs)?;
        // The first group starts where this driver's block starts, which for the live
        // stream is its channel 1 and for a copy is the channel the settings named.
        let output_formats = formats(false, asking.first_output, wanted.outputs)?;
        // The copy's channels are asked the same question, and the driver answers it
        // per channel: its third output is not obliged to be the sample type its first
        // one is, and a copy is only faithful in the format the driver agreed to.
        let mirror_formats = match wanted.second_output {
            // `start` is beyond the first group, which is checked above, so this is
            // the driver's own zero-based index of that same channel.
            Some(start) => formats(false, start - 1, wanted.outputs)?,
            None => Vec::new(),
        };

        // Both directions, one call, inputs first, and the copy's channels last. One
        // call is not a style: `ASIOCreateBuffers` takes the whole buffer list and
        // `ASIODisposeBuffers` throws the whole list away, so a channel this host will
        // write has to be in the list it makes now (`azo-0.6.1/src/driver.rs:404-415`).
        // The buffers come back as two pointers per channel, in the order they were
        // asked for (`azo-0.6.1/src/driver.rs:92-103`).
        let mirror_ids = wanted.second_output.into_iter().flat_map(|start| {
            (start - 1..start - 1 + wanted.outputs).map(move |index| ChannelId {
                input: false,
                index: index as c_long,
            })
        });
        let ids = (0..wanted.inputs)
            .map(|index| ChannelId {
                input: true,
                index: index as c_long,
            })
            .chain(
                (asking.first_output..asking.first_output + wanted.outputs).map(|index| {
                    ChannelId {
                        input: false,
                        index: index as c_long,
                    }
                }),
            )
            .chain(mirror_ids);
        let asked_buffers = wanted.inputs + wanted.outputs + mirror_formats.len() as u32;
        let frames_long =
            c_long::try_from(frames).map_err(|_| "Buffer size is out of range".to_owned())?;
        // The one obligation `create_buffers` states: the callbacks must outlive the
        // buffers. Each side gets its own set, and a set is a `static` entry, so it
        // outlives everything (`azo-0.6.1/src/driver.rs:92-94`).
        let halves =
            unsafe { driver.create_buffers(ids, frames_long, &raw const CALLBACKS[asking.slot]) }
                .map_err(|error| {
                    refusal(
                        complaint(
                            &driver,
                            &format!("'{driver_name}' refused {frames}-frame buffers"),
                        ),
                        error,
                    )
                })?
                .collect::<Vec<_>>();
        if halves.len() != asked_buffers as usize {
            // A driver that answered fewer buffers than channels would have the
            // indexing below reach past the answer, so it is an error and not a panic.
            return Err(format!(
                "'{driver_name}' answered {} buffers for {} channels",
                halves.len(),
                asked_buffers
            ));
        }
        let (input_halves, rest) = halves.split_at(wanted.inputs as usize);
        let (output_halves, mirror_halves) = rest.split_at(wanted.outputs as usize);
        let inputs: Vec<Channel> = input_formats
            .iter()
            .zip(input_halves)
            .map(|(pcm, halves)| Channel {
                halves: *halves,
                pcm: *pcm,
            })
            .collect();
        let outputs: Vec<Channel> = output_formats
            .iter()
            .zip(output_halves)
            .map(|(pcm, halves)| Channel {
                halves: *halves,
                pcm: *pcm,
            })
            .collect();
        let mirror: Vec<Channel> = mirror_formats
            .iter()
            .zip(mirror_halves)
            .map(|(pcm, halves)| Channel {
                halves: *halves,
                pcm: *pcm,
            })
            .collect();

        let frames_usize = frames as usize;
        let state = Box::new(State {
            name: driver_name.to_owned(),
            inputs,
            outputs,
            mirror,
            frames: frames_usize,
            capture: UnsafeCell::new(vec![0.0; frames_usize * wanted.inputs as usize]),
            playback: UnsafeCell::new(vec![0.0; frames_usize * wanted.outputs as usize]),
            pump: UnsafeCell::new(make_pump(frames)),
            errors: errors.clone(),
            phase: AtomicU8::new(STARTING),
            inside: AtomicUsize::new(0),
            last_half: AtomicUsize::new(usize::MAX),
            rate: applied,
            overloads: AtomicU64::new(0),
        });

        // Silence, before the driver starts and before anyone can hear it. Zero bytes
        // is silence in every format this module writes: the integral ones are
        // two's-complement and a float zero is all-bits-zero.
        for channel in state.outputs.iter().chain(&state.mirror) {
            for half in 0..HALVES {
                if let Some(plane) = channel.plane(half, frames_usize) {
                    plane.fill(0);
                }
            }
        }

        let published = Box::into_raw(state);
        // The claim [`open`] took makes this slot ours alone, so a plain store publishes
        // it. No callback can run before `start`, which is what lets the first one
        // find its state and nothing find it before it is ready.
        STATES[asking.slot].store(published, Ordering::Release);
        if let Err(error) = driver.start() {
            // Back the state away before freeing it. Nothing was started, so nothing
            // can be inside it; the buffers go back the same way they would have on a
            // close.
            unsafe {
                (*published).phase.store(DETACHED, Ordering::SeqCst);
                STATES[asking.slot].store(null_mut(), Ordering::Release);
                drop(Box::from_raw(published));
            }
            let problem = refusal(
                complaint(&driver, &format!("'{driver_name}' would not start")),
                error,
            );
            if let Err(error) = driver.dispose_buffers() {
                return Err(format!(
                    "{problem}, and it kept the buffers it had made ({error})"
                ));
            }
            return Err(problem);
        }
        unsafe {
            (*published)
                .phase
                .compare_exchange(STARTING, RUNNING, Ordering::AcqRel, Ordering::Relaxed)
                .ok();
        }
        // Built last, because it takes the driver with it: after this there is no
        // handle left to ask a question of, and `start` was the last thing needed.
        let name = driver_name.to_owned();
        let close = Box::new(move || {
            let mut problems = Vec::new();
            // `stop` first: after it returns, a driver that keeps its side of the
            // standard calls no more callbacks. Both sentences name the driver, because
            // with two of them open "it would not stop" is not an answer.
            if let Err(error) = driver.stop() {
                problems.push(refusal(
                    complaint(&driver, &format!("'{name}' would not stop")),
                    error,
                ));
            }
            if let Err(error) = driver.dispose_buffers() {
                problems.push(refusal(
                    complaint(
                        &driver,
                        &format!("'{name}' would not give its buffers back"),
                    ),
                    error,
                ));
            }
            problems
        });
        Ok(Side {
            slot: asking.slot,
            state: published,
            close: Some(close),
            notify: errors.clone(),
        })
    }
}

/// The period to cut this driver's buffers at, or the sentence explaining why there
/// is none.
///
/// A live stream's period is the one the host was built around and nothing else will
/// do. A copy's is a preference: whatever this driver's ladder offers at or above it
/// carries the same seconds of sound, and the crossing in front of the copy is what
/// turns one block into the other.
fn period(ladder: &Ladder, driver: &str, frames: u32, exact: bool) -> Result<u32, String> {
    if ladder.accepts(frames) {
        return Ok(frames);
    }
    if exact {
        return Err(refused(driver, frames, *ladder));
    }
    ladder
        .nearest_at_least(frames)
        .ok_or_else(|| refused(driver, frames, *ladder))
}

/// The sentence for a size this driver does not take, carrying the sizes it does.
fn refused(driver: &str, frames: u32, ladder: Ladder) -> String {
    let options = devices::asio_buffer_options(Some(ladder));
    if options.is_empty() {
        return format!(
            "'{driver}' takes no buffer size at all, so it cannot be opened. Its own answer \
             was a minimum of {} and a maximum of {}.",
            ladder.min, ladder.max
        );
    }
    format!(
        "'{driver}' does not take a buffer of {frames} frames. It takes: {}.",
        options
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null;
    use std::sync::Mutex;

    /// A pump that counts. Half of what is asserted below is that a host hears
    /// nothing when there is nothing to hear, and that is only checkable if
    /// something is keeping count.
    struct Counting(Arc<AtomicUsize>);

    impl Pump for Counting {
        fn input(&mut self, _interleaved: &[f32]) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn output(&mut self, _interleaved: &mut [f32]) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn counting() -> (Counting, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (Counting(calls.clone()), calls)
    }

    /// A host that also remembers what it was handed, for the one test that is
    /// about the bytes rather than the answers.
    struct Tape {
        blocks: Arc<AtomicUsize>,
        heard: Arc<Mutex<Vec<f32>>>,
    }

    impl Pump for Tape {
        fn input(&mut self, interleaved: &[f32]) {
            self.blocks.fetch_add(1, Ordering::Relaxed);
            self.heard.lock().unwrap().extend_from_slice(interleaved);
        }

        fn output(&mut self, interleaved: &mut [f32]) {
            // A ramp, so a byte order or a stride that is wrong shows up as
            // different bytes rather than as a number that happens to match.
            for (index, frame) in interleaved.iter_mut().enumerate() {
                *frame = (index + 1) as f32 / 8.0;
            }
        }
    }

    /// The shortest request that is a real ASIO stream, so each test names only
    /// the one thing wrong with it.
    fn wanted<'a>(input: &'a str, output: &'a str) -> Wanted<'a> {
        Wanted {
            input,
            output,
            input_rate: 48_000,
            output_rate: 48_000,
            input_frames: 64,
            output_frames: 64,
            inputs: 1,
            outputs: 2,
            second_output: None,
            second_device: None,
        }
    }

    /// The four requests that are not ASIO streams, in the words a user can act
    /// on. These checks run before the driver is loaded, so this test needs no
    /// driver, no registry and no machine with ASIO on it.
    #[test]
    fn a_request_that_is_not_a_stream_is_refused_in_words() {
        // `.err()` rather than `.unwrap_err()`, which would need `Transport:
        // Debug` — a derive it cannot have honestly, since half of it is a driver
        // handle and a raw state the driver is entitled to be inside.
        let (pump, calls) = counting();
        let two_drivers = open(wanted("Left", "Right"), pump)
            .err()
            .expect("two drivers in one request");
        assert!(
            two_drivers.contains("Left")
                && two_drivers.contains("Right")
                && two_drivers.contains("one driver"),
            "{two_drivers}"
        );

        let (pump, _) = counting();
        let mut two_clocks = wanted("Driver", "Driver");
        two_clocks.output_rate = 44_100;
        let two_clocks = open(two_clocks, pump)
            .err()
            .expect("two clocks in one request");
        assert!(
            two_clocks.contains("one clock") && two_clocks.contains("48000 Hz in and 44100 Hz out"),
            "{two_clocks}"
        );

        let (pump, _) = counting();
        let mut two_sizes = wanted("Driver", "Driver");
        two_sizes.output_frames = 128;
        let two_sizes = open(two_sizes, pump)
            .err()
            .expect("two buffer sizes in one request");
        assert!(
            two_sizes.contains("one buffer") && two_sizes.contains("64 frames in and 128 out"),
            "{two_sizes}"
        );

        let (pump, _) = counting();
        let nothing = Wanted {
            inputs: 0,
            outputs: 0,
            ..wanted("Driver", "Driver")
        };
        let nothing = open(nothing, pump)
            .err()
            .expect("a stream with no channels");
        assert!(nothing.contains("at least one"), "{nothing}");

        // A copy that starts inside the group it copies is not a second output
        // pair; it is the same signal written to one channel twice, and the
        // driver would be the one to notice.
        let (pump, _) = counting();
        let mut inside = wanted("Driver", "Driver");
        inside.second_output = Some(2);
        let inside = open(inside, pump)
            .err()
            .expect("a second output pair inside the first one");
        assert!(
            inside.contains("starts at output 2") && inside.contains("pick 3 or later"),
            "{inside}"
        );

        let (pump, _) = counting();
        let mut no_copy = wanted("Driver", "Driver");
        no_copy.outputs = 0;
        no_copy.second_output = Some(3);
        let no_copy = open(no_copy, pump)
            .err()
            .expect("a second output pair with nothing to copy");
        assert!(no_copy.contains("no output channels to copy"), "{no_copy}");

        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "a refused request still reached the host"
        );
    }

    /// The driver's own message call, in the second slot — the one a copy's driver
    /// is given — with the two arguments this host does not use. The numbers in the
    /// assertions are Steinberg's, which is what the module's comment about the
    /// numbering claims; a renumbering in a future `azo` shows up here as a red test
    /// rather than as a driver whose reset requests are silently declined.
    fn ask(selector: MessageSelector, value: c_long) -> c_long {
        unsafe { asio_message::<1>(selector, value, null(), null()) }
    }

    /// The same call in the other slot, which nothing is published in during these
    /// tests: what the second driver is told cannot come from an empty first one.
    fn ask_empty(selector: MessageSelector, value: c_long) -> c_long {
        unsafe { asio_message::<0>(selector, value, null(), null()) }
    }

    /// One block each way, in the driver's own bytes, and the two rules about
    /// which half a callback is about. The buffers are two `Vec`s standing in for
    /// what `createBuffers` would hand back; everything else — the conversion,
    /// the deduplication, the interleaving — is the code a real driver runs.
    #[test]
    fn one_half_is_one_block_however_often_the_driver_names_it() {
        // Two frames: four bytes of 16-bit integers in, eight bytes of 32-bit
        // floats out, per half.
        let mut input = [vec![0u8; 4], vec![0u8; 4]];
        let mut output = [vec![0u8; 8], vec![0u8; 8]];
        // 0 and +32767: the first is the easiest number and the second is the
        // top of the scale, where a sign bit or a full-scale divisor that is
        // wrong stops being plausible.
        input[0].copy_from_slice(&[0x00, 0x00, 0xff, 0x7f]);
        let blocks = Arc::new(AtomicUsize::new(0));
        let heard = Arc::new(Mutex::new(Vec::new()));
        let (sender, _errors) = bounded(16);
        let state = State {
            name: "Test driver".into(),
            inputs: vec![Channel {
                halves: [input[0].as_mut_ptr().cast(), input[1].as_mut_ptr().cast()],
                pcm: Pcm::Int16 { big_endian: false },
            }],
            outputs: vec![Channel {
                halves: [output[0].as_mut_ptr().cast(), output[1].as_mut_ptr().cast()],
                pcm: Pcm::Float32 { big_endian: false },
            }],
            mirror: Vec::new(),
            frames: 2,
            capture: UnsafeCell::new(vec![0.0; 2]),
            playback: UnsafeCell::new(vec![0.0; 2]),
            pump: UnsafeCell::new(Box::new(Tape {
                blocks: blocks.clone(),
                heard: heard.clone(),
            }) as Box<dyn Pump>),
            errors: sender,
            phase: AtomicU8::new(RUNNING),
            inside: AtomicUsize::new(0),
            last_half: AtomicUsize::new(usize::MAX),
            rate: 48_000.0,
            overloads: AtomicU64::new(0),
        };

        state.exchange(0, false);
        assert_eq!(
            &*heard.lock().unwrap(),
            &[0.0, 1.0],
            "the driver's 16-bit frames did not arrive as floats"
        );
        let played: Vec<u8> = [0.125f32.to_le_bytes(), 0.25f32.to_le_bytes()].concat();
        assert_eq!(
            output[0].as_slice(),
            played.as_slice(),
            "the host's ramp did not reach the driver's bytes in the order the host wrote it"
        );

        // The same half a second time is a driver doing what Focusrite USB ASIO
        // and ReaRoute do (`cpal-0.18.2/src/host/asio/stream.rs:227-230`), and
        // one block is all it is.
        state.exchange(0, false);
        assert_eq!(
            blocks.load(Ordering::Relaxed),
            1,
            "the same half twice became two blocks"
        );
        state.exchange(1, false);
        assert_eq!(blocks.load(Ordering::Relaxed), 2, "the other half was lost");
        // With `directProcess` there is no other half to wait for, so the same
        // index twice is two blocks. Dropping the flag — which is what the SDK
        // path does (`asio-sys-0.4.0/src/bindings/mod.rs:1226`) — turns these two
        // into one, which is a block of silence every other period.
        state.exchange(1, true);
        state.exchange(1, true);
        assert_eq!(
            blocks.load(Ordering::Relaxed),
            4,
            "`directProcess` blocks were deduplicated like double-buffered ones"
        );
    }

    /// One block, two groups: the copy is the same frames the pump filled, in the
    /// order it filled them, laid down again in the copy's own format. The copy is
    /// `Float64` where the group above it is `Float32`, so bytes that came out
    /// equal to their neighbours would mean a shortcut this module does not have.
    #[test]
    fn a_second_output_group_gets_the_same_block_in_its_own_format() {
        let mut group_left = [vec![0u8; 8], vec![0u8; 8]];
        let mut group_right = [vec![0u8; 8], vec![0u8; 8]];
        let mut copy_left = [vec![0u8; 16], vec![0u8; 16]];
        let mut copy_right = [vec![0u8; 16], vec![0u8; 16]];
        let channel = |buffer: &mut [Vec<u8>; 2], pcm| Channel {
            halves: [buffer[0].as_mut_ptr().cast(), buffer[1].as_mut_ptr().cast()],
            pcm,
        };
        let (sender, _errors) = bounded(16);
        let state = State {
            name: "Test driver".into(),
            inputs: Vec::new(),
            outputs: vec![
                channel(&mut group_left, Pcm::Float32 { big_endian: false }),
                channel(&mut group_right, Pcm::Float32 { big_endian: false }),
            ],
            mirror: vec![
                channel(&mut copy_left, Pcm::Float64 { big_endian: false }),
                channel(&mut copy_right, Pcm::Float64 { big_endian: false }),
            ],
            frames: 2,
            capture: UnsafeCell::new(Vec::new()),
            playback: UnsafeCell::new(vec![0.0; 4]),
            pump: UnsafeCell::new(Box::new(Tape {
                blocks: Arc::new(AtomicUsize::new(0)),
                heard: Arc::new(Mutex::new(Vec::new())),
            }) as Box<dyn Pump>),
            errors: sender,
            phase: AtomicU8::new(RUNNING),
            inside: AtomicUsize::new(0),
            last_half: AtomicUsize::new(usize::MAX),
            rate: 48_000.0,
            overloads: AtomicU64::new(0),
        };

        // `Tape` writes `(index + 1) / 8` across the block, so two frames of two
        // channels is 0.125 and 0.25 in the first frame, 0.375 and 0.5 in the next.
        state.exchange(0, false);
        let float = |left: f32, right: f32| -> Vec<u8> {
            [left.to_le_bytes(), right.to_le_bytes()].concat()
        };
        let double = |left: f32, right: f32| -> Vec<u8> {
            [(left as f64).to_le_bytes(), (right as f64).to_le_bytes()].concat()
        };
        assert_eq!(
            group_left[0],
            float(0.125, 0.375),
            "the first channel of the first group did not get its frames"
        );
        assert_eq!(
            group_right[0],
            float(0.25, 0.5),
            "the second channel of the first group did not get its frames"
        );
        assert_eq!(
            copy_left[0],
            double(0.125, 0.375),
            "the copy's first channel is not the signal the first group's first channel got"
        );
        assert_eq!(
            copy_right[0],
            double(0.25, 0.5),
            "the copy's second channel is not the signal the first group's second channel got"
        );
        assert_eq!(
            copy_left[1],
            vec![0u8; 16],
            "one callback wrote both halves of the copy"
        );
    }

    /// The process holds two slots and one state apiece, and here are the answers a
    /// driver is given while it holds them — all of it in one test, in order. `cargo
    /// test` runs the tests of a binary in threads of one process, and a second test
    /// claiming a slot the first is standing in would fail for a reason that has
    /// nothing to do with the code under test.
    #[test]
    fn two_slots_one_state_each_and_the_answers_a_driver_gets() {
        // A refusal costs nothing. The shape checks come first, so a settings
        // file naming two drivers leaves the process free to open the one it
        // should have named.
        let (pump, _) = counting();
        let refused = open(wanted("Left", "Right"), pump).err();
        assert!(refused.is_some(), "two drivers were accepted");

        // A stream with a copy needs both slots, and takes them together: a stream
        // that cannot have both takes neither, which is what keeps a half-open pair
        // from being a thing a later open has to work around.
        let claim = Claim::take(true).expect("a stream with a copy takes two slots");
        assert!(
            Claim::take(false).is_none(),
            "a second stream took a slot that was spoken for"
        );
        assert!(
            Claim::take(true).is_none(),
            "a second stream with a copy took the pair again"
        );
        // The other half of that promise is the rollback, which the ordinary path
        // cannot reach because a claim always starts at the first slot: one that
        // takes a slot and loses the next has to give the first back, or a failed
        // open leaves a slot nobody is standing in, unopenable until this process
        // ends.
        drop(claim);
        CLAIMED[1].store(true, Ordering::SeqCst);
        assert!(
            Claim::take(true).is_none(),
            "a copy was claimed onto a held slot"
        );
        assert!(
            !CLAIMED[0].load(Ordering::SeqCst),
            "a failed claim kept the slot it had already taken"
        );
        CLAIMED[1].store(false, Ordering::SeqCst);
        let claim = Claim::take(true).expect("both slots came back");

        let (sender, errors) = bounded(16);
        let (pump, calls) = counting();
        // A stream with no channels and no buffers, published in the *second* slot:
        // everything below is then answered through the callbacks a copy's driver is
        // given, before any driver exists.
        let state = Box::into_raw(Box::new(State {
            name: "Second driver".into(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            mirror: Vec::new(),
            frames: 0,
            capture: UnsafeCell::new(Vec::new()),
            playback: UnsafeCell::new(Vec::new()),
            pump: UnsafeCell::new(Box::new(pump) as Box<dyn Pump>),
            errors: sender,
            phase: AtomicU8::new(RUNNING),
            inside: AtomicUsize::new(0),
            last_half: AtomicUsize::new(usize::MAX),
            rate: 48_000.0,
            overloads: AtomicU64::new(0),
        }));
        STATES[1].store(state, Ordering::Release);

        // An empty first slot answers no and says nothing, so a driver that was
        // never opened here cannot be the one the panel is hearing about.
        assert_eq!(
            ask_empty(MessageSelector::OVERLOAD, 0),
            Bool::FALSE.0,
            "an empty slot claimed a callback"
        );
        assert!(
            errors.try_recv().is_err(),
            "an empty slot reported an overload"
        );
        unsafe { buffer_switch::<0>(0, Bool::FALSE) };

        // A callback for a stream with nothing in it says nothing to the host.
        unsafe { buffer_switch::<1>(0, Bool::FALSE) };
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "a block of nothing reached the host"
        );

        // Only the selectors answered above are claimed, and the driver is told
        // so for each of the ones that are not.
        assert_eq!(
            ask(MessageSelector::SELECTOR_SUPPORTED, WANT_SIZE),
            Bool::TRUE.0
        );
        assert_eq!(
            ask(
                MessageSelector::SELECTOR_SUPPORTED,
                MessageSelector::OVERLOAD.0
            ),
            Bool::FALSE.0,
            "a selector with no arm must not be claimed"
        );
        assert_eq!(
            ask(
                MessageSelector::SELECTOR_SUPPORTED,
                MessageSelector::LATENCIES_CHANGED.0
            ),
            Bool::FALSE.0
        );
        assert_eq!(ask(MessageSelector::ENGINE_VERSION, 0), 2);
        assert_eq!(ask(MessageSelector::LATENCIES_CHANGED, 0), Bool::FALSE.0);

        // A driver that resized underneath this stream is told yes — it is
        // happening either way — and the host is told the stream is over.
        assert_eq!(ask(MessageSelector::BUFFER_SIZE_CHANGE, 128), Bool::TRUE.0);
        let resized = errors.try_recv().expect("a buffer change reaches the host");
        assert!(resized.invalidated, "a resized stream was treated as live");
        assert!(resized.message.contains("128"), "{}", resized.message);
        // And it says which of the two drivers said it, because with both open that
        // is the difference between a settings change and an unplugged interface.
        assert!(
            resized.message.contains("'Second driver'"),
            "{}",
            resized.message
        );

        // An overload is one dropped block, and not a reason to reopen.
        assert_eq!(ask(MessageSelector::OVERLOAD, 0), Bool::TRUE.0);
        let dropout = errors.try_recv().expect("an overload reaches the host");
        assert!(
            !dropout.invalidated,
            "one dropped block asked the host to reopen"
        );
        assert_eq!(unsafe { &*state }.overloads.load(Ordering::Relaxed), 1);

        // Half a hertz is the same clock read coarsely; 44.1 kHz is another clock.
        unsafe { sample_rate_did_change::<1>(48_000.5) };
        assert!(
            errors.try_recv().is_err(),
            "the same clock, read coarsely, was news"
        );
        unsafe { sample_rate_did_change::<1>(44_100.0) };
        assert!(
            errors
                .try_recv()
                .expect("a moved clock reaches the host")
                .invalidated
        );

        // A reset while running is the driver's way of saying reopen.
        assert_eq!(ask(MessageSelector::RESET_REQUEST, 0), Bool::TRUE.0);
        assert!(
            errors.try_recv().is_ok(),
            "a reset was answered without notice"
        );
        // The same request while starting is a driver finding its clock on the way
        // up: answered, and not reported.
        unsafe { (*state).phase.store(STARTING, Ordering::Relaxed) };
        assert_eq!(ask(MessageSelector::RESYNC_REQUEST, 0), Bool::TRUE.0);
        assert!(
            errors.try_recv().is_err(),
            "a driver configuring itself during start was reported as a failure"
        );

        // Taken away the way [`Side`]'s `Drop` takes it away, and the slot with it.
        unsafe {
            (*state).phase.store(DETACHED, Ordering::SeqCst);
            STATES[1].store(null_mut(), Ordering::Release);
            drop(Box::from_raw(state));
        }
        drop(claim);
        assert!(
            Claim::take(true).is_some(),
            "the slots did not come back with the stream"
        );
    }

    /// The eighteen codes the header defines, in `azo`'s spelling, plus one it
    /// does not. The numbers are pinned beside the names for the reason above:
    /// this host converts by name and talks to the driver in numbers, so a
    /// renumbering has to be caught somewhere.
    #[test]
    fn every_named_format_is_readable_and_anything_else_is_refused() {
        let named = [
            SampleType::PCM_I16_MSB,
            SampleType::PCM_I24_MSB,
            SampleType::PCM_I32_MSB,
            SampleType::PCM_F32_MSB,
            SampleType::PCM_F64_MSB,
            SampleType::PCM_I32_MSB_16,
            SampleType::PCM_I32_MSB_18,
            SampleType::PCM_I32_MSB_20,
            SampleType::PCM_I32_MSB_24,
            SampleType::PCM_I16_LSB,
            SampleType::PCM_I24_LSB,
            SampleType::PCM_I32_LSB,
            SampleType::PCM_F32_LSB,
            SampleType::PCM_F64_LSB,
            SampleType::PCM_I32_LSB_16,
            SampleType::PCM_I32_LSB_18,
            SampleType::PCM_I32_LSB_20,
            SampleType::PCM_I32_LSB_24,
        ];
        assert_eq!(named.len(), 18, "the header's list has changed shape");
        for code in named {
            assert!(
                pcm_of(code).is_some(),
                "{code} is a format the header names and this host cannot read"
            );
        }
        // What the drivers on this machine answer, by number as well as by name:
        // `azo-sys-0.4.1/src/lib.rs:100-102` beside the header's own
        // `asio-sys-0.4.0/src/bindings/mod.rs:172-174`.
        assert_eq!(
            pcm_of(SampleType::PCM_I32_LSB),
            Some(Pcm::Int32 { big_endian: false })
        );
        assert_eq!(SampleType::PCM_I32_LSB.0, 18);
        assert_eq!(SampleType::PCM_I16_LSB.0, 16);
        assert_eq!(SampleType::PCM_I24_LSB.0, 17);
        // DSD is in the same header, three codes above the last PCM code
        // (`azo-sys-0.4.1/src/lib.rs:111-113`), and a driver that answers it is
        // refused by name rather than read as integers.
        for dsd in [
            SampleType::DSD_I8_LSB_1,
            SampleType::DSD_I8_MSB_1,
            SampleType::DSD_I8_NER_8,
        ] {
            assert!(
                pcm_of(dsd).is_none(),
                "{dsd} is not a format this host reads"
            );
        }
        assert!(
            pcm_of(SampleType(99)).is_none(),
            "a code nobody has defined was guessed at"
        );
    }

    /// The one question that decides whether a second driver is loaded at all. A
    /// copy naming the driver it is copied from is one driver with two groups: a
    /// different question, with different rules and no second clock, and the name
    /// is the whole of the difference as far as a settings file is concerned.
    #[test]
    fn a_second_driver_is_only_a_second_driver_when_it_is_another_one() {
        let mut request = wanted("First driver", "First driver");
        assert_eq!(request.copy_driver(), None, "no copy was asked for");
        request.second_output = Some(3);
        assert_eq!(
            request.copy_driver(),
            None,
            "a start channel with no driver named rides the first driver"
        );
        request.second_device = Some("First driver");
        assert_eq!(
            request.copy_driver(),
            None,
            "naming the driver the copy is copied from names nothing"
        );
        request.second_device = Some("Second driver");
        assert_eq!(request.copy_driver(), Some("Second driver"));
        request.second_output = None;
        assert_eq!(
            request.copy_driver(),
            None,
            "a driver named with no copy to carry is not a driver to open"
        );
    }

    /// A live stream's period is the one the host was built around, so a driver
    /// that does not take it is met with a refusal. A copy's period is a
    /// preference: the same seconds of sound at whatever size the driver it rides
    /// on takes, with the crossing in front of it doing the converting.
    #[test]
    fn a_copy_takes_the_size_the_second_driver_takes_and_the_live_stream_its_own() {
        use crate::devices::Steps;
        let chunky = Ladder {
            min: 512,
            max: 2_048,
            steps: Steps::Doubling,
        };
        assert_eq!(period(&chunky, "Second driver", 64, false), Ok(512));
        assert_eq!(period(&chunky, "Second driver", 1_024, false), Ok(1_024));
        let for_the_live_stream = period(&chunky, "First driver", 64, true).unwrap_err();
        assert!(
            for_the_live_stream.contains("'First driver'")
                && for_the_live_stream.contains("does not take a buffer of 64"),
            "{for_the_live_stream}"
        );
        // A driver that would not say has nothing to negotiate with in either
        // direction: a period guessed for it would be this host inventing an answer
        // the driver refused to give.
        let mute = period(&Ladder::UNKNOWN, "Quiet driver", 64, false).unwrap_err();
        assert!(mute.contains("no buffer size"), "{mute}");
    }

    /// The crossing between two clocks, read at the rate it was written at: frames
    /// come back in the order they left, a pair comes back as a pair, and the one
    /// frame the resampler keeps as lookahead is the whole of the delay. None of it
    /// needs a driver, which is the point of testing it — this is the arithmetic a
    /// second driver's callback runs sixty times a second, on a machine with no
    /// second driver on it.
    #[test]
    fn a_copy_arrives_as_it_left_and_a_pair_arrives_as_a_pair() {
        let (mut feeds, drains) = cross(2, 64);
        let mut copier = Copier {
            pulls: drains
                .into_iter()
                .map(|drain| drain.pull(48_000.0, 48_000.0, 4))
                .collect(),
            channels: 2,
        };
        let block = |from: f32| -> Vec<f32> {
            (0..4)
                .flat_map(|frame| {
                    let value = from + frame as f32;
                    [value, -value]
                })
                .collect()
        };
        let mut lost = false;
        push_block(&mut feeds, &block(1.0), 2, &mut lost);
        push_block(&mut feeds, &block(10.0), 2, &mut lost);
        assert!(!lost, "a crossing with room in it reported a drop");

        // A `Monitor` starts once it holds the two blocks the crossing is sized for,
        // and then answers one frame behind, so the head of the stream is a frame of
        // silence and every frame after it is the frame before it moved on.
        let mut first = vec![0.25; 8];
        copier.output(&mut first);
        assert_eq!(
            first,
            vec![0.0, 0.0, 1.0, -1.0, 2.0, -2.0, 3.0, -3.0],
            "the first block did not come back as it went in"
        );
        let mut second = vec![0.25; 8];
        copier.output(&mut second);
        assert_eq!(
            second,
            vec![4.0, -4.0, 10.0, -10.0, 11.0, -11.0, 12.0, -12.0],
            "the second block did not follow the first"
        );
        for frame in first.chunks(2).chain(second.chunks(2)) {
            assert_eq!(frame[0], -frame[1], "a pair was smeared apart");
        }
    }

    /// Three channels are one pair and one on its own, and the lone channel is the
    /// part of this shape that goes wrong interestingly: a pair's second channel
    /// written where there is no pair is the NEXT frame's first channel, and a copy
    /// onto a third and fourth output would then carry the second output's left ear
    /// into the third.
    #[test]
    fn an_odd_last_channel_crosses_alone_and_lands_in_its_own_slot() {
        let (mut feeds, drains) = cross(3, 64);
        assert_eq!(feeds.len(), 2, "three channels are not two pairs");
        assert_eq!(drains.len(), 2);
        let mut copier = Copier {
            pulls: drains
                .into_iter()
                .map(|drain| drain.pull(48_000.0, 48_000.0, 2))
                .collect(),
            channels: 3,
        };
        let mut lost = false;
        for _ in 0..2 {
            push_block(&mut feeds, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, &mut lost);
        }
        assert!(!lost, "a crossing with room in it reported a drop");

        let mut out = vec![0.25; 6];
        copier.output(&mut out);
        assert_eq!(out, vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
        copier.output(&mut out);
        assert_eq!(out, vec![4.0, 5.0, 6.0, 1.0, 2.0, 3.0]);
    }

    /// A driver that starts before the other one has handed anything over hears
    /// silence rather than whatever a ring happened to be filled with: two drivers
    /// are not started by the same instruction, so the gap between them is ordinary
    /// and not a fault, whichever of the two resampling paths is in use.
    #[test]
    fn nothing_across_the_crossing_is_silence() {
        let (_feeds, drains) = cross(3, 16);
        let mut copier = Copier {
            pulls: drains
                .into_iter()
                .map(|drain| drain.pull(48_000.0, 44_100.0, 4))
                .collect(),
            channels: 3,
        };
        let mut block = vec![0.25; 12];
        copier.output(&mut block);
        assert!(
            block.iter().all(|sample| *sample == 0.0),
            "an empty crossing answered {block:?}"
        );
    }

    /// The live path never waits for the copy. A second driver that has stopped
    /// taking frames — unplugged, stopped from its own control panel, a process that
    /// lost its CPU for a hundred milliseconds — is not allowed to hold up a rack,
    /// so the newest frame is dropped where a queue would have waited for room.
    #[test]
    fn a_copy_that_stops_reading_drops_the_newest_frame_and_returns_anyway() {
        let (mut feeds, drains) = cross(1, 4);
        let mut lost = false;
        push_block(&mut feeds, &[1.0, 2.0, 3.0], 1, &mut lost);
        assert!(
            !lost,
            "three frames went into room for four and were reported lost"
        );
        push_block(&mut feeds, &[4.0, 5.0, 6.0], 1, &mut lost);
        assert!(lost, "a crossing full to the top reported nothing");
        // What survived is what was written first: the frames nobody had taken yet
        // are the newest sound, and losing the newest is what a live path does when
        // it has decided not to wait.
        let mut receiver = match drains.into_iter().next().unwrap() {
            Drain::Mono(receiver) => receiver,
            Drain::Stereo(_) => panic!("one channel is not a pair"),
        };
        let mut kept = Vec::new();
        while let Ok(sample) = receiver.pop() {
            kept.push(sample);
        }
        assert_eq!(kept, vec![1.0, 2.0, 3.0, 4.0]);
    }

    /// One sentence per stream, not one per block: a driver that is behind is behind
    /// every block, and a panel told the same thing sixty times a second reads as
    /// nothing. The host is still asked for its block each time, and a copy having
    /// gone quiet does not invalidate the live stream.
    #[test]
    fn a_late_copy_is_reported_once_and_leaves_the_live_path_alone() {
        let (feeds, _drains) = cross(1, 1);
        let (notify, problems) = bounded(16);
        let (inner, calls) = counting();
        let mut repeater = Repeater {
            inner,
            feed: feeds,
            channels: 1,
            notify,
            name: "Second driver".into(),
            warned: false,
        };
        let mut block = [1.0f32, 2.0, 3.0];
        repeater.output(&mut block);
        repeater.output(&mut block);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            2,
            "the host was skipped to save a copy"
        );
        let once = problems
            .try_recv()
            .expect("a copy that lost frames went unreported");
        assert!(once.message.contains("'Second driver'"), "{}", once.message);
        assert!(!once.invalidated, "a late copy closed the live stream");
        assert!(
            problems.try_recv().is_err(),
            "the same complaint was sent twice"
        );
    }
}
