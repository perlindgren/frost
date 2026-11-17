//! Opt-in, main-thread NAM destruction/entry-release test in a child process.
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]

use clack_host::prelude::*;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct Host;
impl HostHandlers for Host {
    type Shared<'a> = ();
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();
}

fn cycles(path: &Path) {
    let info = HostInfo::new(
        "NAM unload regression",
        "frost",
        "https://github.com/perlindgren/frost",
        "0.1.0",
    )
    .unwrap();
    for _ in 0..32 {
        // SAFETY: explicit opt-in to the repository's freshly built nice-nam
        // export. No installed third-party plugin is scanned or auto-loaded.
        // Use Clack directly: our Entry deliberately pins libraries for life,
        // which would mask the worker bug this isolated process tests.
        let entry = unsafe { PluginEntry::load(path).unwrap() };
        let instance =
            PluginInstance::<Host>::new(|_| (), |_| (), &entry, c"com.frost.nice-nam", &info)
                .unwrap();
        drop(instance);
        drop(entry);
        // Give a previously detached 20-ms polling worker time to resume after
        // entry release. OS loader residency policies can still delay unmapping;
        // the unit tests independently assert completion before Drop returns.
        std::thread::sleep(Duration::from_millis(25));
    }
    println!("32 native NAM load/create/destroy/entry-release cycles passed");
}

fn main() {
    let Some(path) = std::env::var_os("NICE_RACK_TEST_NAM") else {
        println!(
            "Native NAM unload check not run: set NICE_RACK_TEST_NAM to the repository's freshly built nice-nam CLAP export"
        );
        return;
    };
    let path = PathBuf::from(path)
        .canonicalize()
        .expect("resolve the NAM export path");
    if std::env::var_os("NICE_NAM_UNLOAD_CHILD").is_some() {
        cycles(&path);
        return;
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
        .env("NICE_RACK_TEST_NAM", &path)
        .env("NICE_NAM_UNLOAD_CHILD", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start isolated NAM unload check");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "Native NAM destruction/unload check timed out:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "Native NAM destruction/unload check failed ({}):\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
}
