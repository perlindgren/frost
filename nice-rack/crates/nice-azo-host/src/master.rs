//! The master role: whoever wins the election owns the clock, the mix,
//! and (when configured) the real hardware driver hosted through `azo`.
//!
//! The engine has one loop shape for both clock sources. In hardware
//! mode the loop body runs inside the real driver's `bufferSwitch`
//! callback — the hardware is the clock, and every client hears it
//! exactly as the master's own host does. In virtual mode a paced
//! thread produces the same calls at the same cadence. Either way the
//! body is [`Engine::step`]: consume published output blocks into a
//! frame-addressed mix scratch, then publish input blocks (hardware
//! pass-through plus the summed virtual channels) to every slot.
//!
//! The mix scratch is why there are no locks on the audio path at all.
//! Every cell is addressed by absolute frame number; the master is the
//! single writer and single reader of both scratch planes, clients only
//! ever touch their own sequence-gated rings, and the real driver's
//! half-buffers are copied at the callback edges. A consumer that wakes
//! late reads zeros or skips stale blocks — never a torn mix.

use std::os::raw::c_void;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use azo::driver::{Driver as AzDriver, SafeHandle};
use azo::dto::{ChannelCounts, ChannelId};
use azo::sys::{self, Callbacks, Time};

use crate::bus::{Dir, SharedBus};
use crate::proto::{self, MAX_BLOCK, SLOT_ACTIVE, VIRT_CH, VIRT_FIRST, align_up};
use crate::register::{self, Config};
use crate::win;

/// Frame-addressed circular scratch: 16k frames (~350 ms at 48 kHz) per
/// channel, addressed `frame & CIRC_MASK`. Far wider than any sane
/// client's jitter, and the modulus is a mask.
const CIRC_FRAMES: u64 = 1 << 14;
const CIRC_MASK: u64 = CIRC_FRAMES - 1;

/// How far behind the live edge a block may sit and still be summed
/// into the mix — one quarter of the scratch, comfortably inside the
/// window whose cells the zero frontier has not already re-armed for
/// their next wrap-around. Older than this is history: nobody can hear
/// it any more, and waiting for it could stall the whole bus.
const BACKLOG_FRAMES: u64 = CIRC_FRAMES / 4;

/// Diagnostic pin: `NICEASIO_SILENT_HW=1` makes the master measure the
/// mix it is about to hand the real driver, log its peak several times
/// a second, and send **silence** to the hardware instead.
///
/// It exists because the last leg of the bus — client output → bus →
/// master → the driver's own output buffers — is the one leg no
/// in-process test can hear. Loopback measures what a *client*
/// receives; the DAC never enters that picture, and a test that played
/// the tone out loud is not a test anyone should have to run. With the
/// pin on, `niceroute-probe solo --tone` proves the whole chain with
/// numbers and zero sound: if the peak reaches the master's hardware
/// stage, the mix is landing where the driver will read it.
fn silent_hw() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("NICEASIO_SILENT_HW").is_some_and(|v| v != "0" && !v.is_empty())
    })
}

pub(crate) struct Master {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

/// What `init` learns once the master thread has made the hardware
/// decision — synchronously, so the first `getChannels` after `init`
/// already answers real numbers. (A device that answers "0 channels,
/// ask me later" is the specific failure this whole design exists to
/// outgrow.)
pub(crate) struct Decision {
    pub message: String,
}

/// What one hardware channel's buffer actually holds.
///
/// `createBuffers` hands back `void**` and no format: the shape of the
/// samples is whatever each channel reported from `getChannelInfo`, and
/// that answer is not a formality. The Komplete Audio driver on this
/// machine says `PCM_I32_LSB` — 32-bit integer — for every channel, so a
/// host that writes `0.25f32` (bit pattern `0x3E800000`) into it is not
/// writing a quiet sample at all: read as an integer that is 1 048 576 000,
/// which against a 2^31 full scale is 0.488, about 6 dB hot — a true
/// quarter of full scale would be 536 870 912. Worse than the level is the
/// *shape*: the bit pattern of a sine read this way is a sawtooth of
/// exponents that flips wildly whenever the sample goes negative. Silence
/// still survives it, because `0.0f32`
/// and the integer `0` are the same bytes, which is why the device plays
/// quiet instead of refusing. Everything measured inside the bus looked
/// perfect because the corruption is one `memcpy` away from where the
/// measuring happens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HwFmt {
    F32,
    I32,
}

/// Both supported shapes are four bytes a frame.
const HW_SAMPLE_BYTES: usize = 4;

/// The smallest block the bus will let the hardware clock it at, when the
/// driver picked the size itself.
///
/// This is a measured floor, not a guess. On the machine this was built
/// on, with a tone muted at the driver door so measuring stayed silent, the
/// driver's own callback spacing over ~10 s per size was:
///
/// | block | period  | late of ticks | worst gap | worst work |
/// |-------|---------|---------------|-----------|------------|
/// | 64    | 1.33 ms | 255 of 1501   | 2.08 ms   | 54 µs      |
/// | 128   | 2.67 ms | 4 of 751      | 4.05 ms   | 91 µs      |
/// | 256   | 5.33 ms | 6 of 376      | 8.05 ms   | 59 µs      |
///
/// `late` is a gap of more than 1.5 periods, so the 128 and 256 rows are
/// each compared against their own window. A sixth of the 64-frame
/// callbacks landed late because the driver wakes on 1 ms USB frames and
/// 1.33 ms does not divide them; the work inside a callback never came
/// close to the budget at any size, so the misses are the clock and not
/// the load. The ring held everywhere — 0 dropped, 0 stalls — but at 64 it
/// held on a schedule the driver was missing a sixth of the time.
///
/// 1.33 ms of extra latency buys a thirty-fold drop in a metric that had
/// no margin in it, which is the trade this device exists to make: nobody
/// is holding a microphone to a speaker through this path. An explicit
/// `HwBlock` is honoured exactly and ignores this floor; the floor only
/// speaks for the size the driver chose.
const CLOCK_FLOOR: i32 = 128;

impl HwFmt {
    /// Accept the formats that mean "linear samples, little-endian, full
    /// scale is ±1.0" and nothing else — an unknown code is not a format
    /// to guess at, it is a driver to refuse.
    fn of(t: sys::SampleType) -> Option<HwFmt> {
        match t {
            sys::SampleType::PCM_F32_LSB => Some(HwFmt::F32),
            sys::SampleType::PCM_I32_LSB => Some(HwFmt::I32),
            _ => None,
        }
    }
}

