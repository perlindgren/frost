//! Watching a file for change: the ladder's shared sense.
//!
//! Every hot tier climbs the same rung-shape — notice, re-read, swap
//! in — and noticing is this module's whole job: remember a file's
//! stamp, and answer `changed()` truthfully whenever it differs.
//! Polling metadata is a deliberate modesty: one `stat` per watched
//! file per frame costs microseconds, carries no OS event machinery,
//! and works identically for data, textures, maps, and shaders — the
//! tiers differ in what they re-read, never in how they notice.
//!
//! A file that does not exist is watched like one that does:
//! appearing is a change, and so is vanishing — a loose development
//! file removed beside a shipped binary tells the app to fall back
//! to what the binary carries. On targets without a filesystem every
//! answer is `false`: a shipped world does not change under itself.

/// One file, watched by its stamp: `changed()` reports that the file
/// on disk is not the one last seen — rewritten, appeared, or gone.
#[derive(Clone, Debug)]
pub struct Watch {
    /// The file being watched, kept for the log's sake.
    target: std::path::PathBuf,
    /// The stamp at the last look, `None` while the file did not
    /// exist: no stamp is a stamp of its own.
    stamp: Option<(std::time::SystemTime, u64)>,
}

impl Watch {
    /// Watch a file from its state right now: the first `changed()`
    /// answers for changes after this moment, not before it.
    #[must_use]
    pub fn new(target: impl Into<std::path::PathBuf>) -> Self {
        let target = target.into();
        Self {
            stamp: stamp(&target),
            target,
        }
    }

    /// True once per change: the file was rewritten, appeared, or
    /// vanished since the last look. Every other frame: false.
    pub fn changed(&mut self) -> bool {
        let now = stamp(&self.target);
        let changed = now != self.stamp;
        if changed {
            self.stamp = now;
            log::debug!("watch: '{}' changed", self.target.display());
        }
        changed
    }
}

/// The file's stamp — its last-modified time and its length — or
/// `None` while it cannot be asked: absent, or a target without a
/// filesystem. Both answer the same, and both are honest: nothing
/// has changed that can be seen.
///
/// The length rides with the time because time alone lies at the
/// tick: a filesystem stamps writes no finer than its own resolution
/// — 100 ns on NTFS, two seconds on FAT, and anything a real-time
/// scanner or a syncing temp folder does to the metadata in between —
/// so two writes inside one tick carry one stamp and a watcher built
/// on time alone sleeps through the second. Length asks the same
/// `stat` for nothing, and the rewrite that keeps its exact length
/// to the byte *and* lands inside one tick is a collision test
/// suites manufacture and no editor has ever delivered.
fn stamp(target: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(target).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch file with a name of its own, reserved by the claim
    /// that creates it, gone when the test is.
    ///
    /// The name carries a pid *and* a claim counter, and the claim is
    /// `create_new`: on Windows a leftover from a dead run — whose
    /// pid a later process may well have inherited — sits on the same
    /// name, and whatever holds it (a real-time scanner, a syncing
    /// temp folder, a crashed process's handle) answers an overwrite
    /// with Access denied, which is a sharing violation wearing a
    /// permission mask. Asking instead for a name that does not exist
    /// yet makes leftovers someone else's problem: the claim simply
    /// takes the next number. And a temp directory that refuses
    /// creation outright — a redirected, syncing, or read-only one —
    /// sends the claim to the workspace's own `target`, where a dev
    /// loop is guaranteed write access by the fact that it just
    /// compiled there.
    fn scratch(name: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let dirs = [
            std::env::temp_dir(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"),
        ];
        for dir in &dirs {
            for claim in 0.. {
                let path = dir.join(format!("frost_watch_{name}_{pid}_{claim}"));
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(_) => return path,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(_) => break,
                }
            }
        }
        panic!("no scratch directory the watcher tests can write to");
    }

    /// The quiet case first: a file nobody touches never reports.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_untouched_file_is_never_a_change() {
        let path = scratch("quiet");
        std::fs::write(&path, b"one").unwrap();
        let mut w = Watch::new(&path);
        assert!(!w.changed());
        assert!(!w.changed());
        std::fs::remove_file(&path).ok();
    }

    /// A rewrite is a change, once: the watcher notices and settles.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_rewrite_is_one_change_not_an_argument() {
        let path = scratch("rewrite");
        std::fs::write(&path, b"one").unwrap();
        let mut w = Watch::new(&path);
        std::fs::write(&path, b"two, rewritten").unwrap();
        assert!(w.changed());
        assert!(!w.changed());
        std::fs::remove_file(&path).ok();
    }

    /// Appearing is a change — the development file written after
    /// the app booted — and so is vanishing, which sends the app
    /// back to what its binary carries.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn appearing_and_vanishing_are_changes_too() {
        let path = scratch("appear");
        std::fs::remove_file(&path).ok();
        let mut w = Watch::new(&path);
        assert!(!w.changed());
        std::fs::write(&path, b"hello").unwrap();
        assert!(w.changed());
        std::fs::remove_file(&path).ok();
        assert!(w.changed());
    }
}
