//! The ledger: every hill ever won, at its fewest moves, with the walk
//! that earned that number beside it. One entry per hill, the best run
//! only, and a stored walk that always walks exactly the number it sits
//! next to — the law every deposit keeps.
use crate::route::decode_run;

/// The highest move count a hill ever reports. The game itself
/// counts past it without stopping the player; the number is the
/// display's, and a display that must fit a table needs a ceiling:
/// "999 moves" is the widest score row the ledger can ever paint, so
/// the progress page's columns are settled against it once and for
/// all.
pub const MOVES_CAP: i32 = 999;

/// Where the player's descent stands, kept under its own name beside
/// the settings — a file in the launch folder on the desktop, a key in
/// the browser's memory on the web: which hill they last stood on,
/// how deep they ever got (the progress page's list grows down to
/// here and never back), and the ledger the page reads — every hill
/// they warmed, in the fewest moves they ever warmed it. It is
/// written wherever the hill moves — New Game, N, P — and wherever a
/// hill is won, never on launch and never inside `load`, so starting
/// the game leaves no fingerprint and a window-manager kill loses
/// nothing newer than the hill entered or the hill finished. The file
/// counts hills the way the HUD does, from one; each score is a
/// quoted pair, `"hill moves"`, so the hand-written parser stays a
/// walk down two atoms and nothing more.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Progress {
    pub hill: usize,
    /// The deepest hill ever stood on. It only ever grows: walking
    /// back to replay a shallow hill un-stands nowhere.
    pub reached: usize,
    /// Warm hills and their best runs, `(hill, moves)`, zero-based in
    /// the code and sorted by hill; an unwarmed hill owns no entry.
    pub scores: Vec<(usize, i32)>,
    /// The walk that earned each best, `(hill, glyphs)`, in the
    /// walkthrough's alphabet — the routes the *player* ran, never a
    /// solver's. A hill owns at most one, and it changes only when a
    /// better score does, so a route always shows how its number was
    /// honestly earned.
    pub routes: Vec<(usize, String)>,
}

impl Progress {
    const FILE: &'static str = "antifrost.progress.ron";