/// Convert one block of the driver's samples into linear floats.
///
/// # Safety
/// `src` must point at `dst.len()` samples in the format `fmt` says.
unsafe fn load_frames(fmt: HwFmt, src: *const c_void, dst: &mut [f32]) {
    match fmt {
        HwFmt::F32 => {
            dst.copy_from_slice(unsafe { std::slice::from_raw_parts(src.cast::<f32>(), dst.len()) })
        }
        HwFmt::I32 => {
            let src = unsafe { std::slice::from_raw_parts(src.cast::<i32>(), dst.len()) };
            // The inverse of `store_frames`: 2³¹ counts is full scale, so
            // `i32::MIN` reads back as exactly −1.0 and `i32::MAX` one
            // count shy of +1.0.
            let scale = 1.0 / 2_147_483_648.0f32;
            for (d, &s) in dst.iter_mut().zip(src) {
                *d = s as f32 * scale;
            }
        }
    }
}

/// Convert one block of linear floats into the driver's samples, clamped
/// to full scale. Returns the loudest frame written, which is what the
/// `NICEASIO_SILENT_HW` pin reports.
///
/// # Safety
/// `dst` must point at `src.len()` samples in the format `fmt` says.
unsafe fn store_frames(fmt: HwFmt, dst: *mut c_void, src: &[f32]) -> f32 {
    let mut peak = 0.0f32;
    match fmt {
        HwFmt::F32 => {
            let dst = unsafe { std::slice::from_raw_parts_mut(dst.cast::<f32>(), src.len()) };
            for (d, &s) in dst.iter_mut().zip(src) {
                *d = s;
                peak = peak.max(s.abs());
            }
        }
        HwFmt::I32 => {
            let dst = unsafe { std::slice::from_raw_parts_mut(dst.cast::<i32>(), src.len()) };
            // Full scale is 2³¹, which is what `i32::MAX as f32` rounds
            // to anyway — and because it does, +1.0 lands one count above
            // the top of the range while −1.0 lands exactly on the bottom.
            // Both rails are clamped here instead of being left to the
            // saturating behaviour of `as i32`, so a full-scale square
            // wave comes out symmetric instead of loud on one half-cycle
            // and one count short on the other.
            const FULL: f32 = 2_147_483_648.0;
            for (d, &s) in dst.iter_mut().zip(src) {
                peak = peak.max(s.abs());
                let v = (s.clamp(-1.0, 1.0) * FULL).round();
                *d = if v >= i32::MAX as f32 {
                    i32::MAX
                } else if v <= i32::MIN as f32 {
                    i32::MIN
                } else {
                    v as i32
                };
            }
        }
    }
    peak
}

/// # Safety
/// `dst` must point at `frames * HW_SAMPLE_BYTES` writable bytes.
unsafe fn silence_frames(dst: *mut c_void, frames: usize) {
    // Zero is the same bit pattern in both shapes we speak, so one
    // memset covers the format question.
    unsafe { std::slice::from_raw_parts_mut(dst.cast::<u8>(), frames * HW_SAMPLE_BYTES) }.fill(0);
}

/// The real driver and its host buffers. The halves are the two
/// pointers ASIO's `createBuffers` handed back, exactly as handed back —
/// untyped, because the type they want is per channel and lives beside
/// them.
struct Hw {
    handle: SafeHandle,
    /// Frames per callback, as the driver accepted them. The engine's
    /// clock advances by exactly this much per `bufferSwitch`, so it is
    /// carried here rather than re-read from `getBufferSize` — which is
    /// an *offer*, and a clock derived from an offer the driver never
    /// agreed to would copy the wrong number of frames every callback.
    block: usize,
    in_bufs: Vec<[*mut c_void; 2]>,
    out_bufs: Vec<[*mut c_void; 2]>,
    in_fmt: Vec<HwFmt>,
    out_fmt: Vec<HwFmt>,
}

struct Engine {
    bus: SharedBus,
    hw: Option<Hw>,
    quantum: usize,
    hw_in: usize,
    hw_out: usize,
    /// Absolute frame the next step will consume/produce from.
    g: u64,
    /// Hardware input scratch, frame-addressed; written at the callback
    /// edge (or never, in virtual mode), read by `step`.
    in_circ: Vec<Box<[f32; CIRC_FRAMES as usize]>>,
    /// One callback's frames, in and out, as linear floats: the staging
    /// box between the driver's own shape and the frame-addressed
    /// scratch. Sized `quantum`, touched only on the callback thread.
    in_tmp: Vec<f32>,
    out_tmp: Vec<f32>,
    /// The mix, frame-addressed, one plane per virtual channel; both
    /// written and read by the master thread only.
    out_circ: Vec<Box<[f32; CIRC_FRAMES as usize]>>,
    /// High-water frame of `out_circ`: cells at or beyond it are kept
    /// zeroed, so a reader that runs ahead of the mix sums silence
    /// rather than somebody's old audio.
    mixed_upto: u64,
    mix_done: [u64; proto::SLOTS],
    /// The last output block each slot contributed to the mix, copied
    /// out of the ring at summing time (`c * VIRT_CH + j`, one
    /// `MAX_BLOCK`-frame box per slot channel). Publishing a client's
    /// inputs subtracts this snapshot back out of the mix so nobody
    /// hears their own output; reading the live ring cell instead would
    /// race the client recycling it for its next block.
    own_last: Vec<Box<[f32]>>,
    /// Per slot: the first output block actually summed into the mix.
    /// Blocks below it were skipped — published before the adoption
    /// stamp, or dropped as unrecoverable backlog — so their frames are
    /// *not* in the mix and must not be subtracted when a client hears
    /// everything-but-itself.
    mixed_from: [u64; proto::SLOTS],
    in_written: [u64; proto::SLOTS],
    /// `NICEASIO_SILENT_HW` bookkeeping: loudest sample the master has
    /// seen on its way to the real driver, and how many callbacks that
    /// covers. Unused (and always zero) when the pin is off.
    out_peak: f32,
    out_ticks: u32,
    /// One callback period in microseconds, for the deadline witness.
    period_us: u64,
    /// Deadline witness. The bus drops a client's block rather than
    /// stutter, so a distortion report is really the question "who
    /// missed the deadline, and by how much?" — which is answerable only
    /// from inside the callback. Three QPC reads per tick, no allocation,
    /// no logging unless something is actually wrong.
    tick_prev: i64,
    stat_prev: i64,
    ticks: u32,
    work_max_us: u64,
    late: u32,
    late_max_us: u64,
    dropped: u64,
    /// …and lateness past two and a half periods, which is no longer
    /// jitter: at that gap a driver with a two-half ring has started
    /// reading the half it is asking for. Counted apart because "often a
    /// little late" and "sometimes very late" are different bugs.
    starved: u32,
    /// Real-time registration for the thread the real driver calls us
    /// on. Not the master's own thread — see `hardware_tick`.
    rt: Option<win::Realtime>,
    /// Output blocks handed to the driver that were byte-for-byte the
    /// previous block again. Audio does not repeat itself across a
    /// callback boundary by accident, so a repeat is the DAC being fed
    /// the same frames twice: the literal waveform of "it sounds fuzzy"
    /// and, when the clock has stopped, of "it keeps looping".
    repeated: u32,
    prev_out: Vec<u8>,
    have_prev: bool,
    /// Loudest hardware *input* sample in the current window. A howling
    /// room and a feedback loop sound alike from the sofa; this is what
    /// tells them apart — nothing is coming in, or something is.
    in_peak: f32,
    /// Times a running tenant was caught behind the clock, and how many
    /// blocks deep the worst episode got. Every block of that depth is a
    /// block of zeros where a host's audio should have been, which is the
    /// usual shape of a "messy sound" report at a small block size: the
    /// host's own graph no longer fits inside one callback period, so the
    /// bus goes looking for a block that is still being rendered.
    stalls: u32,
    stall_max: u64,
    stalled_now: [bool; proto::SLOTS],
}

