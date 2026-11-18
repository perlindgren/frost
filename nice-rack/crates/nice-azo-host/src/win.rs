//! The handful of Win32 calls this driver needs, declared the way the
//! rest of the workspace declares them — raw `extern` blocks, no
//! metadata crate — wrapped in types whose ownership story makes the
//! protocol mistakes impossible: a mapping that unmaps itself, an event
//! that cannot be waited on after close, a master claim that releases
//! the election mutex exactly once, by dropping.

// The raw Win32 type aliases mirror the platform headers; renaming them
// to `Handle`/`Dword` would make the FFI signatures harder to read.
#![allow(clippy::upper_case_acronyms)]

use std::ffi::{CString, OsString};
use std::os::windows::ffi::OsStringExt;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

pub type HANDLE = *mut core::ffi::c_void;
pub type HKEY = *mut core::ffi::c_void;
pub type DWORD = u32;
pub type BOOL = i32;
pub type LONG = i32;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

const PAGE_READWRITE: DWORD = 0x04;
const FILE_MAP_ALL_ACCESS: DWORD = 0x000F_F01F;
const SYNCHRONIZE: DWORD = 0x0010_0000;
const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
pub const WAIT_OBJECT_0: DWORD = 0;
pub const WAIT_ABANDONED: DWORD = 0x0000_0080;
const ERROR_ALREADY_EXISTS: DWORD = 183;

const HKCU: HKEY = 0x8000_0001 as HKEY;
const HKLM: HKEY = 0x8000_0002 as HKEY;
const KEY_READ: DWORD = 0x20019;
const KEY_WRITE: DWORD = 0x20006;
const REG_SZ: DWORD = 1;
const REG_DWORD: DWORD = 4;

unsafe extern "system" {
    fn CreateFileMappingA(
        file: HANDLE,
        attrs: *mut core::ffi::c_void,
        protect: DWORD,
        size_high: DWORD,
        size_low: DWORD,
        name: *const i8,
    ) -> HANDLE;
    fn MapViewOfFile(
        mapping: HANDLE,
        access: DWORD,
        offset_high: DWORD,
        offset_low: DWORD,
        count: usize,
    ) -> *mut core::ffi::c_void;
    fn UnmapViewOfFile(base: *const core::ffi::c_void) -> BOOL;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn CreateMutexA(attrs: *mut core::ffi::c_void, initial: BOOL, name: *const i8) -> HANDLE;
    fn CreateEventA(
        attrs: *mut core::ffi::c_void,
        manual_reset: BOOL,
        initial: BOOL,
        name: *const i8,
    ) -> HANDLE;
    fn SetEvent(event: HANDLE) -> BOOL;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> DWORD;
    fn GetLastError() -> DWORD;
    fn GetCurrentProcessId() -> DWORD;
    fn QueryPerformanceCounter(count: *mut i64) -> BOOL;
    fn QueryPerformanceFrequency(freq: *mut i64) -> BOOL;
    fn Sleep(ms: u32);
    fn CreateWaitableTimerExW(
        attrs: *mut core::ffi::c_void,
        name: *const u16,
        flags: DWORD,
        access: DWORD,
    ) -> HANDLE;
    fn CreateWaitableTimerW(
        attrs: *mut core::ffi::c_void,
        manual_reset: BOOL,
        name: *const u16,
    ) -> HANDLE;
    fn SetWaitableTimer(
        timer: HANDLE,
        due: *const i64,
        period: LONG,
        routine: *mut core::ffi::c_void,
        arg: *mut core::ffi::c_void,
        resume: BOOL,
    ) -> BOOL;

    fn RegCreateKeyExW(
        key: HKEY,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: DWORD,
        access: DWORD,
        security: *mut core::ffi::c_void,
        out: *mut HKEY,
        disposition: *mut DWORD,
    ) -> LONG;
    fn RegOpenKeyExW(
        key: HKEY,
        sub: *const u16,
        reserved: u32,
        access: DWORD,
        out: *mut HKEY,
    ) -> LONG;
    fn RegCloseKey(key: HKEY) -> LONG;
    fn RegSetValueExW(
        key: HKEY,
        name: *const u16,
        reserved: u32,
        kind: DWORD,
        data: *const u8,
        len: DWORD,
    ) -> LONG;
    fn RegQueryValueExW(
        key: HKEY,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut DWORD,
        data: *mut u8,
        len: *mut DWORD,
    ) -> LONG;
    fn RegEnumKeyExW(
        key: HKEY,
        index: DWORD,
        name: *mut u16,
        len: *mut DWORD,
        reserved: *mut u32,
        class: *mut u16,
        class_len: *mut DWORD,
        time: *mut FileTime_,
    ) -> LONG;
    fn RegDeleteTreeW(key: HKEY, sub: *const u16) -> LONG;

    fn GetCurrentThread() -> HANDLE;
    fn SetThreadPriority(thread: HANDLE, priority: i32) -> BOOL;
}

