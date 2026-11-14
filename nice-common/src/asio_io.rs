//! The Windows transport: one ASIO driver, driven through `azo`.
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
//! is reachable here: one entry point, one driver, one `createBuffers` for both
//! directions, and the combinations ASIO cannot express are refused before the
//! driver is asked anything.
//!
//! What the standard asks of a host, and where this file does it:
//!
//! * One driver per process: [`open`] claims a process-global state and holds it
//!   until the returned [`Transport`] is dropped. A second call says so in words
//!   instead of loading a second instance.
//! * Both directions in one `createBuffers`, at a size the driver itself said it
//!   takes ([`crate::devices::Ladder::accepts`]), with the rate asked through
//!   `canSampleRate` before `setSampleRate` and read back afterwards.
//! * Callbacks that allocate nothing, lock nothing, never panic, and never call
//!   the driver back. The last one is what `azo`'s `Host` makes fatal: every
//!   driver method is marshalled to its pump thread with a blocking `SendMessageW`,
//!   so a driver call from a callback would be a deadlock with the driver standing
//!   inside our own callback. It is also why [`Transport::drop`] does its stopping
//!   from the host thread, where blocking is allowed.
//! * Teardown in the order the SDK uses — `stop`, then `disposeBuffers`
//!   (`asio-sys-0.4.0/src/bindings/mod.rs:987-999`) — with the errors reported
//!   instead of swallowed (`…:1040` ends in `.ok()`).
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
use azo::Host;
use azo::driver::Driver as _;
use azo::dto::{ChannelId, ChannelInfoResponse};
use azo::sys::{Bool, Callbacks, MessageSelector, SampleType, Time};
use crossbeam::channel::{Receiver, Sender, bounded};
use std::cell::UnsafeCell;
use std::ffi::{c_long, c_void};
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::thread::yield_now;
use std::time::{Duration, Instant};

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

/// The two directions of one ASIO stream, as a host that owns settings wants them.
///
/// ASIO has no second driver slot, no second clock and no second buffer list, so a
/// request naming two of any of them is not a stream this module can open. cpal
/// answered that situation by using the first stream's size and re-rating the
/// driver for the second stream's rate; this answers it with a sentence, because a
/// user who chose 64 in and 128 out chose something that does not exist.
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
}

/// What a host does with each block. Both runs are interleaved — `frames` frames
/// of [`Transport::inputs`] and [`Transport::outputs`] channels — so a host that
/// wants one physical input reads every `inputs`-th sample, and a host that owns
/// the driver's first two outputs writes them in order.
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