static ENGINE: AtomicPtr<Engine> = AtomicPtr::new(ptr::null_mut());

unsafe extern "system" fn buffer_switch(half: i32, _direct: sys::Bool) {
    // Safety: the pointer is set before the real driver starts and
    // cleared after it stops. The callback thread and the master thread
    // hold it at disjoint times — and during the priming callbacks a
    // driver fires *inside* `start`, the engine's `hw` field is still
    // detached, so the tick is a well-defined no-op.
    unsafe {
        if let Some(engine) = ENGINE.load(Ordering::Acquire).as_mut() {
            engine.hardware_tick(half.max(0) as usize);
        }
    }
}

unsafe extern "system" fn buffer_switch_time_info(
    time: *mut Time,
    half: i32,
    direct: sys::Bool,
) -> *mut Time {
    unsafe { buffer_switch(half, direct) };
    time
}

unsafe extern "system" fn sample_rate_did_change(_rate: f64) {}

unsafe extern "system" fn asio_message(
    _sel: sys::MessageSelector,
    _v: i32,
    _m: *const c_void,
    _o: *const f64,
) -> i32 {
    // Claim no optional host features: real drivers fall back to plain
    // `bufferSwitch`, which is all the engine implements.
    0
}

static CALLBACKS: Callbacks = Callbacks {
    buffer_switch,
    sample_rate_did_change,
    asio_message,
    buffer_switch_time_info,
};

impl Master {
    /// Takes the master role for `bus`. Blocks until the hardware
    /// decision is made; dropping the returned value stops and joins
    /// everything (the election claim itself is held by the caller and
    /// dropped afterwards, so abdication is ordered: clock first, seat
    /// second).
    pub(crate) fn start(bus: SharedBus, note: Arc<std::sync::Mutex<String>>) -> Master {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let thread_stop = stop.clone();
        let thread_note = note.clone();
        let join = std::thread::Builder::new()
            .name("niceroute-master".into())
            .spawn(move || master_thread(bus, thread_stop, tx, thread_note))
            .expect("spawn master thread");
        let decision = match rx.recv() {
            Ok(d) => d,
            Err(e) => {
                // The master thread died before it could decide the
                // bus. Failing `init` over that is how one internal
                // fault turns into "the device cannot even open": be
                // inert, be loud in the log, let the host live.
                crate::trace::log(&format!("master thread died before deciding: {e}"));
                *note.lock().unwrap() =
                    "NiceRack ASIO: master thread failed; device inert (see %TEMP%\\niceroute.log)"
                        .to_string();
                stop.store(true, Ordering::SeqCst);
                return Master {
                    stop,
                    join: Some(join),
                };
            }
        };
        if !decision.message.is_empty() {
            *note.lock().unwrap() = decision.message;
        }
        Master {
            stop,
            join: Some(join),
        }
    }
}

