//! One ASIO connection: the object a host holds, the buffers it was
//! given, its seat on the bus, and the pump thread that feeds it.
//!
//! An instance is a *client* of the bus by default; the only thing that
//! distinguishes the one instance that also runs the master role is
//! that it won the election during `init` — and if the master dies
//! mid-session, whichever client notices first through the heartbeat
//! can promote itself without anybody's host knowing anything changed.

use std::os::raw::c_void;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::bus::{Bus, SharedBus};
use crate::master::Master;
use crate::proto::{self, MAX_BLOCK, MIN_BLOCK, VIRT_CH, VIRT_FIRST};
use crate::win::MasterClaim;
use azo::sys::{Bool, Callbacks};

pub(crate) const NO_SLOT: u32 = u32::MAX;

/// The buffers `create_buffers` handed the host. The boxes own them;
/// the raw copies are for the audio path. Pointers stay valid until
/// `dispose_buffers`, which joins the pump *first* — that join is what
/// makes handing `*const Instance` to the pump thread legal.
struct Bufs {
    in_half: Vec<[*mut f32; 2]>,
    out_half: Vec<[*mut f32; 2]>,
    /// Read nowhere, dropped at `dispose_buffers`: this is what keeps
    /// the raw pointers above alive while the host holds them.
    #[allow(dead_code)]
    owned: Vec<Box<[f32]>>,
}

pub(crate) struct Instance {
    refs: AtomicU32,
    bus: Mutex<Option<SharedBus>>,
    /// Last diagnostic for `getErrorMessage` — what the master decided,
    /// which driver it hosts, or why it does not.
    note: Arc<Mutex<String>>,
    // Declared before `claim` on purpose: on drop the clock must stop
    // before the seat is surrendered, or the next master inherits a
    // half-stopped engine.
    master: Mutex<Option<Master>>,
    claim: Mutex<Option<MasterClaim>>,
    slot: AtomicU32,
    block: AtomicU32,
    opened_in: Mutex<Vec<i32>>,
    opened_out: Mutex<Vec<i32>>,
    bufs: Mutex<Option<Bufs>>,
    callbacks: AtomicPtr<Callbacks>,
    running: Arc<std::sync::atomic::AtomicBool>,
    pump: Mutex<Option<JoinHandle<()>>>,
}

impl Instance {
    pub(crate) fn new() -> Self {
        Instance {
            // Born unreferenced: the QI in the class factory is what
            // takes the first reference, as COM prescribes.
            refs: AtomicU32::new(0),
            bus: Mutex::new(None),
            note: Arc::new(Mutex::new(String::new())),
            master: Mutex::new(None),
            claim: Mutex::new(None),
            slot: AtomicU32::new(NO_SLOT),
            block: AtomicU32::new(128),
            opened_in: Mutex::new(Vec::new()),
            opened_out: Mutex::new(Vec::new()),
            bufs: Mutex::new(None),
            callbacks: AtomicPtr::new(ptr::null_mut()),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pump: Mutex::new(None),
        }
    }

