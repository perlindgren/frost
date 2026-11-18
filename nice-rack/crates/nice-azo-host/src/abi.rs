//! The ASIO ABI itself: the hand-written replica-interface vtable, the
//! COM class factory, and the legacy C entry points.
//!
//! Two instantiation paths have to work because hosts disagree about
//! which one is legal. Modern hosts (and `azo`) go through
//! `CoCreateInstance` with the driver's CLSID, so the DLL exports
//! `DllGetClassObject` with a minimal `IClassFactory`; older ones dlopen
//! the classic `ASIOCreateDriver`/`ASIOInit` C functions. Both hand out
//! the same object: `Instance` behind a vtable of three `IUnknown` slots
//! followed by the twenty-one ASIO methods, matching the layout every
//! driver on this machine uses — `azo`'s host side, driving the probe,
//! is the second independent implementation that this layout is checked
//! against.
//!
//! The vtable reuses `azo_sys`'s struct definitions (`BufferInfo`,
//! `ChannelInfo`, `Callbacks`) verbatim: one source of truth for the
//! memory layout, so the driver and the host bindings on this machine
//! cannot drift apart by an accidental `#[repr]`.

use std::os::raw::{c_long, c_void};
use std::panic::AssertUnwindSafe;
use std::ptr;

use azo::sys::{Bool, BufferInfo, Callbacks, ChannelInfo, SampleType, Samples, TimeStamp};

use crate::instance::Instance;
use crate::win::Guid;
use crate::{CLSID, DRIVER_NAME};

const S_OK: i32 = 0;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const E_POINTER: i32 = 0x8000_4003u32 as i32;
const CLASS_E_NOAGGREGATION: i32 = 0x8004_0110u32 as i32;
const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154u32 as i32;