impl Drop for Master {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn master_thread(
    bus: SharedBus,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<Decision>,
    note: Arc<std::sync::Mutex<String>>,
) {
    // This thread paces the bus whenever there is no hardware clock, and
    // owns the real driver's setup and teardown either way; at normal
    // priority the virtual clock is a lottery.
    let _rt = win::real_time("Pro Audio");
    let config = Config::load();
    crate::trace::log(&format!(
        "master: starting, thread priority {}, real driver {:?}",
        _rt.how,
        config.real_driver.as_deref().unwrap_or("(none)")
    ));
    let mut message = String::new();
    let mut hw: Option<Hw> = None;
    if let Some(name) = config.real_driver.as_deref() {
        match open_hardware(name, &config, &mut message) {
            Ok(opened) => hw = Some(opened),
            Err(e) => {
                crate::trace::log(&format!("master: '{name}' unavailable: {e}"));
                message = format!("NiceRack ASIO: '{name}' unavailable ({e}); virtual-only")
            }
        }
    }

    let (rate, hw_in, hw_out, quantum) = match &hw {
        Some(Hw { handle, block, .. }) => {
            let counts = handle
                .channel_counts()
                .unwrap_or(ChannelCounts { in_: 0, out: 0 });
            let rate = handle.get_sample_rate().unwrap_or(48000.0);
            (
                rate as u32,
                (counts.in_.max(0) as usize).min(config.hw_in as usize),
                (counts.out.max(0) as usize).min(config.hw_out as usize),
                *block,
            )
        }
        None => {
            use Ordering::Relaxed;
            let existing = bus.header().rate_hz.load(Relaxed);
            let rate = if (8000..=192000).contains(&existing) {
                existing
            } else {
                48000
            };
            (rate, 0, 0, proto::DEFAULT_QUANTUM)
        }
    };
    let quantum = quantum.clamp(1, MAX_BLOCK);

    {
        use Ordering::Relaxed;
        let h = bus.header();
        h.rate_hz.store(rate, Relaxed);
        h.hw_in.store(hw_in as u32, Relaxed);
        h.hw_out.store(hw_out as u32, Relaxed);
        h.quantum.store(quantum as u32, Relaxed);
        h.master_pid.store(win::pid(), Relaxed);
        h.epoch.fetch_add(1, Relaxed);
    }
    crate::trace::log(&format!(
        "master: bus is {rate} Hz, hw {hw_in}+{hw_out}, quantum {quantum}"
    ));

    // The real driver starts with its `hw` detached from the engine: a
    // driver that primes inside `start` finds a no-op tick and zeroed
    // buffers (we zeroed both halves at creation), and the live tick
    // begins when the handle is re-attached after `start` returns.
    let mut engine = Engine {
        bus: bus.clone(),
        hw: None,
        quantum,
        hw_in,
        hw_out,
        g: 0,
        in_circ: (0..hw_in.max(1))
            .map(|_| Box::new([0.0f32; CIRC_FRAMES as usize]))
            .collect(),
        in_tmp: vec![0.0f32; quantum],
        out_tmp: vec![0.0f32; quantum],
        out_circ: (0..VIRT_CH)
            .map(|_| Box::new([0.0f32; CIRC_FRAMES as usize]))
            .collect(),
        mixed_upto: 0,
        mix_done: [0; proto::SLOTS],
        own_last: (0..proto::SLOTS * VIRT_CH)
            .map(|_| vec![0.0f32; MAX_BLOCK].into_boxed_slice())
            .collect(),
        mixed_from: [0; proto::SLOTS],
        in_written: [0; proto::SLOTS],
        out_peak: 0.0,
        out_ticks: 0,
        period_us: ((quantum as u64).saturating_mul(1_000_000) / (rate as u64).max(1)).max(1),
        tick_prev: 0,
        stat_prev: 0,
        ticks: 0,
        work_max_us: 0,
        late: 0,
        late_max_us: 0,
        dropped: 0,
        starved: 0,
        rt: None,
        repeated: 0,
        prev_out: vec![0u8; hw_out.max(1) * quantum * HW_SAMPLE_BYTES],
        have_prev: false,
        in_peak: 0.0,
        stalls: 0,
        stall_max: 0,
        stalled_now: [false; proto::SLOTS],
    };
    if let Some(h) = hw.as_mut() {
        for bufs in h.in_bufs.iter_mut().chain(h.out_bufs.iter_mut()) {
            for half in bufs.iter_mut() {
                unsafe { silence_frames(*half, quantum) };
            }
        }
    }

    let mut started_hw = false;
    if let Some(pending) = hw {
        let engine_ptr: *mut Engine = &mut engine;
        ENGINE.store(engine_ptr, Ordering::SeqCst);
        match pending.handle.start() {
            Ok(()) => {
                engine.hw = Some(pending);
                started_hw = true;
                crate::trace::log("master: hardware started, it is the clock");
            }
            Err(e) => {
                crate::trace::log(&format!("master: hardware refused to start: {e:?}"));
                message = format!("NiceRack ASIO: hardware refused to start ({e:?}); virtual-only");
                drop(pending); // SafeHandle dies here, on its apartment thread
                engine.hw_in = 0;
                engine.hw_out = 0;
                engine.quantum = proto::DEFAULT_QUANTUM;
                let h = engine.bus.header();
                h.hw_in.store(0, Ordering::Relaxed);
                h.hw_out.store(0, Ordering::Relaxed);
                h.quantum
                    .store(proto::DEFAULT_QUANTUM as u32, Ordering::Relaxed);
                ENGINE.store(ptr::null_mut(), Ordering::SeqCst);
            }
        }
    }

    if !started_hw {
        crate::trace::log("master: no hardware clock; running the virtual clock");
    }
    if !message.is_empty() {
        crate::trace::log(&format!("master: note: {message}"));
    }
    let _ = tx.send(Decision { message });

    if started_hw {
        // The hardware is the clock; park until the role ends.
        while !stop.load(Ordering::Relaxed) {
            win::sleep_ms(25);
        }
        if let Some(hw) = engine.hw.as_mut() {
            // Hand the driver silence and let it play that out *before*
            // letting go. A driver stops cleanly when the last thing it
            // was given was silence; given a block of music, it is free
            // to keep cycling its own DMA ring after the host walked
            // away, and the room keeps hearing the last 64 samples of
            // REAPER over and over — which is exactly "it went on
            // looping after I stopped".
            for bufs in hw.out_bufs.iter_mut() {
                for half in bufs.iter_mut() {
                    unsafe { silence_frames(*half, engine.quantum) };
                }
            }
            let period_ms = (engine.period_us / 1000).max(1) as u32 + 2;
            win::sleep_ms(period_ms);
            let _ = hw.handle.stop();
            // No ticks after the buffers are gone: clear the pointer
            // while they are still alive, so a driver that calls back
            // late finds a well-defined no-op instead of freed memory.
            ENGINE.store(ptr::null_mut(), Ordering::SeqCst);
            let _ = hw.handle.dispose_buffers();
        } else {
            ENGINE.store(ptr::null_mut(), Ordering::SeqCst);
        }
    } else {
        // Virtual clock: a paced thread producing identical steps.
        // Absolute deadlines — the period is not a whole number of
        // milliseconds, and re-rounding it every step would drift the
        // world slow even with a perfect timer. A missed deadline
        // rebases the schedule instead of firing a burst of expired
        // steps: audio positions stay monotonic, consumers just see
        // one longer gap.
        let period_ticks = (engine.quantum as i128 * 1_000_000_000 / rate as i128
            * win::qpc_freq() as i128
            / 1_000_000_000)
            .max(1) as i64;
        let timer = win::Timer::pacing();
        let mut next = win::now();
        while !stop.load(Ordering::Relaxed) {
            let t0 = win::now();
            engine.step(engine.quantum);
            engine.g += engine.quantum as u64;
            engine.witness(t0);
            next.0 += period_ticks;
            let wall = win::now();
            if next.0 <= wall.0 {
                next = wall;
            } else {
                timer.wait_until(&next);
            }
        }
    }
    // `engine` — and with it the SafeHandle, on its own apartment
    // thread — drops here.
    let _ = note;
}

/// Resolve a display name to a CLSID (both registry hives), instantiate
/// and initialize it, and negotiate buffers — all on the master thread,
/// so the driver keeps the COM apartment it was born in.
fn open_hardware(name: &str, config: &Config, note: &mut String) -> Result<Hw, String> {
    let clsid = register::clsid_for_name(name).ok_or("not registered")?;
    let guid: azo::windows_core::GUID = unsafe { std::mem::transmute_copy(&clsid) };
    debug_assert_eq!(
        std::mem::size_of::<azo::windows_core::GUID>(),
        std::mem::size_of::<win::Guid>()
    );
    let handle = SafeHandle::new(&guid).map_err(|e| format!("CoCreateInstance: {e}"))?;
    if !handle.init(None) {
        return Err("init refused".into());
    }
    let counts = handle
        .channel_counts()
        .map_err(|e| format!("getChannels: {e:?}"))?;
    let size = handle
        .buffer_size()
        .map_err(|e| format!("getBufferSize: {e:?}"))?;
    // The driver's preferred size is by definition inside its
    // granularity set; `HwBlock` overrides the offer but is still clamped
    // into the driver's own range, and everything is clamped against
    // nonsense.
    let asked = i32::try_from(config.hw_block).unwrap_or(0);
    let block = if asked > 0 {
        asked.clamp(size.min.max(1), size.max.max(size.min))
    } else {
        // A driver asking for 64 frames is talking about its own DMA
        // ring, not about how late it is allowed to be. Measured here, a
        // 1.33 ms clock on this USB device gets answered in 2.07 ms gaps
        // a sixth of the time, because the callbacks land on 1 ms frames
        // and 1.33 does not divide 2. Nothing dropped — the two halves of
        // the ring were exactly the margin that saved it — but that was
        // the entire margin, and the same shape at 128 frames and above
        // costs 1 late callback in a hundred instead of one in six.
        // So a preference is honoured upward, never downward, and
        // `HwBlock` (or `NICEASIO_HW_BLOCK`) still says otherwise.
        let preferred = size.preferred.max(size.min).min(size.max.max(size.min));
        let floor = preferred.max(CLOCK_FLOOR);
        if floor != preferred {
            crate::trace::log(&format!(
                "master: driver prefers {preferred} frames, clocking at {floor}; HwBlock overrides"
            ));
        }
        floor
    };
    let block = block.clamp(32, MAX_BLOCK as i32) as usize;
    let in_n = (counts.in_.max(0) as usize).min(config.hw_in as usize);
    let out_n = (counts.out.max(0) as usize).min(config.hw_out as usize);
    let mut ids = Vec::new();
    ids.extend((0..in_n).map(|index| ChannelId {
        input: true,
        index: index as i32,
    }));
    ids.extend((0..out_n).map(|index| ChannelId {
        input: false,
        index: index as i32,
    }));
    // Ask the driver what it is going to put in those buffers, before
    // treating a single byte of them as a sample.
    //
    // `createBuffers` hands back `void**` and no format; the format is
    // whatever each channel reported from `getChannelInfo`, and it is the
    // only thing that says how to read them. Write the wrong shape and
    // every measurement inside the bus still says the mix is perfect,
    // because the corruption happens on the far side of the boundary, in
    // the driver's interpretation of bytes we never looked at — silence
    // survives, because zero is zero in both, and music does not. So:
    // ask, log the answer for every channel, convert to whatever it
    // asked for, and refuse a code we do not know rather than guess at it.
    let mut fmts = Vec::with_capacity(ids.len());
    let mut unknown = Vec::new();
    for id in ids.iter() {
        match handle.channel_info(*id) {
            Ok(info) => {
                crate::trace::log(&format!(
                    "master: hw {} {} is '{}', type {} ({:#010x})",
                    if id.input { "in " } else { "out" },
                    id.index,
                    info.name.to_string_lossy(),
                    info.sample_type,
                    i64::from(info.sample_type.0),
                ));
                match HwFmt::of(info.sample_type) {
                    Some(f) => fmts.push(f),
                    None => unknown.push(format!(
                        "{} {} = {}",
                        if id.input { "in" } else { "out" },
                        id.index,
                        info.sample_type
                    )),
                }
            }
            // A driver that will not say is a driver that has to be
            // listened to, not argued with: the shape of a sample is only
            // safe once it has been confirmed.
            Err(e) => unknown.push(format!(
                "{} {} refused getChannelInfo ({e:?})",
                if id.input { "in" } else { "out" },
                id.index
            )),
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "buffers are not 32-bit float or int: {}",
            unknown.join(", ")
        ));
    }
    let in_fmt = fmts[..in_n].to_vec();
    let out_fmt = fmts[in_n..].to_vec();
    let buffers: Vec<[*mut c_void; 2]> = unsafe {
        handle.create_buffers(ids.iter().copied(), block as i32, ptr::addr_of!(CALLBACKS))
    }
    .map_err(|e| format!("createBuffers: {e:?}"))?
    .collect();
    if buffers.len() != ids.len() {
        return Err("createBuffers returned a short list".into());
    }
    *note = format!(
        "NiceRack ASIO: hosting '{name}' at {} Hz, block {block}",
        handle.get_sample_rate().unwrap_or(0.0)
    );
    Ok(Hw {
        handle,
        block,
        in_bufs: buffers[..in_n].to_vec(),
        out_bufs: buffers[in_n..].to_vec(),
        in_fmt,
        out_fmt,
    })
}

