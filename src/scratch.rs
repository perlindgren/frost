//! Where a test needs a folder of its own on disk, the name is
//! claimed by the claim that takes it — never assumed.
//!
//! The assumed pattern — `temp_dir().join(name-with-pid)`, then
//! create or overwrite — goes quiet-and-dead on Windows, where two
//! facts compose badly: a process id is reused eagerly, and a dead
//! run's leftover file is often still held by something (a real-time
//! scanner, a sync client, a crashed process's handle). The new run
//! meets its own inherited name already spoken for, and the
//! filesystem answers the overwrite with a sharing violation wearing
//! a permission mask — *Access denied* on a machine that never
//! denied a permission.
//!
//! Claiming is the whole remedy: `create_dir` a name with a counter
//! on the end and take the first number the filesystem hands out
//! whole. A leftover — locked or not — fails the claim as *exists*,
//! which is a fact to step over rather than a permission to fight.
//! And a temp directory that refuses creation outright (redirected,
//! syncing, read-only) sends the claim to the workspace's own
//! `target`, where a test that just compiled there writes by
//! construction. What is given up is the tidy sweep: claims leave
//! their folders behind for the OS temp policy to age out, and
//! deleting a file you do not own was never the point anyway.

use std::path::PathBuf;

/// Claim a fresh scratch directory named `{tag}-{pid}-{n}`, trying the
/// system temp first and the workspace's `target` after it. Panics
/// only if both refuse — a machine where no writable folder can be
/// claimed has larger problems than this test, and says so.
#[must_use]
pub(crate) fn claim(tag: &str) -> PathBuf {
    let pid = std::process::id();
    let dirs = [
        std::env::temp_dir(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"),
    ];
    for dir in &dirs {
        // The fallback's parent may itself be missing (a member crate
        // built from the workspace root); making room for claims is
        // not the same act as claiming one — only `create_dir` below
        // distinguishes *exists* from *made*, so leftovers still
        // cannot sneak in behind this call.
        let _ = std::fs::create_dir_all(dir);
        for n in 0.. {
            let path = dir.join(format!("{tag}-{pid}-{n}"));
            match std::fs::create_dir(&path) {
                Ok(()) => return path,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => break,
            }
        }
    }
    panic!("no scratch directory `{tag}` can claim");
}
