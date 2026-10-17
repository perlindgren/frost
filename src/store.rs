//! The store: text kept under a name until the next launch. The design
//! is `frost::Assets` turned around. The assets table answers *give me
//! the bytes this name ships with*, and cannot write; the store answers
//! *hold this text under this name for next time*, and cannot ship.
//! The same noun survives both — the name — and the same two worlds
//! answer: a native run keeps a file beside the launch folder, the
//! shape settings and saves have always had; a web run has no disc, so
//! the browser's own memory, its per-origin `localStorage`, holds what
//! a file holds. A game asks once, by name, and never learns which
//! world it lives in: `antifrost.progress.ron` is a file on the desktop
//! and a key in the browser, and the code that reads it is one code.
//!
//! What travels is text, because what wants keeping is text — Ron
//! documents, save files, a settings page the player can read and fix
//! by hand. Binary rides no such promise: a save that outlives a GPU
//! format should not be bytes with a layout nobody remembers. Games
//! that want binary keep it as text they encode themselves.
//!
//! Failure is a returned shrug, never a popped frame: no memory to
//! write to (a private window, a browser that refuses storage) costs
//! persistence, not the game.

/// Reads the text kept under `name`, if anything is. A name never kept,
/// an unreadable file, and a window with no memory all answer the same
/// way: `None`, the words for *nothing to remember yet*.
pub fn load(name: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = std::env::current_dir().unwrap_or_default().join(name);
        std::fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let item = storage()?.get_item(name).ok().flatten();
        item
    }
}

/// Keeps `text` under `name` until the next launch — over what stood
/// there, in whichever world this runs. The `String` error is a message
/// for the log line, not a condition to branch on.
pub fn save(name: &str, text: &str) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = std::env::current_dir().unwrap_or_default().join(name);
        std::fs::write(path, text).map_err(|err| err.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let store = storage().ok_or("this browser offers no local storage")?;
        store.set_item(name, text).map_err(|err| format!("{err:?}"))
    }
}

/// The browser's per-origin memory, when this window has one: no
/// window, a browser that throws at `localStorage` (Safari answers
/// exactly that from a private window), and a denied permission are
/// all the same absent memory.
#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test's own name, so a run that died before its cleanup
    /// leaves one obvious file to notice rather than a mystery.
    const NAME: &str = "frost-store-test.tmp.ron";

    #[test]
    fn what_is_kept_is_read_back() {
        save(NAME, "(\n    hill: 3,\n)\n").expect("the launch folder takes the write");
        assert_eq!(load(NAME).as_deref(), Some("(\n    hill: 3,\n)\n"));
        // And over: the second keeping stands, not the first.
        save(NAME, "(\n    hill: 4,\n)\n").expect("the write lands again");
        assert_eq!(load(NAME).as_deref(), Some("(\n    hill: 4,\n)\n"));
        std::fs::remove_file(NAME).expect("the test leaves no file");
    }

    #[test]
    fn a_name_never_kept_answers_nothing() {
        assert_eq!(load("frost-store-absent.tmp.ron"), None);
    }
}