impl Engine {
    /// One real-driver callback: stage its inputs, run the world, take
    /// the mix as its outputs — all at the absolute position `g`.
    fn hardware_tick(&mut self, half: usize) {
        let t0 = win::now();
        if self.rt.is_none() {
            // The thread that feeds the DAC is the *driver's* callback
            // thread. `master_thread` parks itself as soon as the
            // hardware is running, so the master's own MMCSS registration
            // dresses the spectator and leaves the performer in the
            // crowd: the real-time claim has to be made from inside the
            // first callback, by the thread that will be answering all
            // the others.
            let rt = win::real_time("Pro Audio");
            crate::trace::log(&format!("master: callback thread priority {}", rt.how));
            self.rt = Some(rt);
        }
        let block = self.quantum;
        let half = half % 2;
        // Snapshot what the callback owns first: `step` needs `&mut
        // self`, and the buffers themselves are owned by `hw`, which
        // cannot move or die while this callback is on the stack.
        let (in_ptrs, out_ptrs, in_fmt, out_fmt): (
            Vec<*mut c_void>,
            Vec<*mut c_void>,
            Vec<HwFmt>,
            Vec<HwFmt>,
        ) = match self.hw.as_ref() {
            Some(hw) => (
                hw.in_bufs.iter().map(|b| b[half]).collect(),
                hw.out_bufs.iter().map(|b| b[half]).collect(),
                hw.in_fmt.clone(),
                hw.out_fmt.clone(),
            ),
            None => return,
        };
        for ch in 0..self.hw_in.min(in_ptrs.len()).min(in_fmt.len()) {
            let frames = &mut self.in_tmp[..block];
            unsafe { load_frames(in_fmt[ch], in_ptrs[ch], frames) };
            for &s in frames.iter() {
                self.in_peak = self.in_peak.max(s.abs());
            }
            let dst = &mut self.in_circ[ch];
            for (i, &s) in frames.iter().enumerate() {
                dst[((self.g + i as u64) & CIRC_MASK) as usize] = s;
            }
        }
        self.step(block);
        // The mix only exists *behind* the clock edge. A healthy
        // lockstep client publishes block `m` as the clock passes its
        // last frame, and the consume pass sums it at `f0 = start +
        // m*block` — so at the edge `g` the newest frames anyone's
        // audio sits in are `[g-block, g)`, and `[g, g+block)` is still
        // the zero frontier the frontier pass armed ahead of the
        // readers. Reading the frontier and handing it to the DAC is
        // how a live, correctly summing bus plays permanent silence:
        // the virtual channels prove the mix is right (clients hear
        // each other exactly), because the publish pass reads that same
        // scratch one block back. The hardware takes the same window
        // now, at the cost of one block of output delay — which is what
        // the clients already pay, so the two directions stay in phase.
        let out_g = self.g.checked_sub(block as u64);
        for k in 0..self.hw_out.min(out_ptrs.len()).min(out_fmt.len()) {
            let frames = &mut self.out_tmp[..block];
            match out_g {
                Some(out_g) => {
                    let plane = &self.out_circ[k];
                    for (i, d) in frames.iter_mut().enumerate() {
                        *d = plane[((out_g + i as u64) & CIRC_MASK) as usize];
                    }
                }
                // The first callback has no "one block back" to read:
                // silence is the only honest answer, and writing the
                // frames at 0 here would put the same samples in both
                // halves — a click, and a repeat the witness below would
                // have to excuse.
                None => frames.fill(0.0),
            }
            let p = out_ptrs[k];
            let peak = unsafe { store_frames(out_fmt[k], p, frames) };
            // Compared on the bytes the driver was actually handed: the
            // same frames going out twice is a fact about the DAC feed,
            // whatever shape that feed happens to be in. Silence is
            // excluded, and not for economy: an idle channel repeats zero
            // forever, and calling that a stall would bury the one
            // repeat worth reading about.
            let bytes = block * HW_SAMPLE_BYTES;
            let written = unsafe { std::slice::from_raw_parts(p as *const u8, bytes) };
            if let Some(prev) = self
                .prev_out
                .get_mut(k * bytes..)
                .and_then(|s| s.get_mut(..bytes))
            {
                if self.have_prev && peak > 0.0 && prev == written {
                    self.repeated += 1;
                }
                prev.copy_from_slice(written);
            }
            if silent_hw() {
                // The pin: what would have gone to the DAC is measured,
                // then erased. Logging runs on the driver's own thread,
                // which is exactly why this is opt-in and never on by
                // default — but it is the only way to see this leg.
                self.out_peak = self.out_peak.max(peak);
                unsafe { silence_frames(p, block) };
            }
        }
        self.have_prev = true;
        if silent_hw() {
            self.out_ticks += 1;
            if self.out_ticks >= 256 {
                crate::trace::log(&format!(
                    "master: hw-out peak {:.6} over {} callbacks at block {}",
                    self.out_peak, self.out_ticks, self.quantum
                ));
                self.out_peak = 0.0;
                self.out_ticks = 0;
            }
        }
        self.g += block as u64;
        self.witness(t0);
    }