    /// The player now stands on `hill`: the resume point moves there
    /// and the depth of the descent is asked to remember it, forever
    /// if it is a new low.
    pub fn stood(&mut self, hill: usize) {
        self.hill = hill;
        self.reached = self.reached.max(hill);
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "(\n    hill: {},\n    reached: {},\n    scores: [",
            self.hill + 1,
            self.reached + 1
        );
        if self.scores.is_empty() {
            text.push_str("],\n");
        } else {
            for (hill, moves) in &self.scores {
                text.push_str(&format!("\n        \"{} {}\",", hill + 1, moves));
            }
            text.push_str("\n    ],\n");
        }
        // The routes ride the same file: a number and the walk that
        // earned it belong on the same shelf.
        text.push_str("    routes: [");
        if self.routes.is_empty() {
            text.push_str("],\n)\n");
        } else {
            for (hill, glyphs) in &self.routes {
                text.push_str(&format!("\n        \"{} {}\",", hill + 1, glyphs));
            }
            text.push_str("\n    ],\n)\n");
        }
        text
    }

    /// A hill just finished: the run joins the ledger, and the ledger
    /// keeps the truth it always keeps — one entry per hill, the
    /// fewest moves of any attempt. A warm hill was stood on, so the
    /// descent's depth takes note of it too. The answer says whether
    /// this run *is* the new best — the walk that earns a number is
    /// the walk worth remembering.
    pub fn record(&mut self, hill: usize, moves: i32) -> bool {
        // The ledger writes a number, so the number has an upper
        // limit: a hill stays winnable past it and the game keeps
        // counting itself, but the ledger — and every layout that
        // trusts its width — sees `MOVES_CAP` at the most.
        let moves = moves.min(MOVES_CAP);
        self.reached = self.reached.max(hill);
        match self.scores.iter_mut().find(|(h, _)| *h == hill) {
            Some(entry) => {
                if moves < entry.1 {
                    entry.1 = moves;
                    true
                } else {
                    false
                }
            }
            None => {
                self.scores.push((hill, moves));
                self.scores.sort();
                true
            }
        }
    }

    /// The best run's own walk, when the ledger keeps one for `hill`.
    pub fn route_of(&self, hill: usize) -> Option<&str> {
        self.routes
            .iter()
            .find(|(h, _)| *h == hill)
            .map(|(_, g)| g.as_str())
    }

    /// The walk that earned the best number, into the ledger's
    /// keeping. Only letters a hill could walk are kept: a hand at
    /// the file may change a score, but it cannot write a route the
    /// game would ever show.
    pub fn remember(&mut self, hill: usize, glyphs: &str) {
        if glyphs.is_empty() || decode_run(glyphs).is_none() {
            return;
        }
        match self.routes.iter_mut().find(|(h, _)| *h == hill) {
            Some(entry) => entry.1 = glyphs.to_string(),
            None => {
                self.routes.push((hill, glyphs.to_string()));
                self.routes.sort();
            }
        }
    }

    /// A won hill's whole deposit: the number, and the walk that
    /// earned it. The ledger keeps its law — fewest moves — and the
    /// walk rides only the number it walks: every new best carries
    /// its walk in, and a hill whose number predates the recordings
    /// backfills its walk when a run ties that number. A tie over an
    /// existing walk changes nothing (the elder walk walks the same
    /// number, and churn would lie about novelty); a worse run
    /// carries nothing anywhere.
    pub fn record_run(&mut self, hill: usize, moves: i32, glyphs: &str) {
        let improved = self.record(hill, moves);
        let walked = moves.min(MOVES_CAP);
        let keeps = self
            .scores
            .iter()
            .find(|(h, _)| *h == hill)
            .is_some_and(|(_, m)| *m == walked);
        if keeps && (improved || self.route_of(hill).is_none()) {
            self.remember(hill, glyphs);
        }
    }

    pub fn parse(src: &str) -> Progress {
        let Ok(root) = frost::ron::parse(src) else {
            return Progress::default();
        };
        let hill = match frost::ron::field(&root, "hill") {
            Some(frost::ron::Val::Atom(text, _)) => {
                text.trim_matches('"').parse::<usize>().unwrap_or(1)
            }
            _ => 1,
        };
        let reached = match frost::ron::field(&root, "reached") {
            Some(frost::ron::Val::Atom(text, _)) => {
                text.trim_matches('"').parse::<usize>().unwrap_or(1)
            }
            // A file from before the descent kept its depth says no
            // more than where it stands: the stood-on hill is the
            // deepest it ever named.
            _ => hill,
        };
        // The ledger survives what a hand may not: entries out of
        // order fold to sorted, a hill named twice keeps its better
        // number, and garbage in the list is simply not a score.
        // A file from before the ledger carries no `scores` at all —
        // which is no scores, not a broken file.
        let mut scores: Vec<(usize, i32)> = match frost::ron::field(&root, "scores") {
            Some(frost::ron::Val::Seq { items, .. }) => items
                .iter()
                .filter_map(|item| match &item.val {
                    frost::ron::Val::Atom(text, _) => {
                        let mut pair = text.trim_matches('"').split_whitespace();
                        let hill = pair.next()?.parse::<usize>().ok()?.checked_sub(1)?;
                        let moves = pair.next()?.parse::<i32>().ok()?;
                        // The file's scores pass the same turnstile
                        // the record does: the cap holds whatever
                        // hand went to the keyboard after the game.
                        Some((hill, moves.clamp(0, MOVES_CAP)))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        scores.sort();
        scores.dedup_by(|a, b| {
            if a.0 == b.0 {
                // `a` is the entry being dropped; it dies passing its
                // better number on to the one that stays.
                b.1 = b.1.min(a.1);
                true
            } else {
                false
            }
        });
        // One-based in the file, zero-based in the code — and nothing
        // sits below the first hill. The depth of the descent is at
        // least the hill it stands on and at least the deepest warm
        // hill: a warm hill was stood on by definition, which folds
        // the pre-`reached` files (a score deeper than the standing
        // hill) onto the truth they already carry.
        let hill = hill.saturating_sub(1);
        let reached = reached
            .saturating_sub(1)
            .max(hill)
            .max(scores.iter().map(|(h, _)| *h).max().unwrap_or(0));
        // The routes read themselves the same forgiving way: a file
        // from before replays carries none, a hill named twice keeps
        // the first walk mentioned, and any string the hill could not
        // walk is not a route at all.
        let mut routes: Vec<(usize, String)> = match frost::ron::field(&root, "routes") {
            Some(frost::ron::Val::Seq { items, .. }) => items
                .iter()
                .filter_map(|item| match &item.val {
                    frost::ron::Val::Atom(text, _) => {
                        let mut pair = text.trim_matches('"').split_whitespace();
                        let hill = pair.next()?.parse::<usize>().ok()?.checked_sub(1)?;
                        let glyphs = pair.next()?;
                        if pair.next().is_some() || decode_run(glyphs).is_none() {
                            return None;
                        }
                        Some((hill, glyphs.to_string()))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        routes.sort();
        routes.dedup_by(|a, b| a.0 == b.0);
        Progress {
            hill,
            reached,
            scores,
            routes,
        }
    }

    pub fn load() -> Progress {
        frost::store::load(Self::FILE)
            .map(|src| Progress::parse(&src))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Err(err) = frost::store::save(Self::FILE, &self.text()) {
            log::warn!("antifrost: could not save {:?}: {err}", Self::FILE);
        }
    }
}