/// The one file-time shape `RegEnumKeyExW` insists on accepting a pointer
/// to; nobody reads it.
#[repr(C)]
pub struct FileTime_ {
    pub lo: DWORD,
    pub hi: DWORD,
}

pub fn last_error() -> DWORD {
    unsafe { GetLastError() }
}

pub fn pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}

/// Millisecond sleep; the audio pacing never trusts it alone, only that
/// it never returns early.
pub fn sleep_ms(ms: u32) {
    unsafe { Sleep(ms) }
}

fn c(name: &str) -> CString {
    // Bus and event names are ASCII by construction (`proto` builds
    // them); a name with anything else in it is a programming error, and
    // an interior NUL would silently truncate the name into a different
    // object, so panic loudly instead.
    CString::new(name).expect("object name without interior NUL")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> OsString {
    OsString::from_wide(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())])
}

// ---------------------------------------------------------------- clock

pub struct Instant_(pub i64);

pub fn qpc_freq() -> i64 {
    static FREQ: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *FREQ.get_or_init(|| {
        let mut f = 0i64;
        unsafe { QueryPerformanceFrequency(&mut f) };
        f.max(1)
    })
}

pub fn now() -> Instant_ {
    let mut c = 0i64;
    unsafe { QueryPerformanceCounter(&mut c) };
    Instant_(c)
}

impl Instant_ {
    pub fn elapsed_ms(&self) -> u64 {
        let now = now();
        ((now.0 - self.0).max(0) as u128 * 1000 / qpc_freq() as u128) as u64
    }
}

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(ms: DWORD) -> DWORD;
    fn timeEndPeriod(ms: DWORD) -> DWORD;
}

// ------------------------------------------------------------ priority

#[link(name = "avrt")]
unsafe extern "system" {
    fn AvSetMmThreadCharacteristicsW(task: *const u16, task_index: *mut DWORD) -> HANDLE;
    fn AvSetMmThreadPriority(task: HANDLE, priority: DWORD) -> BOOL;
    fn AvRevertMmThreadCharacteristics(task: HANDLE) -> BOOL;
}

/// `AVRT_PRIORITY_HIGH`, the top of the band a "Pro Audio" client is
/// allowed to sit at.
const AVRT_PRIORITY_HIGH: DWORD = 1;
/// `THREAD_PRIORITY_TIME_CRITICAL`, for the fallback path.
const THREAD_PRIORITY_TIME_CRITICAL: i32 = 15;

/// The scheduler's answer to "this thread carries audio", undone by
/// dropping it — so a thread keeps it for exactly as long as it lives.
///
/// Without it, the thread that runs a host's entire `bufferSwitch` is an
/// ordinary, *normal*-priority thread: the first thing a browser, a
/// compiler or a disk flush does to a 1.3 ms deadline. Every professional
/// ASIO driver asks MMCSS for this instead of letting its host's graph be
/// scheduled like a text editor.
pub struct Realtime {
    mmss: Option<HANDLE>,
    /// What was actually granted, for the trace log.
    pub how: &'static str,
}

impl Drop for Realtime {
    fn drop(&mut self) {
        if let Some(handle) = self.mmss {
            unsafe { AvRevertMmThreadCharacteristics(handle) };
        }
    }
}