    /// One callback's deadline accounting: how long the master's own work
    /// took against the period it had, and whether a client's block had
    /// to be dropped. Silent while the bus keeps up; one line every two
    /// seconds the moment it does not, because "it sounds distorted" is
    /// a question about deadlines and this is the only place that can
    /// answer it. Cost is three counter reads and no allocation.
    fn witness(&mut self, start: win::Instant_) {
        let now = win::now();
        let freq = win::qpc_freq().max(1);
        if self.tick_prev != 0 {
            let gap_us = (now.0 - self.tick_prev).max(0) as u64 * 1_000_000 / freq as u64;
            if gap_us > self.period_us + self.period_us / 2 {
                self.late += 1;
                self.late_max_us = self.late_max_us.max(gap_us);
            }
            if gap_us > self.period_us * 2 + self.period_us / 2 {
                self.starved += 1;
            }
        }
        self.tick_prev = now.0;
        let work_us = (now.0 - start.0).max(0) as u64 * 1_000_000 / freq as u64;
        self.work_max_us = self.work_max_us.max(work_us);
        self.ticks += 1;
        if self.stat_prev == 0 {
            self.stat_prev = now.0;
            return;
        }
        let window_us = (now.0 - self.stat_prev).max(0) as u64 * 1_000_000 / (freq as u64);
        if window_us < 2_000_000 {
            return;
        }
        if self.late > 0 || self.dropped > 0 || self.repeated > 0 || self.stalls > 0 {
            crate::trace::log(&format!(
                "master: {} ticks, work max {} us of {} us period, {} late/{} dry (worst {} us), {} repeated blocks, {} dropped, {} stalls (worst {} blocks deep), hw-in peak {:.6}",
                self.ticks,
                self.work_max_us,
                self.period_us,
                self.late,
                self.starved,
                self.late_max_us,
                self.repeated,
                self.dropped,
                self.stalls,
                self.stall_max,
                self.in_peak,
            ));
        }
        self.stat_prev = now.0;
        self.ticks = 0;
        self.work_max_us = 0;
        self.late = 0;
        self.late_max_us = 0;
        self.starved = 0;
        self.repeated = 0;
        self.in_peak = 0.0;
        self.stalls = 0;
        self.stall_max = 0;
        self.dropped = 0;
    }

