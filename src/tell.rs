//! The telling: how a running application announces itself to its
//! guests.
//!
//! In frost the truth is a running program — the scene tree is a
//! frame artifact, manufactured and discarded — so a tool cannot
//! *extract* an application's state of affairs; it can only be
//! *told*. This module is the apparatus of telling: an announce
//! card in a well-known folder (tools enumerate it the way a
//! debugger lists processes), and a live file the application
//! re-publishes when its being changes.
//!
//! The telling is transition-driven and carries structure only.
//! The clock is a value-key dirty check — the serialization *is*
//! the snapshot, so comparing against the last published body is
//! nearly free and a forgotten flag cannot hide a change. Every
//! write carries a monotonic `rev`, and the first write happens at
//! boot: coming into existence is a transition. Idle apps write
//! nothing, ever — this is an autosave with manners, not a stream.
//!
//! Liveness stays the card's pid question — change and life are
//! different questions and do not mix in one file. The card dies
//! with the app (its `Drop`) and stale cards — crashed apps — are
//! pruned by pid liveness when a new app opens its folder. The
//! dialect is the house RON: tools read the tree with the parser
//! they already have, and tools write documents only.

use std::fs;
use std::path::{Path, PathBuf};

/// The apparatus one application uses to tell: the card it leaves
/// in the folder, the live file it publishes, and the last body
/// published — the value key the dirty check turns on.
#[derive(Debug)]
pub struct Teller {
    name: String,
    pid: u32,
    card: PathBuf,
    live: PathBuf,
    last: Option<String>,
    rev: u64,
}

impl Teller {
    /// Open the telling in `dir`: create the folder, prune the cards
    /// of the departed, and pin this app's card. The card names the
    /// live file's location; the live file itself waits for the
    /// first publish — which should come at boot, because coming
    /// into existence is a transition.
    pub fn open(dir: impl Into<PathBuf>, name: &str) -> Self {
        let dir = dir.into();
        let _ = fs::create_dir_all(&dir);
        let pid = std::process::id();
        prune(&dir, pid);
        let card = dir.join(format!("{name}-{pid}.card.ron"));
        let live = dir.join(format!("{name}-{pid}.live.ron"));
        let root = crate::ron::record_named(&[
            ("name", crate::ron::text(name)),
            ("pid", crate::ron::number(f64::from(pid))),
            (
                "asset_root",
                crate::ron::text(
                    &std::env::current_dir()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default(),
                ),
            ),
            ("live_file", crate::ron::text(&live.display().to_string())),
        ]);
        let header = format!(
            "// {name} is running — this card dies with the app, and is pruned while it sleeps\n"
        );
        let _ = fs::write(&card, crate::ron::to_text_doc(&header, &root, ""));
        Self {
            name: name.into(),
            pid,
            card,
            live,
            last: None,
            rev: 0,
        }
    }

    /// The live file's path — the place guests read the being.
    pub fn live_file(&self) -> &Path {
        &self.live
    }

    /// Tell the app's being: serialize, compare against the last
    /// published body, and write only on change — every write
    /// bumps `rev`. The body excludes the `rev` line, so a
    /// heartbeat-shaped write is impossible: identical means silent.
    /// Returns whether the file changed.
    pub fn publish(&mut self, being: &crate::ron::Val) -> bool {
        let body = crate::ron::to_text_doc("", being, "");
        if self.last.as_deref() == Some(body.as_str()) {
            return false;
        }
        self.rev += 1;
        let header = format!(
            "// {} tells its being — rev {}, pid {}\n",
            self.name, self.rev, self.pid
        );
        match fs::write(&self.live, crate::ron::to_text_doc(&header, being, "")) {
            Ok(()) => {
                self.last = Some(body);
                log::debug!("telling: '{}' published rev {}", self.name, self.rev);
                true
            }
            Err(err) => {
                // The key stays unset so the next frame retries;
                // a dev folder that cannot be written is worth one
                // warning, then silence until it heals.
                log::warn!("telling: '{}' cannot speak ({err})", self.name);
                false
            }
        }
    }
}

/// A card's answer: what a running application announced itself
/// to be, read back from its card. The guest's view of `ls` on the
/// telling folder — a debugger's process list, wearing the house
/// dialect.
#[derive(Debug, Clone, PartialEq)]
pub struct Announcement {
    /// The application's own name for itself.
    pub name: String,
    /// The author's process — liveness is still this question alone.
    pub pid: u32,
    /// Where the app was working: the root its relative paths speak
    /// from.
    pub asset_root: std::path::PathBuf,
    /// The file the app's tellings land in.
    pub live_file: PathBuf,
    /// The card itself — its absence is how the app's death is
    /// noticed.
    pub card: PathBuf,
}