/// `IID_IUnknown`, spelled out because this file may not depend on a
/// metadata crate for one constant.
const IID_IUNKNOWN: Guid = Guid {
    data1: 0,
    data2: 0,
    data3: 0,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
/// `IID_IClassFactory`.
const IID_ICLASS_FACTORY: Guid = Guid {
    data1: 1,
    data2: 0,
    data3: 0,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

// ASIO result codes, taken from the same enumeration `azo` maps. Only
// zero is safe as a "worked" answer: hosts are split on whether a
// nonzero return means success, and the common reading is `if (ret)
// error(...)`.
const ASIO_OK: i32 = 0;
const ASIO_HWMALFUNCTION: i32 = -999;
const ASIO_NOT_PRESENT: i32 = -1000;
const ASIO_NO_CLOCK: i32 = -995;

/// `kAsioSelectorSupported`, the one `future` selector we answer.
const SEL_SELECTOR_SUPPORTED: c_long = 0x11;

/// Every vtable slot is guarded: a panic inside our own code must never
/// unwind across a host's frame — the host has no way to survive that,
/// and "the DAW closed silently" is the worst bug report there is. On a
/// panic the method returns its documented failure shape and records
/// the fault for `getErrorMessage`.
macro_rules! guarded {
    ($this:expr, $fallback:expr, $body:expr) => {{
        match std::panic::catch_unwind(AssertUnwindSafe($body)) {
            Ok(value) => value,
            Err(payload) => {
                let reason = payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("panic with no message");
                crate::trace::log(&format!("vtable slot fault: {reason}"));
                note_fault($this);
                $fallback
            }
        }
    }};
}

fn note_fault(this: *mut c_void) {
    if !this.is_null() {
        let obj = unsafe { &*(this as *const AsioObject) };
        *obj.instance().note_public().lock().unwrap() =
            "NiceRack ASIO: internal fault (see niceroute log)".to_string();
    }
}

/// The object handed to hosts: a COM header pointing at the driver
/// vtable, then the instance pointer. `vtbl` first is the entire COM
/// contract; hosts cast the same pointer to their own `ASIODriver*`.
#[repr(C)]
struct AsioObject {
    vtbl: *const AsioVtbl,
    inst: *mut Instance,
}

impl AsioObject {
    fn instance(&self) -> &Instance {
        unsafe { &*self.inst }
    }
}

/// Recover `&Instance` from any vtable slot's `this`.
unsafe fn inst_of(this: *mut c_void) -> &'static Instance {
    unsafe { &*(&*(this as *const AsioObject)).inst }
}

/// The driver interface: `IUnknown`'s three slots, then the twenty-one
/// ASIO methods in declaration order. Return types mirror
/// `IIASIORedecl` exactly — including the void-named `get_driver_name`
/// and the ResultCode-returning `control_panel`, both of which hosts
/// have opinions about.
#[repr(C)]
#[rustfmt::skip]
pub struct AsioVtbl {
    pub query_interface:  unsafe extern "system" fn(this: *mut c_void, iid: *const Guid, obj: *mut *mut c_void) -> i32,
    pub add_ref:          unsafe extern "system" fn(this: *mut c_void) -> u32,
    pub release:          unsafe extern "system" fn(this: *mut c_void) -> u32,
    pub init:             unsafe extern "system" fn(this: *mut c_void, sys_ref: *mut c_void) -> Bool,
    pub get_driver_name:  unsafe extern "system" fn(this: *mut c_void, name: *mut u8),
    pub get_driver_version: unsafe extern "system" fn(this: *mut c_void) -> c_long,
    pub get_error_message: unsafe extern "system" fn(this: *mut c_void, string: *mut u8),
    pub start:            unsafe extern "system" fn(this: *mut c_void) -> i32,
    pub stop:             unsafe extern "system" fn(this: *mut c_void) -> i32,
    pub get_channels:     unsafe extern "system" fn(this: *mut c_void, ins: *mut c_long, outs: *mut c_long) -> i32,
    pub get_latencies:    unsafe extern "system" fn(this: *mut c_void, in_lat: *mut c_long, out_lat: *mut c_long) -> i32,
    pub get_buffer_size:  unsafe extern "system" fn(this: *mut c_void, min: *mut c_long, max: *mut c_long, preferred: *mut c_long, granularity: *mut c_long) -> i32,
    pub can_sample_rate:  unsafe extern "system" fn(this: *mut c_void, rate: f64) -> i32,
    pub get_sample_rate:  unsafe extern "system" fn(this: *mut c_void, rate: *mut f64) -> i32,
    pub set_sample_rate:  unsafe extern "system" fn(this: *mut c_void, rate: f64) -> i32,
    pub get_clock_sources: unsafe extern "system" fn(this: *mut c_void, clocks: *mut azo::sys::ClockSource, num: *mut c_long) -> i32,
    pub set_clock_source: unsafe extern "system" fn(this: *mut c_void, index: c_long) -> i32,
    pub get_sample_position: unsafe extern "system" fn(this: *mut c_void, pos: *mut Samples, stamp: *mut TimeStamp) -> i32,
    pub get_channel_info: unsafe extern "system" fn(this: *mut c_void, info: *mut ChannelInfo) -> i32,
    pub create_buffers:   unsafe extern "system" fn(this: *mut c_void, infos: *mut BufferInfo, num: c_long, size: c_long, callbacks: *mut Callbacks) -> i32,
    pub dispose_buffers:  unsafe extern "system" fn(this: *mut c_void) -> i32,
    pub control_panel:    unsafe extern "system" fn(this: *mut c_void) -> i32,
    pub future:           unsafe extern "system" fn(this: *mut c_void, selector: c_long, opt: *mut c_void) -> i32,
    pub output_ready:     unsafe extern "system" fn(this: *mut c_void) -> i32,
}

unsafe extern "system" fn v_query_interface(
    this: *mut c_void,
    iid: *const Guid,
    obj: *mut *mut c_void,
) -> i32 {
    guarded!(this, E_POINTER, || {
        if obj.is_null() {
            return E_POINTER;
        }
        // The CLSID doubles as the interface id — accept ourselves and
        // `IUnknown`, decline everything else without aggregation drama.
        let want = unsafe { *iid };
        if want == IID_IUNKNOWN || want == CLSID {
            unsafe { *obj = this };
            unsafe { inst_of(this) }.add_ref();
            S_OK
        } else {
            crate::trace::log(&format!("queryInterface: refused {want:?}"));
            unsafe { *obj = ptr::null_mut() };
            E_NOINTERFACE
        }
    })
}

unsafe extern "system" fn v_add_ref(this: *mut c_void) -> u32 {
    guarded!(this, 1, || unsafe { inst_of(this) }.add_ref())
}

unsafe extern "system" fn v_release(this: *mut c_void) -> u32 {
    guarded!(this, 0, || {
        let last = unsafe { inst_of(this) }.release();
        if last {
            unsafe {
                drop(Box::from_raw((*this.cast::<AsioObject>()).inst));
                drop(Box::from_raw(this.cast::<AsioObject>()));
            }
        }
        0
    })
}

unsafe extern "system" fn v_init(this: *mut c_void, _sys_ref: *mut c_void) -> Bool {
    let ok = guarded!(this, false, || unsafe { inst_of(this) }.init());
    Bool(i32::from(ok))
}

unsafe extern "system" fn v_get_driver_name(this: *mut c_void, name: *mut u8) {
    guarded!(this, (), || {
        if name.is_null() {
            return;
        }
        // ASIO's name buffer is 32 bytes including the terminator.
        let bytes = DRIVER_NAME.as_bytes();
        let len = bytes.len().min(31);
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), name, len) };
        unsafe { *name.add(len) = 0 };
    })
}