    pub(crate) fn add_ref(&self) -> u32 {
        self.refs.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Shared diagnostic channel for the vtable layer's error notes.
    pub(crate) fn note_public(&self) -> &Mutex<String> {
        &self.note
    }

    pub(crate) fn block_public(&self) -> u32 {
        self.block.load(Ordering::Relaxed)
    }

    pub(crate) fn heartbeat(&self) -> u64 {
        self.bus()
            .map(|b| b.header().heartbeat_frames.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Returns true when this was the last reference and the object may
    /// be freed.
    pub(crate) fn release(&self) -> bool {
        self.refs.fetch_sub(1, Ordering::AcqRel) == 1
    }

    fn bus(&self) -> Option<SharedBus> {
        self.bus.lock().unwrap().clone()
    }

    pub(crate) fn header_rate(&self) -> u32 {
        self.bus()
            .map(|b| b.header().rate_hz.load(Ordering::Relaxed))
            .filter(|&r| r >= 8000)
            .unwrap_or(48000)
    }

    fn header_hw_in(&self) -> u32 {
        self.bus()
            .map(|b| b.header().hw_in.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    // ------------------------------------------------------------ vtable

    pub(crate) fn init(&self) -> bool {
        let bus = match Bus::acquire() {
            Ok(b) => b,
            Err(e) => {
                // No bus means no clock and no audio, ever, for this
                // connection. Staying "open" to look friendly only
                // moves the failure later — the host then creates
                // buffers it will never hear from and reports it as a
                // start-up fault. `false` here is the honest answer:
                // hosts call it a failed open, which is exactly what
                // happened, and `getErrorMessage` says why.
                crate::trace::log(&format!("init: bus failed: {e}"));
                let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
                    " — a bus created by a program running as administrator cannot be joined from a normal process"
                } else {
                    ""
                };
                *self.note.lock().unwrap() = format!("NiceRack ASIO: {e}{hint}");
                return false;
            }
        };
        *self.bus.lock().unwrap() = Some(bus.clone());
        if let Some(claim) = bus.try_become_master() {
            crate::trace::log("init: elected master");
            *self.claim.lock().unwrap() = Some(claim);
            *self.master.lock().unwrap() = Some(Master::start(bus.clone(), self.note.clone()));
        } else {
            crate::trace::log("init: joined existing bus as client");
        }
        true
    }

    pub(crate) fn error_message(&self) -> String {
        let mut s = self.note.lock().unwrap().clone();
        if s.is_empty() {
            let bus = self.bus();
            let master = bus
                .as_ref()
                .map(|b| b.header().master_pid.load(Ordering::Relaxed))
                .unwrap_or(0);
            s = match master {
                0 => "NiceRack ASIO: virtual-only, no master yet".into(),
                pid => format!(
                    "NiceRack ASIO: bus live, master pid {pid}, {} Hz",
                    self.header_rate()
                ),
            };
        }
        s
    }

    /// (inputs, outputs) as the host should see them — always real
    /// numbers, never the "0 and 0 until somebody calls back" answer
    /// that breaks hosts.
    pub(crate) fn channel_counts(&self) -> (i32, i32) {
        (self.header_hw_in() as i32 + VIRT_CH as i32, VIRT_CH as i32)
    }

    pub(crate) fn buffer_size(&self) -> (i32, i32, i32, i32) {
        (
            MIN_BLOCK as i32,
            MAX_BLOCK as i32,
            128,
            -1, // powers of two, per the spec's granularity convention
        )
    }

    pub(crate) fn set_sample_rate(&self, rate: f64) -> bool {
        (f64::from(self.header_rate()) - rate).abs() < 0.5
    }

    pub(crate) fn channel_name(&self, channel: i32, is_input: bool) -> Option<String> {
        let hw_in = self.header_hw_in() as i32;
        if is_input {
            if channel < 0 || channel >= hw_in + VIRT_CH as i32 {
                return None;
            }
            Some(if channel < hw_in {
                format!("Line {}", channel + 1)
            } else {
                format!("Nice {}", channel - hw_in + 1)
            })
        } else {
            if channel < 0 || channel >= VIRT_CH as i32 {
                return None;
            }
            Some(format!("Nice Out {}", channel + 1))
        }
    }

    /// ASIO's channel-to-bus map: presentation inputs are the hardware
    /// lines then the virtual taps; presentation outputs *are* the
    /// virtual channels.
    fn bus_channel_in(&self, id: i32) -> usize {
        let hw_in = self.header_hw_in() as i32;
        if id < hw_in {
            id as usize
        } else {
            VIRT_FIRST + (id - hw_in) as usize
        }
    }

    /// Fills `infos` (the host's array) with buffer pointers and
    /// records the channel map. `infos` entries are the packed
    /// `BufferInfo` the host allocated; we write only the `buffers`
    /// field of each.
    ///
    /// # Safety
    /// `infos` must point to `count` valid, initialized `BufferInfo`s.
    pub(crate) unsafe fn create_buffers(
        &self,
        infos: *mut azo::sys::BufferInfo,
        count: i32,
        block: i32,
        callbacks: *mut Callbacks,
    ) -> Result<(), String> {
        if !(MIN_BLOCK as i32..=MAX_BLOCK as i32).contains(&block) {
            crate::trace::log(&format!("createBuffers: refused block {block}"));
            return Err(format!(
                "buffer size {block} outside {}..{MAX_BLOCK}",
                MIN_BLOCK
            ));
        }
        if self.running.load(Ordering::Relaxed) {
            crate::trace::log("createBuffers: refused, already running");
            return Err("cannot create buffers while running".into());
        }
        crate::trace::log(&format!("createBuffers: {count} channels, block {block}"));
        let mut ins = self.opened_in.lock().unwrap();
        let mut outs = self.opened_out.lock().unwrap();
        ins.clear();
        outs.clear();
        let mut in_half = Vec::new();
        let mut out_half = Vec::new();
        let mut owned = Vec::new();
        for k in 0..count.max(0) as usize {
            let info = unsafe { infos.add(k) };
            let is_input = unsafe { (*info).is_input.0 != 0 };
            let id = unsafe { (*info).channel_num };
            let mut pair = [ptr::null_mut::<f32>(); 2];
            for half in pair.iter_mut() {
                let mut b = vec![0.0f32; block as usize].into_boxed_slice();
                *half = b.as_mut_ptr();
                owned.push(b);
            }
            if is_input {
                ins.push(id);
                in_half.push(pair);
            } else {
                outs.push(id);
                out_half.push(pair);
            }
            // `BufferInfo` is `packed(4)`; write through a raw place to
            // dodge the unaligned-reference rules without lying about it.
            unsafe {
                std::ptr::addr_of_mut!((*info).buffers)
                    .write([pair[0] as *mut c_void, pair[1] as *mut c_void]);
            }
        }
        self.block.store(block as u32, Ordering::Relaxed);
        *self.bufs.lock().unwrap() = Some(Bufs {
            in_half,
            out_half,
            owned,
        });
        self.callbacks.store(callbacks, Ordering::Relaxed);
        Ok(())
    }

    pub(crate) fn dispose_buffers(&self) {
        self.stop_inner();
        if let Some(bus) = self.bus() {
            let slot = self.slot.swap(NO_SLOT, Ordering::SeqCst);
            if slot != NO_SLOT {
                bus.release_slot(slot as usize);
            }
        }
        *self.bufs.lock().unwrap() = None;
        self.opened_in.lock().unwrap().clear();
        self.opened_out.lock().unwrap().clear();
        self.callbacks.store(ptr::null_mut(), Ordering::Relaxed);
    }

    pub(crate) fn start(&self) -> Result<(), String> {
        let bus = match self.bus() {
            Some(b) => b,
            None => {
                crate::trace::log("start: refused, never initialized");
                return Err("device not initialized".into());
            }
        };
        if self.bufs.lock().unwrap().is_none() {
            crate::trace::log("start: refused, no buffers");
            return Err("no buffers".into());
        }
        if self.slot.load(Ordering::Relaxed) == NO_SLOT {
            let slot = match bus.claim_slot() {
                Some(s) => s,
                None => {
                    crate::trace::log("start: refused, all seats taken");
                    return Err("all NiceRack ASIO seats are taken".into());
                }
            };
            crate::trace::log(&format!("start: claimed slot {slot}"));
            bus.slot(slot)
                .block
                .store(self.block.load(Ordering::Relaxed), Ordering::Release);
            self.slot.store(slot as u32, Ordering::SeqCst);
        }
        let slot = self.slot.load(Ordering::Relaxed) as usize;
        self.running.store(true, Ordering::SeqCst);
        // Soundness of the raw pointer: every teardown path
        // (`stop_inner`, `dispose_buffers`) clears `running` and joins
        // this thread before buffers or the instance can go away.
        let send = SendInst(self);
        let thread_bus = bus.clone();
        let running = self.running.clone();
        let join = std::thread::Builder::new()
            .name("niceroute-pump".into())
            // `pump` takes the courier by value and unwraps it inside:
            // precise capture must never touch the raw field directly.
            .spawn(move || unsafe { pump(send, thread_bus, slot, running) })
            .map_err(|e| e.to_string())?;
        *self.pump.lock().unwrap() = Some(join);
        Ok(())
    }

    pub(crate) fn stop(&self) {
        self.stop_inner();
    }

    fn stop_inner(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(join) = self.pump.lock().unwrap().take() {
            let _ = join.join();
        }
    }

    /// A client that watched the heartbeat stop tries to take over the
    /// clock in place; success is invisible to its host.
    fn try_takeover(&self, bus: &SharedBus) {
        let mut claim = self.claim.lock().unwrap();
        if claim.is_some() {
            return;
        }
        if let Some(new_claim) = bus.try_become_master() {
            *claim = Some(new_claim);
            drop(claim);
            *self.master.lock().unwrap() = Some(Master::start(bus.clone(), self.note.clone()));
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stop_inner();
        if let Some(bus) = self.bus.lock().unwrap().take() {
            let slot = self.slot.swap(NO_SLOT, Ordering::SeqCst);
            if slot != NO_SLOT {
                bus.release_slot(slot as usize);
            }
        }
        // Field drop order frees the clock before the claim: `master`
        // is declared first, and struct fields drop in declaration
        // order, which is the abdication sequence.
    }
}

/// The client half of the protocol, end to end. Borrows `inst` through
/// a raw pointer for exactly as long as `running` says — which the
/// instance guarantees by joining before touching anything this reads.
unsafe fn pump(
    me: SendInst,
    bus: SharedBus,
    slot: usize,
    running: Arc<std::sync::atomic::AtomicBool>,
) {
    let SendInst(inst) = me;
    let inst = unsafe { &*inst };
    // This thread runs the *host's* `bufferSwitch` — for REAPER that is
    // its whole graph, plugins and all. Left at normal priority it is
    // scheduled like a text editor, and a block deadline loses that
    // argument to any disk flush; the bus then has to drop the late
    // block, which is what "distorted" is made of.
    let _rt = crate::win::real_time("Pro Audio");
    crate::trace::log(&format!("start: pump on slot {slot}, priority {}", _rt.how));
    let block = inst.block.load(Ordering::Relaxed) as usize;
    let opened_in = inst.opened_in.lock().unwrap().clone();
    let opened_out = inst.opened_out.lock().unwrap().clone();
    let bus_in: Vec<usize> = opened_in
        .iter()
        .map(|&id| inst.bus_channel_in(id))
        .collect();
    let (in_ptrs, out_ptrs) = match inst.bufs.lock().unwrap().as_ref() {
        Some(b) => (b.in_half.clone(), b.out_half.clone()),
        None => return,
    };
    let cb = inst.callbacks.load(Ordering::Relaxed);
    if cb.is_null() {
        return;
    }
    // The packed `Callbacks` is copied out so we never dereference a
    // host pointer mid-callback.
    let cb = unsafe { ptr::read_unaligned(cb) };

    let mut n_read: u64 = 0;
    let mut last_hb = bus.header().heartbeat_frames.load(Ordering::Relaxed);
    let mut hb_at = crate::win::now();

    while running.load(Ordering::Relaxed) {
        bus.wait_in(slot, 20);
        let hb = bus.header().heartbeat_frames.load(Ordering::Relaxed);
        if hb != last_hb {
            last_hb = hb;
            hb_at = crate::win::now();
        } else if hb_at.elapsed_ms() > 3000 {
            // The clock stopped. Try to own it; either way, stop
            // hammering until there is news.
            inst.try_takeover(&bus);
            hb_at = crate::win::now();
        }

        let slot_ref = bus.slot(slot);
        use Ordering::{Acquire, Release};
        while running.load(Ordering::Relaxed) && slot_ref.in_seq.load(Acquire) > n_read {
            let half = (n_read & 1) as usize;
            // Inputs: our slice of the published block into each host
            // buffer half, then the host fills its outputs underneath.
            for (k, &bus_ch) in bus_in.iter().enumerate() {
                let src = unsafe {
                    std::slice::from_raw_parts(
                        bus.ring_block(slot, crate::bus::Dir::In, n_read, bus_ch),
                        block,
                    )
                };
                let dst = unsafe { std::slice::from_raw_parts_mut(in_ptrs[k][half], block) };
                dst.copy_from_slice(src);
            }
            unsafe { (cb.buffer_switch)(half as i32, Bool(1)) };

            // Outputs: the half the host just filled goes to our ring
            // slot — if the master has freed the position. Skip
            // publishes it: a missing contribution means silence for
            // those frames, never a gap in the timeline.
            let gate = slot_ref.consumed_out.load(Acquire) + (proto::RINGS as u64 - 1);
            if n_read < gate {
                for (j, &pres_id) in opened_out.iter().enumerate() {
                    let bus_ch = VIRT_FIRST + pres_id as usize;
                    let src = unsafe { std::slice::from_raw_parts(out_ptrs[j][half], block) };
                    let dst =
                        unsafe { bus.ring_block_mut(slot, crate::bus::Dir::Out, n_read, bus_ch) };
                    dst[..block].copy_from_slice(src);
                }
                slot_ref.out_seq.store(n_read + 1, Release);
            }
            slot_ref.read_in.store(n_read + 1, Release);
            n_read += 1;
        }
    }
}

/// Raw-pointer courier for the pump thread; legality is `Instance`'s
/// join-before-teardown contract, stated at [`Instance::start`].
struct SendInst(*const Instance);
unsafe impl Send for SendInst {}
