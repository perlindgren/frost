//! Per-user registration, the config the master reads, and a device
//! lister that sees what every host sees.
//!
//! Everything writes to `HKCU`: an ASIO driver that needs elevation to
//! install is a driver nobody's guests can install, and per-user is
//! where REAPER and the NI drivers themselves register on this
//! machine's own terms. Hosts that enumerate only `HKLM` will miss us —
//! REAPER reads both, and `nice-common`'s lister is taught to merge the
//! hives in the same breath.

use std::path::Path;

use crate::win::{Guid, Hive, RegKey};
use crate::{CLSID_STR, DRIVER_NAME};

/// Where the master finds the real hardware driver to host; absent or
/// empty means the safe default, a virtual-only device.
const CONFIG_KEY: &str = r"Software\NiceRackAsio";
/// Where hosts look for drivers, per user and machine-wide.
const ASIO_KEY: &str = r"Software\ASIO";
const CLSID_KEY: &str = r"Software\Classes\CLSID";

pub struct Config {
    pub real_driver: Option<String>,
    pub hw_in: u32,
    pub hw_out: u32,
    /// Frames per hardware callback; 0 lets the first host to start decide.
    ///
    /// This is the one knob that decides whether the whole bus makes its
    /// deadlines, because the hardware's block is the bus's clock: every
    /// client is paced by it, whatever its host asked for. A driver that
    /// is happy at 64 frames *alone* has to serve 64 frames every 1.33 ms
    /// while also copying sixteen rings of shared memory on the same
    /// thread — and a driver whose callbacks land on 1 ms USB frame
    /// boundaries will answer it in 2 ms gaps — a sixth of them measured
    /// here, see `master::CLOCK_FLOOR`, where the same driver at 128
    /// frames misses 4 callbacks in 751 instead.
    ///
    /// Left at 0, the size comes from the host that has the deadline: the
    /// first client to start names a block in its own `createBuffers`, and
    /// the hardware is clocked at that, fitted upward into the driver's own
    /// set of sizes. `HwBlock` outranks it for the cases where that trade is
    /// the wrong one — and it is set in the registry rather than in a host's
    /// UI so the A/B never depends on somebody clicking through a preferences
    /// dialog.
    pub hw_block: u32,
}

impl Config {
    pub fn load() -> Config {
        let key = RegKey::open(Hive::CurrentUser, CONFIG_KEY).ok();
        let real = key
            .as_ref()
            .and_then(|k| k.sz("RealDriver"))
            .filter(|s| !s.trim().is_empty() && !no_hw_env());
        Config {
            real_driver: real,
            hw_in: key
                .as_ref()
                .and_then(|k| k.dword("HwIn"))
                .unwrap_or(2)
                .clamp(0, 8),
            hw_out: key
                .as_ref()
                .and_then(|k| k.dword("HwOut"))
                .unwrap_or(2)
                .clamp(0, 8),
            hw_block: env_hw_block()
                .or_else(|| key.as_ref().and_then(|k| k.dword("HwBlock")))
                .unwrap_or(0)
                .min(4096),
        }
    }
}

/// Measure-first escape hatch: `NICEASIO_HW_BLOCK` outranks `HwBlock` for
/// the same reason `NICEASIO_NO_HW` outranks `RealDriver` — a probe run
/// from a sandbox cannot write the registry at all, and an A/B of two
/// clock sizes has to be two command lines, not two registry edits and a
/// prayer that the writes landed.
pub fn env_hw_block() -> Option<u32> {
    std::env::var("NICEASIO_HW_BLOCK").ok()?.trim().parse().ok()
}

/// The escape hatch for tooling: master behaves virtual-only no matter
/// what the registry says, so a probe or test can never open hardware
/// somebody else is holding.
pub fn no_hw_env() -> bool {
    std::env::var_os("NICEASIO_NO_HW").is_some_and(|v| v != "0")
}