/// Register the calling thread for real-time work. MMCSS ("Pro Audio") is
/// the polite route: the scheduler then knows this thread is latency
/// sensitive and coordinates it with the rest of the system. If the
/// service cannot be reached — a stripped-down machine, a blocked
/// service, group policy — the thread simply goes time-critical, which
/// keeps the deadline and loses the coordination.
pub fn real_time(task: &str) -> Realtime {
    let name = wide(task);
    let mut index: DWORD = 0;
    let handle = unsafe { AvSetMmThreadCharacteristicsW(name.as_ptr(), &mut index) };
    if !handle.is_null() {
        unsafe { AvSetMmThreadPriority(handle, AVRT_PRIORITY_HIGH) };
        return Realtime {
            mmss: Some(handle),
            how: "MMCSS 'Pro Audio'",
        };
    }
    let granted = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
    // `AvSetMmThreadCharacteristicsW` failure is worth one line: it is
    // the difference between coordinated and raw real-time scheduling.
    crate::trace::log(&format!(
        "priority: MMCSS '{task}' refused ({}), thread {}",
        last_error(),
        if granted != 0 {
            "time-critical"
        } else {
            "left at normal"
        }
    ));
    Realtime {
        mmss: None,
        how: if granted != 0 {
            "time-critical"
        } else {
            "normal"
        },
    }
}

/// A precise wait for pacing the virtual clock.
///
/// `Sleep` ticks at the scheduler's cadence — nominally 15.6 ms — so a
/// 2.7 ms pacing period smears into a 15.6 ms one the moment the sleep
/// path touches it. A high-resolution waitable timer (Windows 10 1607
/// and later) wakes within a fraction of a millisecond of its deadline
/// and costs nothing while waiting. On anything older the fallback
/// raises the global timer resolution for exactly as long as the pacer
/// lives and waits on a plain timer; if even that fails, the wait
/// degrades to the old sleep-plus-spin loop.
pub struct Timer {
    handle: HANDLE,
    fine: bool,
    raised_tick: bool,
}

// Waiting on one's own timer handle from whichever thread paces the
// clock; the handle is never shared.
unsafe impl Send for Timer {}

const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: DWORD = 0x0000_0002;
const TIMER_SYNCH: DWORD = 0x0010_0002; // SYNCHRONIZE | TIMER_MODIFY_STATE

impl Timer {
    pub fn pacing() -> Self {
        let mut fine = false;
        let mut handle = unsafe {
            CreateWaitableTimerExW(
                ptr::null_mut(),
                ptr::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_SYNCH,
            )
        };
        if handle.is_null() {
            handle = unsafe { CreateWaitableTimerW(ptr::null_mut(), 0, ptr::null()) };
        } else {
            fine = true;
        }
        // A plain waitable timer expires on the same scheduler tick as
        // `Sleep`, and with no timer at all the sleep loop is all there
        // is — either way, ask the system for 1 ms ticks while the
        // pacer lives, and give them back when it does.
        let raised_tick = !fine;
        if raised_tick {
            unsafe { timeBeginPeriod(1) };
        }
        Timer {
            handle,
            fine,
            raised_tick,
        }
    }

    /// Block until the QPC instant `target`. Returns immediately if the
    /// deadline has already passed — catching up a pacer is the caller's
    /// decision, never a hidden burst of expired steps.
    pub fn wait_until(&self, target: &Instant_) {
        if self.handle.is_null() {
            // Degraded path: ms-granular sleeps (tick raised) plus the
            // spin tail, keyed off the wall clock.
            let freq = qpc_freq();
            loop {
                let left = target.0 - now().0;
                if left <= 0 {
                    return;
                }
                let ms = (left as i128 * 1000 / freq as i128) as i64;
                if ms > 2 {
                    sleep_ms((ms - 1) as u32);
                } else {
                    std::hint::spin_loop();
                }
            }
        }
        loop {
            let left = target.0 - now().0;
            if left <= 0 {
                return;
            }
            let hundred_ns =
                (left as i128 * 10_000_000 / qpc_freq() as i128).clamp(1, i64::MAX as i128) as i64;
            let ok = unsafe {
                SetWaitableTimer(
                    self.handle,
                    &raw const hundred_ns,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                )
            };
            if ok == 0 {
                // Timer disarmed itself under us; behave like the old code.
                let ms = (left as i128 * 1000 / qpc_freq() as i128) as i64;
                if ms > 2 {
                    sleep_ms(ms as u32);
                } else {
                    std::hint::spin_loop();
                }
                continue;
            }
            // The 50 ms cap is a safety net only: a wake that early just
            // re-arms against the same absolute deadline.
            unsafe { WaitForSingleObject(self.handle, 50) };
            if self.fine {
                while now().0 < target.0 {
                    std::hint::spin_loop();
                }
                return;
            }
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { CloseHandle(self.handle) };
        }
        if self.raised_tick {
            unsafe { timeEndPeriod(1) };
        }
    }
}

