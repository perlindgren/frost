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

/// The thread's GPU, or `None` — with a shout, once per thread — on a
/// machine that cannot open one. A green skip there is a hole, never
/// a pass; hence the warning.
pub(crate) fn machine() -> Option<&'static Machine> {
    MACHINE.with(|cell| {
        *cell.get_or_init(|| {
            let instance = Instance::default();
            let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
            else {
                eprintln!("gpu: no adapter on this machine — every GPU test goes unverified!");
                return None;
            };
            let Ok((device, queue)) =
                block_on(adapter.request_device(&DeviceDescriptor::default()))
            else {
                eprintln!("gpu: no device — every GPU test goes unverified!");
                return None;
            };
            Some(Box::leak(Box::new(Machine {
                device,
                queue,
                _adapter: adapter,
                _instance: instance,
            })))
        })
    })
}