/// Installs the device for the current user. `real` names the driver
/// the master should host (exact registry display name); `None` leaves
/// whatever was configured alone, and `Some("")` explicitly clears it
/// back to virtual-only.
pub fn register(
    dll_path: &Path,
    real: Option<&str>,
    hw_in: Option<u32>,
    hw_out: Option<u32>,
) -> std::io::Result<()> {
    let asio = RegKey::create(Hive::CurrentUser, &format!(r"{ASIO_KEY}\{DRIVER_NAME}"))?;
    asio.set_sz("clsid", CLSID_STR)?;
    asio.set_sz("description", DRIVER_NAME)?;

    let inproc = RegKey::create(
        Hive::CurrentUser,
        &format!(r"{CLSID_KEY}\{CLSID_STR}\InprocServer32"),
    )?;
    inproc.set_sz(
        "",
        &dll_path
            .canonicalize()
            .unwrap_or_else(|_| dll_path.to_path_buf())
            .to_string_lossy(),
    )?;
    inproc.set_sz("ThreadingModel", "Both")?;

    let config = RegKey::create(Hive::CurrentUser, CONFIG_KEY)?;
    if let Some(real) = real {
        config.set_sz("RealDriver", real)?;
    }
    if let Some(n) = hw_in {
        config.set_dword("HwIn", n.clamp(0, 8))?;
    }
    if let Some(n) = hw_out {
        config.set_dword("HwOut", n.clamp(0, 8))?;
    }
    Ok(())
}

/// Point the bus's clock at a specific hardware block size; 0 hands the
/// choice back to the first host to start. Separate from [`register`]
/// because it is a knob an operator turns while the device is installed, not
/// part of installing it — and because it lives in the registry, changing it
/// needs no host restarted through its preferences.
pub fn set_hw_block(frames: u32) -> std::io::Result<()> {
    RegKey::create(Hive::CurrentUser, CONFIG_KEY)?.set_dword("HwBlock", frames.min(4096))
}

pub fn unregister() -> std::io::Result<()> {
    RegKey::delete_tree(Hive::CurrentUser, &format!(r"{ASIO_KEY}\{DRIVER_NAME}"))?;
    RegKey::delete_tree(Hive::CurrentUser, &format!(r"{CLSID_KEY}\{CLSID_STR}"))?;
    // The config key outlives the device on purpose: it is where
    // "which real driver did you like" survives a re-install.
    Ok(())
}

/// One entry of a host's driver dropdown.
pub struct DriverEntry {
    pub name: String,
    pub clsid: String,
    pub per_user: bool,
}

/// Every registered ASIO driver, both hives merged — the same view the
/// master needs to turn a display name into a CLSID, and niceroute
/// exposes because `azo`'s own enumerator reads `HKLM` only and would
/// walk past every per-user driver on the machine.
pub fn enumerate_drivers() -> Vec<DriverEntry> {
    let mut out = Vec::new();
    for (hive, per_user) in [(Hive::LocalMachine, false), (Hive::CurrentUser, true)] {
        let Ok(root) = RegKey::open(hive, ASIO_KEY) else {
            continue;
        };
        for name in root.subkeys() {
            let Ok(key) = RegKey::open(hive, &format!(r"{ASIO_KEY}\{name}")) else {
                continue;
            };
            if let Some(clsid) = key.sz("clsid") {
                out.push(DriverEntry {
                    name: key.sz("description").unwrap_or_else(|| name.clone()),
                    clsid,
                    per_user,
                });
            }
        }
    }
    out
}

pub fn clsid_for_name(name: &str) -> Option<Guid> {
    enumerate_drivers()
        .into_iter()
        .find(|e| e.name.eq_ignore_ascii_case(name))
        .and_then(|e| parse_guid(&e.clsid))
}

pub fn parse_guid(s: &str) -> Option<Guid> {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}');
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5
        || parts[0].len() != 8
        || parts[1].len() != 4
        || parts[2].len() != 4
        || parts[3].len() != 4
        || parts[4].len() != 12
    {
        return None;
    }
    let hex = |t: &str| u32::from_str_radix(t, 16).ok();
    let tail: Vec<u8> = (0..8)
        .map(|i| {
            let src = if i < 2 { parts[3] } else { parts[4] };
            let at = if i < 2 { i * 2 } else { (i - 2) * 2 };
            u8::from_str_radix(&src[at..at + 2], 16).ok()
        })
        .collect::<Option<_>>()?;
    Some(Guid {
        data1: hex(parts[0])?,
        data2: hex(parts[1])? as u16,
        data3: hex(parts[2])? as u16,
        data4: tail.try_into().unwrap(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CLSID;

    #[test]
    fn own_clsid_parses_to_itself() {
        assert_eq!(parse_guid(CLSID_STR), Some(CLSID));
    }

    #[test]
    fn junk_does_not_parse() {
        assert_eq!(parse_guid("{0},0,0,0,0"), None);
        assert_eq!(parse_guid("not a guid"), None);
    }
}