// -------------------------------------------------------------- mapping

/// A named page-backed mapping, mapped for its whole size, alive for as
/// long as this value is. `created` says whether this process saw
/// `ERROR_ALREADY_EXISTS` *not* set, which is the only signal the
/// protocol has for "nobody is home yet, you write the header".
pub struct Mapping {
    handle: HANDLE,
    base: *mut u8,
    created: bool,
}

// The whole point of the bus is that threads share it; the atomics and
// the events are what make that legal, not anything about the mapping.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    pub fn open_or_create(name: &str, size: usize) -> std::io::Result<Mapping> {
        let name = c(name);
        let handle = unsafe {
            let h = CreateFileMappingA(
                INVALID_HANDLE_VALUE,
                ptr::null_mut(),
                PAGE_READWRITE,
                (size >> 32) as DWORD,
                size as DWORD,
                name.as_ptr(),
            );
            if h.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let existed = last_error() == ERROR_ALREADY_EXISTS;
            (h, existed)
        };
        let (handle, existed) = handle;
        let base = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size) as *mut u8 };
        if base.is_null() {
            let err = std::io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(err);
        }
        Ok(Mapping {
            handle,
            base,
            created: !existed,
        })
    }

    pub fn base(&self) -> *mut u8 {
        self.base
    }

    pub fn created(&self) -> bool {
        self.created
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.base as *const _);
            CloseHandle(self.handle);
        }
    }
}

// --------------------------------------------------------------- events

/// A named auto-reset event. `CreateEventA` on an existing name just
/// hands back another handle, which is exactly the "everybody creates
/// their own" idiom the protocol wants: no create-then-open race.
pub struct Event {
    handle: HANDLE,
}

unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Event {
    pub fn named(name: &str) -> std::io::Result<Event> {
        let name = c(name);
        let handle = unsafe { CreateEventA(ptr::null_mut(), 0, 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Event { handle })
    }

    pub fn signal(&self) {
        unsafe { SetEvent(self.handle) };
    }

    /// `true` if the event fired before `ms` elapsed.
    pub fn wait(&self, ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.handle, ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

// ------------------------------------------------------------- election

/// The master claim. Kept alive by its holder; dropping it (explicitly
/// or by process death) makes the mutex acquirable by the next instance
/// that polls. One per process additionally gated by a process-local
/// flag, because a mutex is re-acquirable by its own holder and two
/// instances in one process must not both think they run the clock.
pub struct MasterClaim {
    _handle: HANDLE,
}

static PROCESS_CLAIM: AtomicBool = AtomicBool::new(false);

impl MasterClaim {
    /// Takes the master role if it is free *and* no other instance in
    /// this process already took it. `WAIT_ABANDONED` counts: a dead
    /// master is precisely the situation the election exists for.
    pub fn try_elect(name: &str) -> Option<MasterClaim> {
        if PROCESS_CLAIM.swap(true, Ordering::SeqCst) {
            return None;
        }
        let cname = c(name);
        let handle = unsafe { CreateMutexA(ptr::null_mut(), 0, cname.as_ptr()) };
        if handle.is_null() {
            PROCESS_CLAIM.store(false, Ordering::SeqCst);
            return None;
        }
        let wait = unsafe { WaitForSingleObject(handle, 0) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            Some(MasterClaim { _handle: handle })
        } else {
            unsafe { CloseHandle(handle) };
            PROCESS_CLAIM.store(false, Ordering::SeqCst);
            None
        }
    }
}

impl Drop for MasterClaim {
    fn drop(&mut self) {
        unsafe { CloseHandle(self._handle) };
        PROCESS_CLAIM.store(false, Ordering::SeqCst);
    }
}

// ------------------------------------------------------------- registry

/// Registry hive the per-user driver registration lives in, and the one
/// every host's installer writes to; both must be read to list devices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

impl Hive {
    fn raw(self) -> HKEY {
        match self {
            Hive::CurrentUser => HKCU,
            Hive::LocalMachine => HKLM,
        }
    }
}

pub struct RegKey {
    key: HKEY,
}

impl RegKey {
    pub fn create(hive: Hive, sub: &str) -> std::io::Result<RegKey> {
        let sub = wide(sub);
        let mut out = ptr::null_mut();
        let code = unsafe {
            RegCreateKeyExW(
                hive.raw(),
                sub.as_ptr(),
                0,
                ptr::null(),
                0,
                KEY_WRITE | SYNCHRONIZE,
                ptr::null_mut(),
                &mut out,
                ptr::null_mut(),
            )
        };
        if code != 0 {
            return Err(std::io::Error::from_raw_os_error(code));
        }
        Ok(RegKey { key: out })
    }

    pub fn open(hive: Hive, sub: &str) -> std::io::Result<RegKey> {
        let sub = wide(sub);
        let mut out = ptr::null_mut();
        let code = unsafe { RegOpenKeyExW(hive.raw(), sub.as_ptr(), 0, KEY_READ, &mut out) };
        if code != 0 {
            return Err(std::io::Error::from_raw_os_error(code));
        }
        Ok(RegKey { key: out })
    }

    pub fn set_sz(&self, name: &str, value: &str) -> std::io::Result<()> {
        let name = wide(name);
        let bytes: Vec<u8> = wide(value).iter().flat_map(|u| u.to_ne_bytes()).collect();
        let code = unsafe {
            RegSetValueExW(
                self.key,
                name.as_ptr(),
                0,
                REG_SZ,
                bytes.as_ptr(),
                bytes.len() as DWORD,
            )
        };
        if code != 0 {
            return Err(std::io::Error::from_raw_os_error(code));
        }
        Ok(())
    }

    pub fn set_dword(&self, name: &str, value: u32) -> std::io::Result<()> {
        let name = wide(name);
        let bytes = value.to_ne_bytes();
        let code =
            unsafe { RegSetValueExW(self.key, name.as_ptr(), 0, REG_DWORD, bytes.as_ptr(), 4) };
        if code != 0 {
            return Err(std::io::Error::from_raw_os_error(code));
        }
        Ok(())
    }

    pub fn sz(&self, name: &str) -> Option<String> {
        let name = wide(name);
        let mut kind = 0;
        let mut len = 0;
        let probe = unsafe {
            RegQueryValueExW(
                self.key,
                name.as_ptr(),
                ptr::null_mut(),
                &mut kind,
                ptr::null_mut(),
                &mut len,
            )
        };
        if probe != 0 || kind != REG_SZ {
            return None;
        }
        let mut buf = vec![0u16; (len as usize / 2).max(1) + 1];
        let code = unsafe {
            RegQueryValueExW(
                self.key,
                name.as_ptr(),
                ptr::null_mut(),
                &mut kind,
                buf.as_mut_ptr() as *mut u8,
                &mut len,
            )
        };
        if code != 0 || kind != REG_SZ {
            return None;
        }
        Some(from_wide(&buf).to_string_lossy().into_owned())
    }

    pub fn dword(&self, name: &str) -> Option<u32> {
        let name = wide(name);
        let mut kind = 0;
        let mut buf = [0u8; 4];
        let mut len = 4;
        let code = unsafe {
            RegQueryValueExW(
                self.key,
                name.as_ptr(),
                ptr::null_mut(),
                &mut kind,
                buf.as_mut_ptr(),
                &mut len,
            )
        };
        if code != 0 || kind != REG_DWORD || len < 4 {
            return None;
        }
        Some(u32::from_ne_bytes(buf))
    }

    /// Immediate subkey names; `ERROR_NO_MORE_ITEMS` (259) ends the walk.
    pub fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut name = [0u16; 256];
        for index in 0.. {
            let mut len = name.len() as DWORD;
            let code = unsafe {
                RegEnumKeyExW(
                    self.key,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            if code != 0 {
                break;
            }
            out.push(from_wide(&name).to_string_lossy().into_owned());
        }
        out
    }

    pub fn delete_tree(hive: Hive, sub: &str) -> std::io::Result<()> {
        let sub = wide(sub);
        let code = unsafe { RegDeleteTreeW(hive.raw(), sub.as_ptr()) };
        // 2 = ERROR_FILE_NOT_FOUND: deleting an absent registration is
        // a success from the caller's point of view.
        if code == 0 || code == 2 {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(code))
        }
    }
}

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.key) };
    }
}
