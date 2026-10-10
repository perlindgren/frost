//! Assets by name. Game code never says `include_bytes!` and never
//! says a path: it says *give me `basic_tiles`*, and this module
//! answers — from the table compiled into the binary, or from a loose
//! file that shadows its entry.
//!
//! The design is one request and two worlds. Release ships one
//! binary: the [`Table`] answers every name, and no asset file needs
//! to exist. Development runs the same code with loose directories
//! admitted: a file's stem names it, and it outranks the table — so
//! an edit saved in `sprite_util` lands in the next load, and a
//! folder left beside a shipped binary is the update channel the
//! single binary was promised. No feature flag forks the two: the
//! debug binary carries its table and ignores it, which is the price
//! of one code path, paid gladly. Watching files for change is the
//! ladder's next rung; the honesty of this one is that loose files
//! are read when they are asked for.

/// The table an app ships: every asset by name, every name by bytes.
/// The name is the file's stem — the identity that survives folders,
/// slots, and the filesystem itself — and the bytes are whatever the
/// name means in the build.
pub type Table = &'static [(&'static str, &'static [u8])];

/// The resolver behind the one request: loose files first, the
/// compiled table beneath them, `None` beneath both.
#[derive(Default)]
pub struct Assets {
    /// The bytes compiled into this binary.
    table: Table,
    /// The loose files, scanned from the admitted directories at
    /// build time, first directory winning: name to path. A
    /// directory that cannot be read — the normal case beside a
    /// shipped binary — simply admits nothing.
    loose: Vec<(String, std::path::PathBuf)>,
}

impl Assets {
    /// The resolver of a shipped world: the table, and nothing loose.
    #[must_use]
    pub fn embedded(table: Table) -> Self {
        Self {
            table,
            loose: Vec::new(),
        }
    }

    /// Admit one directory of loose assets: every file's stem names
    /// it, and a loose file outranks the table's entry of the same
    /// name. Called in development; harmless anywhere else, where
    /// the directory does not answer and the table stands alone.
    #[must_use]
    pub fn with_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        let dir = dir.into();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let Some(stem) = path.file_stem() else {
                    continue;
                };
                let stem = stem.to_string_lossy().into_owned();
                if !self.loose.iter().any(|(name, _)| *name == stem) {
                    self.loose.push((stem, path));
                }
            }
            // Sorted so the first file of a name wins the same way
            // on every filesystem: `read_dir` owes no order.
            self.loose.sort_by(|a, b| a.1.cmp(&b.1));
        } else {
            log::debug!("assets: '{}' speaks nothing", dir.display());
        }
        self
    }

    /// The bytes a name means now, whatever format they arrive in:
    /// the loose file if one speaks it, the table otherwise, nothing
    /// if neither does. A loose file that cannot be read — mid-save,
    /// half-written — falls to the table rather than going silent.
    #[must_use]
    pub fn bytes(&self, name: &str) -> Option<std::borrow::Cow<'static, [u8]>> {
        self.asset(name, &[])
    }

    /// The bytes a name means in one of the accepted formats. The
    /// extension filter is not ceremony: a directory of sprites keeps
    /// its sidecars beside the PNGs, and `Getingeye1.ron` — the
    /// sprite's metadata, found the hard way — shares its stem with
    /// the sprite itself. Name and format together are the full
    /// request; a loose file of an unaccepted format speaks nothing,
    /// and the table's entry answers undisturbed.
    #[must_use]
    pub fn asset(&self, name: &str, exts: &[&str]) -> Option<std::borrow::Cow<'static, [u8]>> {
        let speaks = |path: &std::path::Path| {
            exts.is_empty()
                || path
                    .extension()
                    .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
        };
        if let Some((_, path)) = self
            .loose
            .iter()
            .find(|(stem, path)| stem == name && speaks(path))
        {
            match std::fs::read(path) {
                Ok(bytes) => return Some(std::borrow::Cow::Owned(bytes)),
                Err(err) => log::warn!("assets: '{}' unreadable: {err}", path.display()),
            }
        }
        self.table
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, bytes)| std::borrow::Cow::Borrowed(*bytes))
    }

    /// The names the table ships: an app's guard test asks what its
    /// world names and compares — every tileset a map calls for must
    /// be in the build that carries it.
    pub fn names(&self) -> impl Iterator<Item = &'static str> {
        self.table.iter().map(|(name, _)| *name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIP: Table = &[("tiles", b"table bytes"), ("grass", b"table grass")];

    /// The shipped world answers by name, and a name nobody ships
    /// answers nothing — a missing asset is a miss, not a guess.
    #[test]
    fn the_table_answers_by_name() {
        let assets = Assets::embedded(SHIP);
        assert_eq!(assets.bytes("tiles").as_deref(), Some(&b"table bytes"[..]));
        assert!(assets.bytes("barn").is_none());
    }

    /// The names the table ships are askable: this is what the
    /// guard test of an app will compare its maps against.
    #[test]
    fn the_guard_can_ask_what_ships() {
        let names: Vec<_> = Assets::embedded(SHIP).names().collect();
        assert_eq!(names, vec!["tiles", "grass"]);
    }

    /// The sidecar trap, pinned: a directory of sprites keeps its
    /// `.ron` metadata beside the PNGs, and a format-aware ask walks
    /// past the shadow into the table's picture.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_sidecar_never_dresses_a_sprite() {
        let dir = std::env::temp_dir().join(format!("frost_assets_sidecar_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tiles.ron"), b"(atlas: [2, 8])").unwrap();
        let assets = Assets::embedded(SHIP).with_dir(&dir);
        assert_eq!(
            assets.asset("tiles", &["png"]).as_deref(),
            Some(&b"table bytes"[..])
        );
        assert_eq!(
            assets.asset("tiles", &["ron"]).as_deref(),
            Some(&b"(atlas: [2, 8])"[..])
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A loose file outranks its table entry — the dev loop's edit
    /// is what the next load sees — while names without a loose
    /// file keep answering from the table.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_loose_file_outranks_the_table() {
        let dir = std::env::temp_dir().join(format!("frost_assets_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("grass.png"), b"loose grass").unwrap();
        let assets = Assets::embedded(SHIP).with_dir(&dir);
        assert_eq!(assets.bytes("grass").as_deref(), Some(&b"loose grass"[..]));
        assert_eq!(assets.bytes("tiles").as_deref(), Some(&b"table bytes"[..]));
        std::fs::remove_dir_all(&dir).ok();
    }
}