    /// Advance the whole bus by `frames`: adopt new slots, consume
    /// published output blocks into the mix, publish complete input
    /// blocks. Called only from the master thread (the real driver's
    /// callback, or the virtual clock).
    fn step(&mut self, frames: usize) {
        let g = self.g;
        let committed = g + frames as u64;
        let bus = &self.bus;

        // Extend the zero frontier of the mix ahead of any reader,
        // before this step's sums and before the heartbeat moves.
        let frontier =
            (committed + CIRC_FRAMES / 4).min(committed + CIRC_FRAMES - MAX_BLOCK as u64);
        if frontier > self.mixed_upto {
            for plane in self.out_circ.iter_mut() {
                let mut f = self.mixed_upto;
                while f < frontier {
                    plane[(f & CIRC_MASK) as usize] = 0.0;
                    f += 1;
                }
            }
            self.mixed_upto = frontier;
        }

        for c in 0..proto::SLOTS {
            let slot = bus.slot(c);
            use Ordering::{Acquire, Release};
            if slot.state.load(Acquire) != SLOT_ACTIVE {
                continue;
            }
            let block = slot.block.load(Acquire).max(1) as u64;
            let mut start = slot.start_frame.load(Acquire);
            if start == 0 {
                // New tenant: adopt at the next block boundary ahead of
                // the clock. Only the master-side cursors are set —
                // whatever the client published before the stamp is
                // skipped, not unwound, so the two sides never fight
                // over the same counter.
                start = align_up(committed, block);
                slot.start_frame.store(start, Release);
                slot.consumed_out.store(0, Release);
                self.mix_done[c] = slot.out_seq.load(Acquire);
                self.mixed_from[c] = self.mix_done[c];
                self.in_written[c] = slot.in_seq.load(Acquire);
            }

            // — consume this slot's published output blocks into the mix —
            let out_seq = slot.out_seq.load(Acquire);
            // Blocks this tenant has been handed to render and still has
            // not delivered. Measured against the releases, not against
            // the clock: a client is *allowed* to be one block deep in
            // its own rendering — that is the protocol, we publish its
            // inputs, then it publishes its outputs — and a metric that
            // counts that as a miss cries on every other callback.
            // Beyond one, the frames it owes are being read as the zero
            // frontier, which is a block of digital fuzz out of the
            // speakers.
            let owed = self.in_written[c].saturating_sub(out_seq + 1);
            if owed > 0 {
                // Edge-triggered, and measured in how deep it went: a
                // host whose transport is simply stopped is in debt
                // forever, and a counter that grows while nobody is
                // playing teaches nobody anything. Episodes and depth do.
                if !self.stalled_now[c] {
                    self.stalled_now[c] = true;
                    self.stalls += 1;
                }
                self.stall_max = self.stall_max.max(owed);
            } else {
                self.stalled_now[c] = false;
            }
            while self.mix_done[c] < out_seq {
                let m = self.mix_done[c];
                let f0 = start + m * block;
                if f0 + block + BACKLOG_FRAMES < g {
                    // Fell out of the hearable window: these frames
                    // are history now. Drop the backlog and resync at
                    // the live edge instead of mixing the past into
                    // the present — but a block that merely *reached*
                    // the clock edge is exactly what a healthy
                    // lockstep client always publishes, and sums as
                    // the live audio it is.
                    self.mix_done[c] = (g.saturating_sub(start) / block).max(m + 1);
                    self.mixed_from[c] = self.mix_done[c];
                    slot.consumed_out.store(self.mix_done[c], Release);
                    self.dropped += 1;
                    continue;
                }
                if f0 >= self.mixed_upto {
                    break; // beyond the zero frontier — the world isn't there yet
                }
                let len = block.min(MAX_BLOCK as u64) as usize;
                for (j, plane) in self.out_circ.iter_mut().enumerate() {
                    let src = unsafe {
                        std::slice::from_raw_parts(
                            bus.ring_block(c, Dir::Out, m, VIRT_FIRST + j),
                            len,
                        )
                    };
                    for (i, &s) in src.iter().enumerate() {
                        plane[((f0 + i as u64) & CIRC_MASK) as usize] += s;
                    }
                    // Keep what this slot just added: publishing its
                    // own inputs subtracts this snapshot back out.
                    self.own_last[c * VIRT_CH + j][..len].copy_from_slice(src);
                }
                self.mix_done[c] = m + 1;
                slot.consumed_out.store(m + 1, Release);
            }

            // — publish complete input blocks — second pass, so this
            // step's sums are visible to the mix gate below.
        }
        for c in 0..proto::SLOTS {
            let slot = bus.slot(c);
            use Ordering::{Acquire, Release};
            if slot.state.load(Acquire) != SLOT_ACTIVE {
                continue;
            }
            let block = slot.block.load(Acquire).max(1) as u64;
            let start = slot.start_frame.load(Acquire);
            if start == 0 {
                continue;
            }
            // Frames whose mix already contains this slot's own output
            // — the window to subtract back out as we publish, so a
            // client hears every other tenant and never itself.
            let own_lo = start + self.mixed_from[c] * block;
            let own_hi = start + self.mix_done[c] * block;
            while self.in_written[c] < slot.read_in.load(Acquire) + (proto::RINGS as u64 - 1) {
                let n = self.in_written[c];
                let f0 = start + n * block;
                if f0 + block > committed {
                    break; // not backed by the clock yet
                }
                if !self.others_mixed_to(c, f0, g) {
                    // Somebody else's output for these frames is still
                    // in flight; publishing now would write silence
                    // where their audio belongs. The pumps are paced
                    // by this very store, so waiting is self-limiting.
                    break;
                }
                let len = block.min(MAX_BLOCK as u64) as usize;
                for ch in 0..proto::BUS_CH {
                    let dst = unsafe { bus.in_block_mut(c, n, ch) };
                    let dst = &mut dst[..len];
                    if ch < self.hw_in {
                        // Same slice of bus time as the virtual channels
                        // below: one block back from the release edge.
                        // Reading the hardware ring at `[f0, f0+block)`
                        // while the mix is read at `[f0-block, f0)`
                        // would hand a client a mic signal one block
                        // out of phase with everything else on the same
                        // callback — inaudible alone, wrong the moment
                        // the two are summed.
                        let base = f0.saturating_sub(block);
                        if base < g.saturating_sub(CIRC_FRAMES - MAX_BLOCK as u64) {
                            dst.fill(0.0); // fell off the scratch entirely
                        } else {
                            let plane = &self.in_circ[ch];
                            for (i, d) in dst.iter_mut().enumerate() {
                                *d = plane[((base + i as u64) & CIRC_MASK) as usize];
                            }
                        }
                    } else if ch >= VIRT_FIRST {
                        // One block of deliberate cross-client delay:
                        // the mix at [f0 - block, f0), which every
                        // other live slot's consume pass has already
                        // finished summing — see `others_mixed_to`.
                        // Then take back this slot's own share: the
                        // snapshot of its last summed block, subtracted
                        // where the frames line up, so what remains is
                        // every *other* tenant's audio and the device
                        // can be monitored without feeding a host its
                        // own output. Frames older than the snapshot
                        // (a reader catching up after a stall) skip
                        // the subtraction — the ring cells behind it
                        // may already be recycled; a few blocks of
                        // late self-echo while that reader is already
                        // running late is the honest trade.
                        let plane = &self.out_circ[ch - VIRT_FIRST];
                        let own = &self.own_last[c * VIRT_CH + (ch - VIRT_FIRST)];
                        for (i, d) in dst.iter_mut().enumerate() {
                            let f = (f0 + i as u64).saturating_sub(block);
                            let mut v = if f < self.mixed_upto {
                                plane[(f & CIRC_MASK) as usize]
                            } else {
                                0.0
                            };
                            if (own_lo..own_hi).contains(&f) {
                                let off = f - start;
                                if off / block + 1 == self.mix_done[c] {
                                    v -= own[(off % block) as usize];
                                }
                            }
                            *d = v;
                        }
                    } else {
                        // The gap between the hardware's real channels
                        // and the virtual block: no host ever maps a bus
                        // channel here (presentation inputs are the real
                        // lines then the taps), so it reads as silence.
                        dst.fill(0.0);
                    }
                }
                slot.in_seq.store(n + 1, Release);
                self.in_written[c] = n + 1;
                bus.signal_in(c);
            }
        }

        self.bus
            .header()
            .heartbeat_frames
            .store(committed, Ordering::Release);
    }