/// The latest telling, read: the revision from the file's header
/// and the being from its root. What a guest knows; how the app
/// came to tell it stays the app's business.
#[derive(Debug)]
pub struct Tiding {
    /// The monotonic count of tellings published.
    pub rev: u64,
    /// The structure the app told, whole.
    pub being: crate::ron::Val,
}

impl Teller {
    /// List the folder as a debugger lists processes: the surviving
    /// cards of living apps, by name then pid. Prunes the departed
    /// first, because listing and healing are the same question
    /// here — who is gone.
    pub fn announce(dir: impl AsRef<Path>) -> Vec<Announcement> {
        let dir = dir.as_ref();
        prune(dir, std::process::id());
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_card = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".card.ron"));
            if !is_card {
                continue;
            }
            let Ok(src) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(doc) = crate::ron::parse_doc(&src) else {
                continue;
            };
            let name = crate::ron::field(&doc.root, "name").map(spoken);
            let pid =
                crate::ron::field(&doc.root, "pid").and_then(|v| v.inline().parse::<u32>().ok());
            let asset_root = crate::ron::field(&doc.root, "asset_root").map(spoken);
            let live_file = crate::ron::field(&doc.root, "live_file").map(spoken);
            let (Some(name), Some(pid), Some(asset_root), Some(live_file)) =
                (name, pid, asset_root, live_file)
            else {
                continue;
            };
            if alive(pid) {
                out.push(Announcement {
                    name,
                    pid,
                    asset_root: PathBuf::from(asset_root),
                    live_file: PathBuf::from(live_file),
                    card: path,
                });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.pid.cmp(&b.pid)));
        out
    }

    /// Read an announcement's latest telling: `None` when the file
    /// refuses to parse or cannot be found — from either side, a
    /// relative path is resolved against the card's own answers,
    /// so guest and app need not share a working directory.
    pub fn tiding(what: &Announcement) -> Option<Tiding> {
        let mut candidates = vec![what.live_file.clone()];
        if what.live_file.is_relative() {
            candidates.push(what.asset_root.join(&what.live_file));
            candidates.push(what.card.parent()?.join(&what.live_file));
        }
        let src = candidates
            .iter()
            .find_map(|path| fs::read_to_string(path).ok())?;
        let rev = src.lines().find_map(|line| {
            let rest = line.split_once("rev ")?.1;
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        })?;
        let doc = crate::ron::parse_doc(&src).ok()?;
        Some(Tiding {
            rev,
            being: doc.root,
        })
    }
}

/// A quoted atom's spoken text: the value's one-line inline, minus
/// the quotes the dialect wears.
fn spoken(v: &crate::ron::Val) -> String {
    v.inline().trim_matches('"').to_string()
}

impl Drop for Teller {
    fn drop(&mut self) {
        // The card is the live half of the announcement: gone with
        // the app, so a folder full of cards is a list of what is
        // running — as accurate as exits are honored.
        let _ = fs::remove_file(&self.card);
    }
}