unsafe extern "system" fn v_get_driver_version(this: *mut c_void) -> c_long {
    guarded!(this, 1, || 1)
}

unsafe extern "system" fn v_get_error_message(this: *mut c_void, string: *mut u8) {
    guarded!(this, (), || {
        if string.is_null() {
            return;
        }
        let msg = unsafe { inst_of(this) }.error_message();
        let bytes = msg.as_bytes();
        let len = bytes.len().min(123);
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), string, len) };
        unsafe { *string.add(len) = 0 };
    })
}

unsafe extern "system" fn v_start(this: *mut c_void) -> i32 {
    crate::trace::log("start: called");
    guarded!(
        this,
        ASIO_NOT_PRESENT,
        || match unsafe { inst_of(this) }.start() {
            Ok(()) => {
                crate::trace::log("start: ok");
                // `ASE_OK`, never `ASE_SUCCESS`: the spec calls both
                // success, but hosts are split on the test, and the common
                // one is `if (start()) error(...)`. A host reading a huge
                // positive number as failure is how a working device earns
                // "Error starting ASIO" — zero is the answer nobody
                // misreads.
                ASIO_OK
            }
            Err(e) => {
                crate::trace::log(&format!("start: refused: {e}"));
                *unsafe { inst_of(this) }.note_public().lock().unwrap() =
                    format!("NiceRack ASIO: start: {e}");
                ASIO_HWMALFUNCTION
            }
        }
    )
}

unsafe extern "system" fn v_stop(this: *mut c_void) -> i32 {
    crate::trace::log("stop: called");
    guarded!(this, ASIO_OK, || {
        unsafe { inst_of(this) }.stop();
        ASIO_OK
    })
}

unsafe extern "system" fn v_get_channels(
    this: *mut c_void,
    ins: *mut c_long,
    outs: *mut c_long,
) -> i32 {
    guarded!(this, ASIO_NOT_PRESENT, || {
        let (i, o) = unsafe { inst_of(this) }.channel_counts();
        unsafe { *ins = i };
        unsafe { *outs = o };
        ASIO_OK
    })
}

unsafe extern "system" fn v_get_latencies(
    this: *mut c_void,
    in_lat: *mut c_long,
    out_lat: *mut c_long,
) -> i32 {
    guarded!(this, ASIO_NOT_PRESENT, || {
        let block = unsafe { inst_of(this) }.block_public() as c_long;
        // Two blocks each way, and the arithmetic is the truth: one
        // block is the double buffer every ASIO round trip costs, the
        // second is the bus relay — the mix for a block of frames is
        // finished at the edge those frames end at, and both the
        // published inputs and the DAC read that finished block
        // (`hardware_tick`, `step`'s publish pass).
        unsafe { *in_lat = 2 * block };
        unsafe { *out_lat = 2 * block };
        ASIO_OK
    })
}

unsafe extern "system" fn v_get_buffer_size(
    this: *mut c_void,
    min: *mut c_long,
    max: *mut c_long,
    preferred: *mut c_long,
    granularity: *mut c_long,
) -> i32 {
    guarded!(this, ASIO_NOT_PRESENT, || {
        let (a, b, c, d) = unsafe { inst_of(this) }.buffer_size();
        unsafe { *min = a };
        unsafe { *max = b };
        unsafe { *preferred = c };
        unsafe { *granularity = d };
        ASIO_OK
    })
}

