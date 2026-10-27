//! The GPU every harness in this tree shares, opened once per thread.
//!
//! Opening a wgpu device — instance, adapter enumeration, device
//! request — walks to the same place every time, and under D3D12 the
//! walk is measured in seconds. The harnesses here call their paint
//! helpers two and three times per test and the suite runs tests by
//! the threadful, so the machine is claimed on a thread's first call
//! and kept by that thread: one walk per test thread instead of one
//! per call, per pipeline rebuild, per readback, forever.
//!
//! Leaked on purpose, and the cell stores a bare reference so no
//! destructor ever runs: a wgpu `Device` destroyed during thread-local
//! teardown meets a world already partly gone and panics there, and a
//! panic in thread-local drop aborts the whole process — the failure
//! this cache first shipped with, learned the loud way. The most a run
//! leaks is one machine per test thread, and process exit reclaims
//! every byte of it.

use crate::backend::app::block_on;
use wgpu::{
    Adapter, AdapterInfo, Device, DeviceDescriptor, Instance, Queue, RequestAdapterOptions,
};

/// One thread's GPU, holding every handle the opening road returned.
pub(crate) struct Machine {
    pub(crate) device: Device,
    pub(crate) queue: Queue,
    // Held for the device's sake, never used directly: a device
    // belongs to the instance that registered it, so the instance has
    // to outlive the device — one field-order decision from a rule.
    _adapter: Adapter,
    _instance: Instance,
}

impl Machine {
    /// What the adapter calls itself, for benches that print their
    /// numbers under the GPU's name and for calibration tables that
    /// key on the vendor.
    pub(crate) fn info(&self) -> AdapterInfo {
        self._adapter.get_info()
    }
}

thread_local! {
    static MACHINE: std::cell::OnceCell<Option<&'static Machine>> =
        const { std::cell::OnceCell::new() };
}

/// Wait for the device's queued work in bounded rounds, and give up
/// loudly.
///
/// `wait_indefinitely` is how a wedged driver becomes silence: the
/// worker thread vanishes into a queue that never signals, libtest
/// holds that thread's captured output, and the suite just stops
/// printing — the last line on the console names the last test that
/// finished, never the one standing over the stall. Every readback in
/// the test tree waits through this instead, so a wedged queue reads
/// as a red line naming its caller and the shape of the wedge, and a
/// merely slow machine — debug layers, shader compiles, a tired
/// laptop — finishes inside the budget and never sees the difference.
/// Thirty seconds a round, four rounds: a driver's honest answer on
/// the slowest calibration we hold versus a stall nobody walks away
/// from.
#[track_caller]
pub(crate) fn drain(device: &wgpu::Device) {
    let round = std::time::Duration::from_secs(30);
    for _ in 0..4 {
        match device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(round),
        }) {
            // The waited-for submission completed; the mapping
            // callbacks the caller armed have run by now.
            Ok(_) => return,
            // A round lapsed; another begins. A wedged queue stays
            // wedged through all four — that is the case this names.
            Err(wgpu::PollError::Timeout) => continue,
            Err(err) => panic!("the gpu refused the wait: {err:?}"),
        }
    }
    panic!(
        "the gpu queue stopped signalling within 4x30 s of bounded waits — \
         a driver stall (hybrid graphics asleep, remote session, a TDR that never \
         finished?); --test-threads=1 --nocapture names the test that was waiting"
    );
}

/// Open the thread's GPU on a thread this one can walk away from.
///
/// Adapter enumeration and device creation answer on the driver's
/// schedule, and `block_on` below never returns early — where a
/// driver is wedged (a dGPU behind a laptop mux that fell asleep, a
/// remote session, a TDR that half-completed), the thread asking for
/// a machine disappears with the test on it and no line printed. Two
/// minutes is that driver's honest worst answer on the slowest
/// Windows machines we know, not a wedged one; past it, every GPU
/// test on this thread refuses by name instead of holding the suite
/// silent. The abandoned opener cannot be cancelled — a thread stuck
/// in the driver's code is the driver's to release, and the process
/// will end it — and it leaks nothing this suite would not have
/// leaked anyway: its machine, if it ever builds one, is the same
/// deliberate leak as every other.
fn open_machine_bounded() -> Option<&'static Machine> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawn = std::thread::Builder::new()
        .name("gpu-machine-open".into())
        .spawn(move || {
            let machine = open_machine();
            // Losing the send only means this thread stopped
            // listening — it panicked at the bound, and the machine
            // (if it ever arrives) is the usual deliberate leak.
            let _ = tx.send(machine);
        });
    if spawn.is_err() {
        // No thread to spare: open here, unbounded, and let the
        // driver hold this test too. Refusing to try would be louder
        // but wrong — a thread-starved machine still has a working
        // GPU most of the time.
        return open_machine();
    }
    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(machine) => machine,
        Err(_) => panic!(
            "the display driver never answered the adapter/device request within 120 s — \
             the gpu stack is wedged; this thread will not guess at what the driver is doing"
        ),
    }
}

fn open_machine() -> Option<&'static Machine> {
    let instance = Instance::default();
    let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default())) else {
        eprintln!("gpu: no adapter on this machine — every GPU test goes unverified!");
        return None;
    };
    let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default())) else {
        eprintln!("gpu: no device — every GPU test goes unverified!");
        return None;
    };
    Some(Box::leak(Box::new(Machine {
        device,
        queue,
        _adapter: adapter,
        _instance: instance,
    })))
}

/// The thread's GPU, or `None` — with a shout, once per thread — on a
/// machine that cannot open one. A green skip there is a hole, never
/// a pass; hence the warning. Opening is bounded ([`open_machine_bounded`]):
/// a wedged driver earns a panic, not silence. A panic leaves the
/// cell unfilled, so the next test on this thread tries again —
/// loudly, each time, which is the point.
pub(crate) fn machine() -> Option<&'static Machine> {
    MACHINE.with(|cell| *cell.get_or_init(open_machine_bounded))
}