    /// May slot `c`'s input block starting at frame `f0` be published
    /// yet? The block carries the mix of frames up to `f0` (one block
    /// of deliberate cross-client delay), so every *other* live slot's
    /// published output must have been summed through `f0` — otherwise
    /// the copy below reads cells the senders have not reached and
    /// publishes silence over their audio. Frames before a slot's
    /// `start_frame` need no summing: the zero frontier keeps them
    /// silence, which is the honest reading of a tenant that is not
    /// making sound yet — and at adoption, when every slot starts at
    /// the same stamp, it is exactly what breaks the simultaneous
    /// first-block deadlock. A slot whose mix position has drifted
    /// more than `BACKLOG_FRAMES` behind the live edge is a crashed or
    /// stalled tenant: waiting on it would let one dead client hold the
    /// entire bus hostage, so the wait expires and its cells — still
    /// frontier zeroes, or sums already made — are published as they
    /// stand.
    fn others_mixed_to(&self, for_slot: usize, f0: u64, g: u64) -> bool {
        use Ordering::Acquire;
        for o in 0..proto::SLOTS {
            if o == for_slot {
                continue;
            }
            let slot = self.bus.slot(o);
            if slot.state.load(Acquire) != SLOT_ACTIVE {
                continue;
            }
            let start = slot.start_frame.load(Acquire);
            if start == 0 || f0 <= start {
                continue; // its silent preamble
            }
            let block = slot.block.load(Acquire).max(1) as u64;
            let mixed_pos = start + self.mix_done[o] * block;
            if mixed_pos < f0 && mixed_pos + BACKLOG_FRAMES >= g {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{HW_SAMPLE_BYTES, HwFmt, load_frames, silence_frames, store_frames};

    /// The shape of the bug this module had, written down as bytes.
    ///
    /// A driver that answers `PCM_I32_LSB` wants `0x20000000` for a
    /// quarter-scale sample. A `0.25f32` is `0x3E800000` — the integer
    /// 1 048 576 000, which happens to be about a quarter of full scale,
    /// so the *level* came out right while the *waveform* was a ramp of
    /// exponents that jumped wildly every time a sample went negative.
    /// Zero is the one value both shapes agree on, which is why a device
    /// with this bug plays silence correctly and everything else as fuzz.
    #[test]
    fn int32_is_not_float32_wearing_a_hat() {
        let src = [0.25f32, -0.25, 1.0, 0.0];
        let mut buf = [0i32; 4];
        let peak = unsafe { store_frames(HwFmt::I32, buf.as_mut_ptr().cast(), &src) };
        assert_eq!(peak, 1.0);
        assert_eq!(buf[0], 536_870_912, "a quarter of full scale");
        assert_ne!(
            buf[0],
            f32::to_bits(0.25) as i32,
            "the old bug, asserted absent"
        );
        assert_eq!(buf[1], -536_870_912);
        assert_eq!(buf[2], i32::MAX);
        assert_eq!(buf[3], 0);

        let mut back = [0.0f32; 4];
        unsafe { load_frames(HwFmt::I32, buf.as_ptr().cast(), &mut back) };
        for (a, b) in back.iter().zip(src) {
            assert!((a - b).abs() < 1e-6, "{a} should round-trip {b}");
        }
    }

    /// `2.0f32 as i32` is zero in Rust, and a zero in the middle of a
    /// waveform is a click: over must saturate, never wrap or vanish.
    /// The rails are the two's-complement extremes, which is what a
    /// full-scale square wave is asking for on its two half-cycles.
    #[test]
    fn over_doubles_saturate() {
        let mut buf = [0i32; 4];
        unsafe { store_frames(HwFmt::I32, buf.as_mut_ptr().cast(), &[4.0, -9.0, -0.0, 1.0]) };
        assert_eq!(buf, [i32::MAX, i32::MIN, 0, i32::MAX]);
    }

    /// Float in, float out, untouched — the shape a driver like ReaRoute
    /// asks for, and the reason the conversions are a choice and not a
    /// global.
    #[test]
    fn float32_passes_through_exactly() {
        let src = [0.5f32, -0.125, f32::MIN_POSITIVE];
        let mut buf = [0f32; 3];
        let peak = unsafe { store_frames(HwFmt::F32, buf.as_mut_ptr().cast(), &src) };
        assert_eq!(buf, src);
        assert_eq!(peak, 0.5);
    }

    /// Both shapes agree that zero bytes are silence, which is what lets
    /// one memset answer the format question at teardown.
    #[test]
    fn silence_is_the_same_bytes_either_way() {
        for fmt in [HwFmt::F32, HwFmt::I32] {
            let mut buf = [0xffu8; 8 * HW_SAMPLE_BYTES];
            unsafe { silence_frames(buf.as_mut_ptr().cast(), 8) };
            assert!(buf.iter().all(|&b| b == 0), "{fmt:?}");
            let mut back = [1.0f32; 8];
            unsafe { load_frames(fmt, buf.as_ptr().cast(), &mut back) };
            assert!(back.iter().all(|&s| s == 0.0), "{fmt:?}");
        }
    }
}