unsafe extern "system" fn v_can_sample_rate(this: *mut c_void, rate: f64) -> i32 {
    guarded!(this, ASIO_NO_CLOCK, || {
        if unsafe { inst_of(this) }.set_sample_rate(rate) {
            ASIO_OK
        } else {
            let have = unsafe { inst_of(this) }.header_rate();
            crate::trace::log(&format!("canSampleRate: {rate} refused (bus is {have})"));
            ASIO_NO_CLOCK
        }
    })
}

unsafe extern "system" fn v_get_sample_rate(this: *mut c_void, rate: *mut f64) -> i32 {
    guarded!(this, ASIO_NOT_PRESENT, || {
        unsafe { *rate = inst_of(this).header_rate() as f64 };
        ASIO_OK
    })
}

unsafe extern "system" fn v_set_sample_rate(this: *mut c_void, rate: f64) -> i32 {
    guarded!(this, ASIO_NO_CLOCK, || {
        // The device adopts the master's clock; it never changes the
        // hardware's rate underneath the master.
        if unsafe { inst_of(this) }.set_sample_rate(rate) {
            ASIO_OK
        } else {
            let have = unsafe { inst_of(this) }.header_rate();
            crate::trace::log(&format!("setSampleRate: {rate} refused (bus is {have})"));
            ASIO_NO_CLOCK
        }
    })
}

unsafe extern "system" fn v_get_clock_sources(
    _this: *mut c_void,
    _clocks: *mut azo::sys::ClockSource,
    _num: *mut c_long,
) -> i32 {
    // No clock-source menu: there is exactly one clock, and the
    // heartbeat already tells the truth about it.
    ASIO_NOT_PRESENT
}

unsafe extern "system" fn v_set_clock_source(_this: *mut c_void, _index: c_long) -> i32 {
    ASIO_NOT_PRESENT
}

unsafe extern "system" fn v_get_sample_position(
    this: *mut c_void,
    pos: *mut Samples,
    stamp: *mut TimeStamp,
) -> i32 {
    guarded!(this, ASIO_NOT_PRESENT, || {
        let frames = unsafe { inst_of(this) }
            .heartbeat()
            .try_into()
            .unwrap_or(i64::MAX);
        unsafe { pos.write(frames.into()) };
        unsafe { stamp.write(0i64.into()) };
        ASIO_OK
    })
}

unsafe extern "system" fn v_get_channel_info(this: *mut c_void, info: *mut ChannelInfo) -> i32 {
    guarded!(this, ASIO_NOT_PRESENT, || {
        let inst = unsafe { inst_of(this) };
        if info.is_null() {
            return ASIO_NOT_PRESENT;
        }
        let info_ref = unsafe { &mut *info };
        let is_input = info_ref.is_input.0 != 0;
        let Some(name) = inst.channel_name(info_ref.channel, is_input) else {
            return ASIO_NOT_PRESENT;
        };
        info_ref.is_active = Bool::TRUE;
        info_ref.channel_group = 0;
        info_ref.sample_type = SampleType::PCM_F32_LSB;
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        info_ref.name = [0u8; 32];
        info_ref.name[..len].copy_from_slice(&bytes[..len]);
        ASIO_OK
    })
}

unsafe extern "system" fn v_create_buffers(
    this: *mut c_void,
    infos: *mut BufferInfo,
    num: c_long,
    size: c_long,
    callbacks: *mut Callbacks,
) -> i32 {
    guarded!(this, ASIO_NOT_PRESENT, || {
        match unsafe { inst_of(this).create_buffers(infos, num, size, callbacks) } {
            Ok(()) => ASIO_OK,
            Err(e) => {
                *unsafe { inst_of(this) }.note_public().lock().unwrap() =
                    format!("NiceRack ASIO: {e}");
                ASIO_NOT_PRESENT
            }
        }
    })
}

unsafe extern "system" fn v_dispose_buffers(this: *mut c_void) -> i32 {
    guarded!(this, ASIO_OK, || {
        unsafe { inst_of(this) }.dispose_buffers();
        ASIO_OK
    })
}

unsafe extern "system" fn v_control_panel(this: *mut c_void) -> i32 {
    guarded!(this, ASIO_OK, || {
        // A driver with no settings has nothing to open; the status
        // line already says everything a panel would.
        let _ = unsafe { inst_of(this) };
        ASIO_OK
    })
}