/// Everything the driver's thread needs, reached through [`STATE`] because ASIO
/// hands a host no reference of any kind: `bufferSwitch` is a bare function
/// pointer with two arguments (`azo-sys-0.4.1/src/lib.rs:220-224`).
struct State {
    inputs: Vec<Channel>,
    outputs: Vec<Channel>,
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
    /// another's news; there is only one stream here, and its own number.
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
    /// channel's bytes down in its own buffer.
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
        for (index, channel) in self.outputs.iter().enumerate() {
            let Some(plane) = channel.plane(half, self.frames) else {
                continue;
            };
            let width = channel.pcm.bytes_per_frame();
            for frame in 0..self.frames {
                let source = &scratch[frame * channels + index..][..1];
                let target = &mut plane[frame * width..][..width];
                channel.pcm.write(source, target);
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

/// The one state the driver's callbacks may reach. ASIO permits one loaded driver
/// per process — cpal and asio-sys hold themselves to it with a process-global
/// `Asio` and refuse a second driver by name
/// (`cpal-0.18.2/src/host/asio/mod.rs:27-44`, `asio-sys-0.4.0/src/bindings/mod.rs:442-458`)
/// — so one pointer is not a limitation this module invents. [`open`] refuses a
/// second stream in words instead of letting two of them write over each other.
static STATE: AtomicPtr<State> = AtomicPtr::new(null_mut());

/// Whether [`STATE`] is spoken for, including the stretch of an open where it is
/// still null. Held by the live [`Transport`] and released when it is dropped.
static CLAIMED: AtomicBool = AtomicBool::new(false);

/// The claim on [`STATE`], released by being dropped — so every early return in
/// [`open`] gives the process its one ASIO slot back without repeating itself.
struct Claim;

impl Claim {
    /// The claim, if this process has no ASIO stream yet.
    fn take() -> Option<Self> {
        CLAIMED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| Claim)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        CLAIMED.store(false, Ordering::Release);
    }
}

/// The four functions a driver is allowed to call. All four are real: `azo`'s own
/// example leaves two of them `unimplemented!()`
/// (`azo-0.6.1/examples/sine.rs`), which is a panic crossing an `extern "system"`
/// boundary into a driver's thread — a driver that happens to use either one
/// would take the process down with it.
static CALLBACKS: Callbacks = Callbacks {
    buffer_switch,
    sample_rate_did_change,
    asio_message,
    buffer_switch_time_info,
};

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

/// `bufferSwitch`: one half of every buffer is ready.
unsafe extern "system" fn buffer_switch(half: c_long, direct: Bool) {
    let Some(state) = (unsafe { STATE.load(Ordering::Acquire).as_ref() }) else {
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
unsafe extern "system" fn sample_rate_did_change(rate: f64) {
    let Some(state) = (unsafe { STATE.load(Ordering::Acquire).as_ref() }) else {
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
        format!("Sample rate changed to {rate} Hz by the ASIO driver"),
        true,
    );
}

/// `asioMessage`: the driver's out-of-band requests, on the driver's thread.
unsafe extern "system" fn asio_message(
    selector: MessageSelector,
    value: c_long,
    _message: *const c_void,
    _opt: *const f64,
) -> c_long {
    let Some(state) = (unsafe { STATE.load(Ordering::Acquire).as_ref() }) else {
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
                    format!("The ASIO driver asked to {what}; the audio needs to be reopened"),
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
                    format!("The ASIO driver changed its buffer to {value} frames"),
                    true,
                );
            }
            Bool::TRUE.0
        }
        MessageSelector::OVERLOAD => {
            if let Some(guard) = Inside::enter(state) {
                guard.0.overloads.fetch_add(1, Ordering::Relaxed);
                guard
                    .0
                    .fail("The ASIO driver reported an overload".to_owned(), false);
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
unsafe extern "system" fn buffer_switch_time_info(
    time: *mut Time,
    half: c_long,
    direct: Bool,
) -> *mut Time {
    unsafe { buffer_switch(half, direct) };
    time
}

/// An open ASIO stream: both directions, one driver, one clock.
///
/// Dropping it stops the driver, gives its buffers back and reports anything that
/// went wrong through [`Transport::failure`], in the order the SDK uses
/// (`asio-sys-0.4.0/src/bindings/mod.rs:987-999`).
pub struct Transport {
    frames: u32,
    rate: u32,
    inputs: u32,
    outputs: u32,
    /// The driver, as a boxed farewell. `azo` keeps its `host` module private and
    /// re-exports only `Host` (`azo-0.6.1/src/lib.rs:6-17`), so the handle
    /// `create_driver` hands back has no name to put in a field. What the
    /// transport still needs from it after the open is `stop` and
    /// `dispose_buffers`, and that travels as this closure, which captured the
    /// handle while its type was still inferable. It reports what it could not do.
    close: Option<Box<dyn FnOnce() -> Vec<String> + Send>>,
    state: *mut State,
    errors: Receiver<Failure>,
    /// A second sender for the same channel, because the one inside [`State`] is
    /// freed with the state and the teardown messages are sent after that.
    notify: Sender<Failure>,
    /// So the panel goes on answering `None` to ASIO capability questions while
    /// this stream is up (`crate::devices::asio_stream_live`).
    _guard: AsioStreamGuard,
    /// The process's one ASIO slot. Released with this value.
    _claim: Claim,
}

// The handle travels inside `close`, and `azo`'s proxy is `Send`; the state is
// reached through the process-global pointer, guarded by the atomics above and by
// the wait in `Drop`; the pump the state owns is `Send` by the trait's own bound.
// So a transport may be moved, and dropped, on any thread. It is deliberately not
// `Sync`: `failure` only reads a channel, but two threads holding one transport
// would be two owners of one driver.
unsafe impl Send for Transport {}

impl Transport {
    /// Frames per block — the number the caller asked for, or nothing.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The rate the stream was opened at.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// How many interleaved channels [`Pump::input`] carries.
    pub fn inputs(&self) -> u32 {
        self.inputs
    }

    /// How many interleaved channels [`Pump::output`] carries.
    pub fn outputs(&self) -> u32 {
        self.outputs
    }

    /// `kAsioOverload` reports so far, for a host that counts dropouts.
    pub fn overloads(&self) -> u64 {
        unsafe { &*self.state }.overloads.load(Ordering::Relaxed)
    }

    /// The next thing the driver said, if it said anything. Nothing here reopens
    /// or repairs anything by itself: `invalidated` is the part that says the host
    /// has to.
    pub fn failure(&self) -> Option<Failure> {
        self.errors.try_recv().ok()
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        // The SDK's order, and the SDK's error reporting which it did not have.
        // Both calls are `SendMessageW` to `azo`'s pump thread, which is why they
        // happen here and never from a callback.
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
            STATE.store(null_mut(), Ordering::Release);
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
        // `_claim` drops after this body, so the slot is free only once the state
        // is gone; `errors` drops with it, and a host still holding the other end
        // sees the messages sent above.
    }
}

/// Open the driver `wanted` names, and start it.
///
/// Either the driver takes exactly the rate and the period asked for, or this
/// returns a sentence about which of the two it refused and what it would have
/// taken. There is no "give me the nearest size": a caller that sized its rings,
/// its monitor and its plugin's block around 64 frames has no use for a stream
/// that came up at 96.
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
    let rate = wanted.input_rate;
    let frames = wanted.input_frames;
    let driver_name = wanted.input;

    // The process's one ASIO slot, before anything is loaded, so a second attempt
    // fails here rather than after a driver has been initialized twice.
    let claim = Claim::take().ok_or_else(|| {
        format!("This process already has an ASIO stream open, so '{driver_name}' cannot be opened again")
    })?;
    let guard = devices::asio_stream_guard();

    let meta = devices::asio_driver(driver_name)
        .ok_or_else(|| format!("'{driver_name}' is not registered as an ASIO driver any more"))?;
    // The thread this driver's apartment lives on. Its own thread is the point:
    // the rack's UI thread is not where a driver's COM apartment belongs, and
    // every call on the handle this hands back is answered by the thread that made
    // the instance.
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
    if !enough(channels.out, wanted.outputs) {
        return Err(format!(
            "'{driver_name}' has {} output channels and this needs {}",
            channels.out, wanted.outputs
        ));
    }

    // The size, asked of the driver and judged by the rule the panel's list is
    // built from. This is where a saved setting naming 24 stops being a surprise:
    // the panel's own check validates only the min..max span, so such a setting
    // reached the driver and was refused there, after a load.
    let mut ladder = driver
        .buffer_size()
        .map(|size| Ladder::from_size(&size))
        .unwrap_or(Ladder::UNKNOWN);
    if !ladder.accepts(frames) {
        return Err(refused(driver_name, frames, ladder));
    }

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
        if !ladder.accepts(frames) {
            return Err(refused(driver_name, frames, ladder));
        }
    }

    // One `channelInfo` per opened channel, both directions. The format is
    // per-channel in ASIO and asio-sys asks channel 0 and assumes the rest agree
    // (`asio-sys-0.4.0/src/bindings/mod.rs:1091-1098`); here a driver that mixes
    // formats is refused by the channel that differs, in the words the driver
    // itself used to name it.
    let formats = |input: bool, count: u32| -> Result<Vec<Pcm>, String> {
        (0..count)
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
    let input_formats = formats(true, wanted.inputs)?;
    let output_formats = formats(false, wanted.outputs)?;

    // Both directions, one call, inputs first. The buffers come back as two
    // pointers per channel, in the order they were asked for
    // (`azo-0.6.1/src/driver.rs:92-103`).
    let ids = (0..wanted.inputs)
        .map(|index| ChannelId {
            input: true,
            index: index as c_long,
        })
        .chain((0..wanted.outputs).map(|index| ChannelId {
            input: false,
            index: index as c_long,
        }));
    let frames_long =
        c_long::try_from(frames).map_err(|_| "Buffer size is out of range".to_owned())?;
    // The one obligation `create_buffers` states: the callbacks must outlive the
    // buffers. `CALLBACKS` is a `static`, so it outlives everything
    // (`azo-0.6.1/src/driver.rs:92-94`).
    let halves = unsafe { driver.create_buffers(ids, frames_long, &raw const CALLBACKS) }
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
    if halves.len() != (wanted.inputs + wanted.outputs) as usize {
        // A driver that answered fewer buffers than channels would have the
        // indexing below reach past the answer, so it is an error and not a panic.
        return Err(format!(
            "'{driver_name}' answered {} buffers for {} channels",
            halves.len(),
            wanted.inputs + wanted.outputs
        ));
    }
    let (input_halves, output_halves) = halves.split_at(wanted.inputs as usize);
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

    let (sender, errors) = bounded(16);
    let notify = sender.clone();
    let frames_usize = frames as usize;
    let state = Box::new(State {
        inputs,
        outputs,
        frames: frames_usize,
        capture: UnsafeCell::new(vec![0.0; frames_usize * wanted.inputs as usize]),
        playback: UnsafeCell::new(vec![0.0; frames_usize * wanted.outputs as usize]),
        pump: UnsafeCell::new(Box::new(pump) as Box<dyn Pump>),
        errors: sender,
        phase: AtomicU8::new(STARTING),
        inside: AtomicUsize::new(0),
        last_half: AtomicUsize::new(usize::MAX),
        rate: applied,
        overloads: AtomicU64::new(0),
    });

    // Silence, before the driver starts and before anyone can hear it. Zero bytes
    // is silence in every format this module writes: the integral ones are
    // two's-complement and a float zero is all-bits-zero.
    for channel in &state.outputs {
        for half in 0..HALVES {
            if let Some(plane) = channel.plane(half, frames_usize) {
                plane.fill(0);
            }
        }
    }

    let published = Box::into_raw(state);
    // The claim above makes this pointer ours alone, so a plain store publishes
    // it. No callback can run before `start`, which is what lets the first one
    // find its state and nothing find it before it is ready.
    STATE.store(published, Ordering::Release);
    if let Err(error) = driver.start() {
        // Back the state away before freeing it. Nothing was started, so nothing
        // can be inside it; the buffers go back the same way they would have on a
        // close.
        unsafe {
            (*published).phase.store(DETACHED, Ordering::SeqCst);
            STATE.store(null_mut(), Ordering::Release);
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
    let close = Box::new(move || {
        let mut problems = Vec::new();
        // `stop` first: after it returns, a driver that keeps its side of the
        // standard calls no more callbacks.
        if let Err(error) = driver.stop() {
            problems.push(refusal(complaint(&driver, "'stop' failed"), error));
        }
        if let Err(error) = driver.dispose_buffers() {
            problems.push(refusal(
                complaint(&driver, "'dispose buffers' failed"),
                error,
            ));
        }
        problems
    });
    Ok(Transport {
        frames,
        // The number the user chose, which is the one the rings were sized from;
        // `applied` is what the driver answered, within a hertz of it.
        rate,
        inputs: wanted.inputs,
        outputs: wanted.outputs,
        close: Some(close),
        state: published,
        errors,
        notify,
        _guard: guard,
        _claim: claim,
    })
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

        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "a refused request still reached the host"
        );
    }

    /// The driver's own message call, with the two arguments this host does not
    /// use. The numbers in the assertions are Steinberg's, which is what the
    /// module's comment about the numbering claims; a renumbering in a future
    /// `azo` shows up here as a red test rather than as a driver whose reset
    /// requests are silently declined.
    fn ask(selector: MessageSelector, value: c_long) -> c_long {
        unsafe { asio_message(selector, value, null(), null()) }
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
            inputs: vec![Channel {
                halves: [input[0].as_mut_ptr().cast(), input[1].as_mut_ptr().cast()],
                pcm: Pcm::Int16 { big_endian: false },
            }],
            outputs: vec![Channel {
                halves: [output[0].as_mut_ptr().cast(), output[1].as_mut_ptr().cast()],
                pcm: Pcm::Float32 { big_endian: false },
            }],
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

    /// The process holds one slot, one state, and the answers a driver is given
    /// while it holds them — so they are tested in one test, in order. `cargo
    /// test` runs the tests of a binary in threads of one process, and a second
    /// test claiming the slot the first is standing in would fail for a reason
    /// that has nothing to do with the code under test.
    #[test]
    fn one_slot_one_state_and_the_answers_a_driver_gets() {
        // A refusal costs nothing. The shape checks come first, so a settings
        // file naming two drivers leaves the process free to open the one it
        // should have named.
        let (pump, _) = counting();
        let refused = open(wanted("Left", "Right"), pump).err();
        assert!(refused.is_some(), "two drivers were accepted");
        let claim = Claim::take().expect("the first stream takes the slot");
        assert!(Claim::take().is_none(), "a second slot was handed out");

        let (sender, errors) = bounded(16);
        let (pump, calls) = counting();
        // A stream with no channels and no buffers: everything below is answered
        // by the state, before any driver exists.
        let state = Box::into_raw(Box::new(State {
            inputs: Vec::new(),
            outputs: Vec::new(),
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
        STATE.store(state, Ordering::Release);

        // A callback for a stream with nothing in it says nothing to the host.
        unsafe { buffer_switch(0, Bool::FALSE) };
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

        // An overload is one dropped block, and not a reason to reopen.
        assert_eq!(ask(MessageSelector::OVERLOAD, 0), Bool::TRUE.0);
        let dropout = errors.try_recv().expect("an overload reaches the host");
        assert!(
            !dropout.invalidated,
            "one dropped block asked the host to reopen"
        );
        assert_eq!(unsafe { &*state }.overloads.load(Ordering::Relaxed), 1);

        // Half a hertz is the same clock read coarsely; 44.1 kHz is another clock.
        unsafe { sample_rate_did_change(48_000.5) };
        assert!(
            errors.try_recv().is_err(),
            "the same clock, read coarsely, was news"
        );
        unsafe { sample_rate_did_change(44_100.0) };
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

        // Taken away the way [`Transport::drop`] takes it away, and the slot with it.
        unsafe {
            (*state).phase.store(DETACHED, Ordering::SeqCst);
            STATE.store(null_mut(), Ordering::Release);
            drop(Box::from_raw(state));
        }
        drop(claim);
        assert!(
            Claim::take().is_some(),
            "the slot did not come back with the stream"
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
}
