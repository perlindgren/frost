//! NiceRack ASIO — a virtual ASIO driver whose bus is served by whichever
//! instance holds the hardware. The long version, with the reasoning, is in
//! [`design.md`](../design.md); this is the shape of it:
//!
//! * every host (REAPER, nice-rack, the probe) loads this DLL and gets a
//!   plain ASIO device: eight virtual outputs, eight virtual inputs, and the
//!   master's hardware — whatever it is, whenever it is — up front;
//! * the instances find each other through one shared mapping and a
//!   per-slot pair of events; the one that wins the election also drives
//!   the clock, hosts the real driver with `azo`, and mixes;
//! * `niceroute.dll` is the device (the `cdylib` target of this crate), and
//!   the `rlib` half is what nice-rack itself calls to put the device in
//!   the registry and to speak to it from the outside, exactly as a host
//!   would.
//!
//! The vtable in [`abi`] is hand-written against the same layout the `azo`
//! host bindings already trust for every real driver on this machine, and
//! the probe drives the whole thing back through `azo`'s host side, so the
//! ABI is checked by a second independent implementation and not only by
//! the one that authored it.

#[cfg(windows)]
pub mod abi;
#[cfg(windows)]
mod bus;
#[cfg(windows)]
mod instance;
#[cfg(windows)]
mod master;
pub mod proto;
#[cfg(windows)]
pub mod register;
#[cfg(windows)]
pub mod trace;
#[cfg(windows)]
mod win;

/// The driver name hosts list. Thirty-two bytes is all ASIO's own name
/// fields carry, and every registry value uses this same string.
pub const DRIVER_NAME: &str = "NiceRack ASIO (x64)";

/// The CLSID doubles as the interface id of the replica interface every
/// ASIO driver implements, so this one pair names both the registration
/// key and the `CoCreateInstance` argument. Typed out, never generated: a
/// device that changes its class id between builds silently unregisters
/// itself from every host that had it.
#[cfg(windows)]
pub const CLSID: crate::win::Guid = crate::win::Guid {
    data1: 0x7A4E_1C93,
    data2: 0x6D2B,
    data3: 0x4F58,
    data4: [0xA0, 0xB7, 0x5C, 0x31, 0xD9, 0xE8, 0x42, 0x16],
};

/// The same GUID the registry spells, braces and dashes included.
pub const CLSID_STR: &str = "{7A4E1C93-6D2B-4F58-A0B7-5C31D9E84216}";

/// [`CLSID`] in the form `CoCreateInstance` takes, so tooling (the probe,
/// nice-rack's own registration path) never re-derives the bytes or has
/// to name the internal `Guid` type to reach the device.
#[cfg(windows)]
pub fn co_clsid() -> azo::windows_core::GUID {
    // The two layouts are asserted identical at the one place that
    // already crosses the boundary in the other direction (master
    // hosting); a mismatch would be a compile-time-visible bug here too.
    debug_assert_eq!(
        std::mem::size_of::<azo::windows_core::GUID>(),
        std::mem::size_of::<win::Guid>()
    );
    unsafe { std::mem::transmute_copy(&CLSID) }
}