unsafe extern "system" fn v_future(this: *mut c_void, selector: c_long, opt: *mut c_void) -> i32 {
    crate::trace::log(&format!("future: selector {selector} (0x{selector:x})"));
    let _ = this;
    match selector {
        // `kAsioSelectorSupported` (0x11): the host asks whether we
        // implement the selector it names in `opt`. We implement none
        // beyond this one, and the answer goes in the long it pointed
        // at — per the ASIO 2.0 convention.
        SEL_SELECTOR_SUPPORTED => {
            if !opt.is_null() {
                unsafe { *(opt as *mut c_long) = 0 };
            }
            ASIO_OK
        }
        // Time info and IO format: honestly not present. Hosts fall
        // back to `getSamplePosition` and never ask again.
        _ => ASIO_NOT_PRESENT,
    }
}

unsafe extern "system" fn v_output_ready(this: *mut c_void) -> i32 {
    guarded!(this, ASIO_OK, || {
        // The pump consumed every block before returning from
        // `bufferSwitch`; readiness is the state of the world.
        let _ = unsafe { inst_of(this) };
        ASIO_OK
    })
}

static VTABLE: AsioVtbl = AsioVtbl {
    query_interface: v_query_interface,
    add_ref: v_add_ref,
    release: v_release,
    init: v_init,
    get_driver_name: v_get_driver_name,
    get_driver_version: v_get_driver_version,
    get_error_message: v_get_error_message,
    start: v_start,
    stop: v_stop,
    get_channels: v_get_channels,
    get_latencies: v_get_latencies,
    get_buffer_size: v_get_buffer_size,
    can_sample_rate: v_can_sample_rate,
    get_sample_rate: v_get_sample_rate,
    set_sample_rate: v_set_sample_rate,
    get_clock_sources: v_get_clock_sources,
    set_clock_source: v_set_clock_source,
    get_sample_position: v_get_sample_position,
    get_channel_info: v_get_channel_info,
    create_buffers: v_create_buffers,
    dispose_buffers: v_dispose_buffers,
    control_panel: v_control_panel,
    future: v_future,
    output_ready: v_output_ready,
};

fn new_object() -> *mut AsioObject {
    Box::into_raw(Box::new(AsioObject {
        vtbl: &VTABLE,
        inst: Box::into_raw(Box::new(Instance::new())),
    }))
}

// ---------------------------------------------------------- COM factory

#[repr(C)]
struct FactoryVtbl {
    query_interface: unsafe extern "system" fn(
        this: *mut c_void,
        iid: *const Guid,
        obj: *mut *mut c_void,
    ) -> i32,
    add_ref: unsafe extern "system" fn(this: *mut c_void) -> u32,
    release: unsafe extern "system" fn(this: *mut c_void) -> u32,
    create_instance: unsafe extern "system" fn(
        this: *mut c_void,
        outer: *mut c_void,
        iid: *const Guid,
        obj: *mut *mut c_void,
    ) -> i32,
    lock_server: unsafe extern "system" fn(this: *mut c_void, lock: i32) -> i32,
}

/// The factory is immortal, which is exactly what `DllCanUnloadNow`
/// admits: the DLL hosts threads and a shared mapping and does not
/// consent to unloading until the process ends.
static FACTORY_VTBL: FactoryVtbl = FactoryVtbl {
    query_interface: f_query_interface,
    add_ref: f_add_ref,
    release: f_release,
    create_instance: f_create_instance,
    lock_server: f_lock_server,
};

#[repr(C)]
struct Factory {
    vtbl: *const FactoryVtbl,
}

// The vtable pointer names a `static`, which is a process-wide constant;
// sharing it across threads is the entire idea.
unsafe impl Sync for Factory {}

static FACTORY: Factory = Factory {
    vtbl: &FACTORY_VTBL,
};