/// Remove whatever an author no longer with us left: cards whose
/// pid is gone, and the live files of departed pids too — orphaned
/// documents (their card died with the app, the document stayed)
/// are folder litter as much as crashed apps' cards are. A process
/// we cannot signal looks departed — the folder is dev furniture,
/// and the cost of that lie is one deleted file a live stranger
/// republishes on its next telling. What the probe cannot answer is
/// not that lie and does not borrow it: see [`alive`], which
/// declines to guess and so prunes nothing.
fn prune(dir: &Path, me: u32) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        let stem = match name
            .strip_suffix(".card.ron")
            .or_else(|| name.strip_suffix(".live.ron"))
        {
            Some(stem) => stem.to_string(),
            None => continue,
        };
        let Some((_, pid)) = stem.rsplit_once('-') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if pid != me && !alive(pid) {
            let _ = fs::remove_file(dir.join(format!("{stem}.card.ron")));
            let _ = fs::remove_file(dir.join(format!("{stem}.live.ron")));
            log::debug!("telling: pruned what the departed {stem} left behind");
        }
    }
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    // Signal zero, asked of the utility instead of the raw call:
    // `/bin/kill -0` checks the pid and delivers nothing, answering
    // the whole question through its exit status — no syscall, no
    // unsafe, and none of `kill`'s sharp edges: a pid past the
    // platform's ceiling and a pid reused by somebody else's
    // process (an EPERM the raw call also answered as gone) both
    // fail the status the same way the raw call judged them gone.
    // If the probe itself cannot run, the card gets the benefit of
    // the doubt: a broken question prunes nothing.
    std::process::Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_or(true, |status| status.success())
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    // Signal-zero the Windows way, asked of a utility exactly as the
    // unix side asks `/bin/kill` — existence through a stranger's
    // exit, no handle, no unsafe. `tasklist` filtered to the one pid
    // answers in CSV, one row per match; the row's second field is
    // the pid itself, so existence is read as that field and absence
    // as its no-match line. The line's PROSE is localized — German
    // Windows words absence differently — which is why the question
    // is the pid's own quoted CSV field, a fact no locale rewords.
    //
    // The exit status is a different question from the answer, and
    // this is where the two roads part: `/bin/kill`'s status IS the
    // answer (exit 1 = "no such process"), while `tasklist`'s says
    // only whether it managed to answer at all. Measured on this box:
    // a live pid, exit 0 with its row; a dead one in range, exit 0
    // with the no-match `INFO:` line; a pid past what the filter will
    // parse, exit 1 and `ERROR: Invalid query`; and a token that may
    // not read the process list — a sandbox, a service session — exit
    // 1 with `ERROR: Access denied`. The last two are a refused
    // question, and reading one as the first is the loud failure:
    // every app on the machine goes departed at once, so `prune`
    // deletes the cards and tellings of living strangers and
    // `announce` lists an empty folder on a machine full of running
    // apps. Which is the one thing this folder promises not to do: a
    // broken question prunes nothing.
    let asked = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    match asked {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let wanted = pid.to_string();
            text.lines().any(|line| {
                line.split("\",\"")
                    .nth(1)
                    .is_some_and(|field| field == wanted)
            })
        }
        // Refused, unparseable, or not there to be run: no answer, so
        // no verdict — the same benefit of the doubt as a missing
        // `/bin/kill`.
        Ok(_) | Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A claimed folder, never an assumed one: see [`crate::scratch`]
    /// — the assumed name was a Windows sharing violation wearing a
    /// permission mask, and the swallowed `remove_dir_all` meant the
    /// test then narrated a stale folder as if it were its own.
    fn scratch(who: &str) -> PathBuf {
        crate::scratch::claim(&format!("frost_tell_{who}"))
    }

    /// A pid no author of this folder could ever have worn: two short of the
    /// 32-bit ceiling. It names a stale card only on a machine that cannot say
    /// who is gone at all, where its own liveness is beside the point — and it
    /// is worth knowing about for itself, because `tasklist` will not even
    /// parse it (`ERROR: Invalid query`, exit 1). A refused question is not an
    /// answer of *gone*, which is exactly what the first draft of these tests
    /// read it as, on the road where an empty stdout meant no such process.
    const NEVER_MINE: u32 = 4_294_967_290;

    /// A pid this machine has just seen living and no longer sees: a child
    /// spawned, watched, waited for, and released. The departed the folder
    /// exists to recognise, manufactured rather than assumed — and asked of the
    /// same probe the folder asks, so a machine that cannot answer gets the
    /// same answer in both places instead of a test that believes its own
    /// fiction.
    ///
    /// `None` means the probe cannot be trusted here: it called the living
    /// child dead, or would not let go of the dead one, which is what a
    /// sandbox or a restricted token does when the process list is shut to
    /// it. Callers answer a `None` with [`probe_declined`], which is loud on
    /// purpose: an unobserved question is not a passing answer.
    fn a_departed_pid() -> Option<u32> {
        // A child that lives about a second: long enough to be seen living,
        // short enough that nobody notices it.
        #[cfg(windows)]
        let (command, args): (&str, &[&str]) = ("cmd", &["/c", "ping", "-n", "2", "127.0.0.1"]);
        #[cfg(not(windows))]
        let (command, args): (&str, &[&str]) = ("/bin/sleep", &["1"]);
        let mut child = std::process::Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let pid = child.id();
        let lived = alive(pid);
        let _ = child.wait();
        // The last handle goes here, and it is the handle that keeps a dead
        // process in the list: probing before this drop would ask about a pid
        // the child still nominally holds.
        drop(child);
        if !lived {
            return None;
        }
        // The list is not obliged to forget the child the instant the last
        // handle closes, so the departure is watched for rather than caught at
        // one instant: a stubborn entry says something about the machine, not
        // about the child, and half a second of asking is the cheapest way to
        // tell the two apart.
        for _ in 0..20 {
            if !alive(pid) {
                return Some(pid);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        None
    }

    /// The probe could not say who has departed — and that is not a result, so
    /// it cannot be a green either. The GPU test tree already refuses this
    /// shape outright for a missing adapter, and the polygon fingerprint guard
    /// for a chip with no row of its own: a guard that passes on a question it
    /// was never able to ask hides exactly the thing it exists to notice.
    ///
    /// So this fails, with the reason and the hand-probe to run, unless the box
    /// has already declared the limitation in its environment: with
    /// `FROST_TELL_PROBE_UNAVAILABLE` set to anything it returns `true` and the
    /// caller carries on to the promise that does survive a refused question —
    /// a broken question prunes nothing and hides nobody. That is the weaker
    /// claim, and a box that wants it has to ask for it by name rather than
    /// fall into it. A message would not do: libtest hides the output of a test
    /// that passes, which is precisely the case this is for.
    fn probe_declined() -> bool {
        if std::env::var_os("FROST_TELL_PROBE_UNAVAILABLE").is_some() {
            return true;
        }
        panic!(
            "no probe on this machine could tell a living child from a dead one, so the \
             departure these tests turn on went unobserved — which is not a passing answer. \
             Either the process list will not answer here (a sandbox, a restricted token, a \
             service session), or it would not release the child within half a second; either \
             way the probe is the suspect, and the question to ask by hand and read the exit \
             status of is `tasklist /FI \"PID eq <pid>\" /FO CSV /NH` on Windows, \
             `/bin/kill -0 <pid>` elsewhere. A box that knows it \
             cannot answer should say so once, where it is configured: \
             `FROST_TELL_PROBE_UNAVAILABLE=1 cargo test` then checks the promise that survives \
             a refused question — that nothing is deleted and nobody is hidden."
        );
    }

    /// The opening of a telling: boot publishes rev 1 to the live
    /// file, the card names the app and its pid — and the same being
    /// published twice writes once, because an idle app writes
    /// nothing, ever.
    #[test]
    fn a_booting_app_publishes_rev_one_and_silence_is_free() {
        let dir = scratch("boot");
        let mut tell = Teller::open(&dir, "boot");
        let being = crate::ron::record_named(&[("told", crate::ron::text("boot"))]);
        assert!(tell.publish(&being));
        let text = fs::read_to_string(tell.live_file()).unwrap();
        assert!(text.contains("rev 1"), "{text}");
        assert!(dir.join(format!("boot-{}.card.ron", tell.pid)).exists());
        assert!(!tell.publish(&being), "the same body is silence");
        assert_eq!(text, fs::read_to_string(tell.live_file()).unwrap());
        fs::remove_dir_all(&dir).ok();
    }

    /// Change is counted: a different being is a new rev, and the
    /// rev rides the file so a guest that coalesces watcher events
    /// still sees that something happened, and in what order.
    #[test]
    fn a_changed_being_bumps_the_rev() {
        let dir = scratch("rev");
        let mut tell = Teller::open(&dir, "rev");
        assert!(tell.publish(&crate::ron::record_named(&[(
            "beds",
            crate::ron::flag(false)
        )])));
        assert!(tell.publish(&crate::ron::record_named(&[(
            "beds",
            crate::ron::flag(true)
        )])));
        assert!(
            fs::read_to_string(tell.live_file())
                .unwrap()
                .contains("rev 2")
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// The folder heals itself: a card whose author is gone — a
    /// crashed app, here a child this test watched die — is pruned
    /// together with the live file it pointed at, when any new app
    /// opens the folder.
    ///
    /// Where nothing can answer who is gone, the promise that holds is
    /// the other one and is asserted in its place: the folder leaves
    /// every file a stranger left alone, because a file deleted on a
    /// guess is a card and a telling of a running app. That weaker
    /// claim is only available to a box which has said it cannot ask —
    /// see [`probe_declined`] — because a green here that observed
    /// nothing would be reporting on the machine, not on the folder.
    #[test]
    fn a_departed_card_is_pruned_with_its_file() {
        let dir = scratch("prune");
        fs::create_dir_all(&dir).unwrap();
        let departed = a_departed_pid();
        let told = departed.is_some() || probe_declined();
        let gone = departed.unwrap_or(NEVER_MINE);
        let ghost = format!("ghost-{gone}");
        fs::write(dir.join(format!("{ghost}.card.ron")), "// stale\n").unwrap();
        fs::write(dir.join(format!("{ghost}.live.ron")), "// stale\n").unwrap();
        let orphan = format!("orphan-{gone}");
        fs::write(
            dir.join(format!("{orphan}.live.ron")),
            "// document without a card\n",
        )
        .unwrap();
        let tell = Teller::open(&dir, "prune");
        // One expectation per file, in whichever direction the answer went:
        // told, the departed's litter is gone; not told, it all stands.
        assert_eq!(
            dir.join(format!("{ghost}.card.ron")).exists(),
            !told,
            "the departed lose their card — and a question that cannot be answered prunes nothing"
        );
        assert_eq!(
            dir.join(format!("{ghost}.live.ron")).exists(),
            !told,
            "the telling goes with the card it was named in"
        );
        assert_eq!(
            dir.join(format!("{orphan}.live.ron")).exists(),
            !told,
            "orphans heal too, and are left alone by the same broken question"
        );
        assert!(dir.join(format!("prune-{}.card.ron", tell.pid)).exists());
        fs::remove_dir_all(&dir).ok();
    }

    /// The card is the live half of the announcement: it dies with
    /// the telling, and the last published being stays behind —
    /// a dead app leaves a document, not an announcement.
    #[test]
    fn the_card_dies_and_the_document_stays() {
        let dir = scratch("drop");
        {
            let mut tell = Teller::open(&dir, "drop");
            assert!(tell.publish(&crate::ron::record_named(&[(
                "told",
                crate::ron::text("drop")
            )])));
            assert!(dir.join(format!("drop-{}.card.ron", tell.pid)).exists());
        }
        assert!(
            !dir.join(format!("drop-{}.card.ron", std::process::id()))
                .exists()
        );
        assert!(
            dir.join(format!("drop-{}.live.ron", std::process::id()))
                .exists()
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// The guest's side of the protocol, end to end in a scratch
    /// folder: a card of a living pid is listed (a ghost's is not),
    /// and its telling reads back with rev from the header and
    /// structure from the root.
    ///
    /// A machine that cannot answer the liveness question lists the ghost too,
    /// and that is the same promise the folder makes when it prunes: an
    /// unanswered question hides nobody and deletes nothing. Reaching that
    /// branch takes a declared `FROST_TELL_PROBE_UNAVAILABLE`; without one the
    /// test fails where the probe cannot speak, rather than passing a question
    /// nobody was able to ask.
    #[test]
    fn a_guest_reads_the_folder_and_the_telling() {
        let dir = scratch("guest");
        let me = std::process::id();
        let tell = Teller::open(&dir, "guest");
        assert_eq!(tell.name, "guest");
        let departed = a_departed_pid();
        let told = departed.is_some() || probe_declined();
        let gone = departed.unwrap_or(NEVER_MINE);
        fs::write(
            dir.join(format!("ghost-{gone}.card.ron")),
            format!(
                "// stale\n(\n    name: \"ghost\",\n    pid: {gone},\n    asset_root: \"/nowhere\",\n    live_file: \"ghost.live.ron\",\n)\n"
            ),
        )
        .unwrap();
        let cards = Teller::announce(&dir);
        assert_eq!(
            cards.len(),
            if told { 1 } else { 2 },
            "only the living are listed, and where nothing can pronounce a pid dead a stranger's \
             card is listed rather than hidden or deleted"
        );
        let mine = cards
            .iter()
            .find(|card| card.name == "guest")
            .expect("an app standing on a machine has its own card listed there");
        assert_eq!(mine.pid, me);
        // The app had not published yet — nothing to read.
        assert!(Teller::tiding(mine).is_none());
        // Now it tells, by hand: header rev and rooted being.
        let being = crate::ron::record_named(&[("told", crate::ron::text("guest"))]);
        let text = format!(
            "// guest tells its being — rev 7, pid {me}\n{}",
            crate::ron::to_text_doc("", &being, "")
        );
        fs::write(tell.live_file(), &text).unwrap();
        let tiding = Teller::tiding(mine).expect("the telling reads");
        assert_eq!(tiding.rev, 7);
        assert_eq!(
            crate::ron::field(&tiding.being, "told")
                .map(|v| v.inline())
                .as_deref(),
            Some("\"guest\"")
        );
        fs::remove_dir_all(&dir).ok();
    }
}