unsafe extern "system" fn f_query_interface(
    this: *mut c_void,
    iid: *const Guid,
    obj: *mut *mut c_void,
) -> i32 {
    if obj.is_null() {
        return E_POINTER;
    }
    let want = unsafe { *iid };
    if want == IID_IUNKNOWN || want == IID_ICLASS_FACTORY {
        unsafe { *obj = this };
        S_OK
    } else {
        unsafe { *obj = ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn f_add_ref(_this: *mut c_void) -> u32 {
    2
}

unsafe extern "system" fn f_release(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn f_create_instance(
    _this: *mut c_void,
    outer: *mut c_void,
    iid: *const Guid,
    obj: *mut *mut c_void,
) -> i32 {
    if !outer.is_null() {
        return CLASS_E_NOAGGREGATION;
    }
    if obj.is_null() {
        return E_POINTER;
    }
    let object = new_object() as *mut c_void;
    // The object's own QueryInterface, not the factory's: the caller's
    // iid is the replica interface id (the CLSID again), which only the
    // driver object answers — and its QI is what takes the first
    // reference, as COM prescribes.
    unsafe { v_query_interface(object, iid, obj) };
    if unsafe { *obj }.is_null() {
        // Nobody wanted it, and the QI only takes its reference on
        // success — the object is still unreferenced as born, so it is
        // freed as such: releasing an unowned reference would underflow
        // the counter rather than undo the birth.
        unsafe {
            drop(Box::from_raw((*object.cast::<AsioObject>()).inst));
            drop(Box::from_raw(object.cast::<AsioObject>()));
        }
        return E_NOINTERFACE;
    }
    S_OK
}

unsafe extern "system" fn f_lock_server(lock: *mut c_void, _lock_flag: i32) -> i32 {
    let _ = lock;
    S_OK
}

// ----------------------------------------------------------- exports

/// COM's DLL entry point: hands out the class factory for our CLSID.
///
/// # Safety
/// Called by the OS loader/COM with valid, non-null `rclsid`/`riid`,
/// and a writable `ppv` — the guarantees every `DllGetClassObject`
/// caller makes by contract.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const Guid,
    riid: *const Guid,
    ppv: *mut *mut c_void,
) -> i32 {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = ptr::null_mut() };
    crate::trace::log("DllGetClassObject");
    if unsafe { *rclsid } != CLSID {
        return REGDB_E_CLASSNOTREG;
    }
    unsafe { f_query_interface(&FACTORY as *const Factory as *mut c_void, riid, ppv) }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> i32 {
    1 // S_FALSE: never.
}

// The legacy C interface, for hosts that dlopen instead of CoCreate.
// `ASIOCreateDriver` compares the requested class id and hands out the
// same object the factory does.

#[unsafe(no_mangle)]
pub extern "system" fn ASIOGetVersion(_future: *mut c_void) -> c_long {
    2
}

/// # Safety
/// ASIO's C entry point; `sys_ref` may be null or a host window handle,
/// neither of which is dereferenced.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ASIOInit(_sys_ref: *mut c_void) -> c_long {
    // The C interface expects a default driver object; make sure the
    // bus is open before reporting readiness.
    ASIO_DEFAULT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let object = new_object();
            let ok = unsafe { v_init(object as *mut c_void, ptr::null_mut()) };
            if ok.0 != 0 {
                unsafe { inst_of(object as *mut c_void) }.add_ref(); // the slot holds one
                *slot = Some(object);
            } else {
                unsafe { v_release(object as *mut c_void) };
            }
        }
        c_long::from(slot.is_some())
    })
}

thread_local! {
    static ASIO_DEFAULT: std::cell::RefCell<Option<*mut AsioObject>> = const { std::cell::RefCell::new(None) };
}

#[unsafe(no_mangle)]
pub extern "system" fn ASIOExit() {
    ASIO_DEFAULT.with(|slot| {
        if let Some(object) = slot.borrow_mut().take() {
            unsafe { v_release(object as *mut c_void) };
        }
    });
}

/// # Safety
/// `class_id`, if non-null, must point to a readable `Guid`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ASIODriverExists(class_id: *const Guid) -> u8 {
    u8::from(!class_id.is_null() && unsafe { *class_id } == CLSID)
}

/// # Safety
/// `driver`, if non-null, must be a writable slot for one pointer; the
/// caller owns the returned object reference.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ASIOCreateDriver(
    class_id: *const Guid,
    driver: *mut *mut c_void,
) -> i32 {
    if driver.is_null() {
        return E_POINTER;
    }
    if class_id.is_null() || unsafe { *class_id } != CLSID {
        unsafe { *driver = ptr::null_mut() };
        return REGDB_E_CLASSNOTREG;
    }
    let object = new_object() as *mut c_void;
    unsafe { inst_of(object) }.add_ref(); // the returned pointer holds one reference
    unsafe { *driver = object };
    S_OK
}
