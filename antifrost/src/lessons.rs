//! The school hills' lessons: the cards each scripted hill deals,
//! the authored scripts themselves, and the tour state machine —
//! which hill teaches, whether it has been warmed, when the cold
//! tour and the oops deck deal, and what the rail and the marks
//! show while a card is open.

use crate::*;

/// One card of a school hill's lesson: the act the hill is waiting
/// for, and the word it says while waiting. A school hill deals its
/// script card by card — while the played log spells the script's
/// prefix, the next card shows itself: a mark over the queen's head
/// (one chevron to walk, two to shove) and its word on the note's
/// floor line. The hill keeps its side of the bargain: while a card
/// is open the rail lets that card's act through and no other — every
/// other key, click or shove is turned away at the gate with a word —
/// so the pupil can neither stray nor strand the lesson; `Z` puts the
/// card back, `R` deals the deck again.
pub(crate) struct Lesson {
    pub(crate) act: game::Act,
    pub(crate) word: &'static str,
}

/// What the rail says when it turns an act away — the hill's own
/// voice, in the note's letters, pointing back at the mark.
pub(crate) const RAIL_NOTE: &str = "follow the mark";

/// First Warm's script: the walkthrough's winning walk — step right,
/// climb to the corridor beside `r`, shove it into `R` with the
/// shift-push, one step home. The colony's first verb: the dunk.
pub(crate) const FIRST_WARM_LESSON: [Lesson; 5] = [
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Up),
        word: "move up",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Up),
        word: "move up",
    },
    Lesson {
        act: game::Act::Push(game::Dir::Right),
        word: "hold shift and push right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left home",
    },
];

/// Attunement's script: five steps due east down the straight hall —
/// over the empty `R` tile, into `r` (who joins the line and, in the
/// queen's own wake, falls asleep on `R` with no step to spare), and
/// on to her bed at the hall's end. The lesson is the wake lock:
/// walking on is what carries an ant home.
pub(crate) const ATTUNEMENT_LESSON: [Lesson; 5] = [
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right collect the ant",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right to your bed",
    },
];

/// First Cold's script: the walkthrough's nine acts — grab `r`, at
/// once leave it behind so it sits on the corridor, post it home with
/// two shoves (the second dunks it onto `R`), then the long walk
/// home. The lesson is the leave-behind, and the dunk shown before
/// it is ever needed.
pub(crate) const FIRST_COLD_LESSON: [Lesson; 9] = [
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left collect the ant",
    },
    Lesson {
        act: game::Act::Detach,
        word: "press space leave it behind",
    },
    Lesson {
        act: game::Act::Push(game::Dir::Right),
        word: "hold shift and push right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Push(game::Dir::Right),
        word: "hold shift and push right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left to your bed",
    },
];

/// The hill whose first crossing is taught by the cold tour.
pub(crate) const FIRST_COLD_HILL: usize = 2;

/// What the cold tour says the moment the walk has spent its
/// par — the first move a cleverer walk would not have needed.
pub(crate) const COLD_NOTE: &str = "you are now getting cold";

/// The hill's answer to a tour's own praise — both double-dealt
/// hills keep it — the praise splash spins first like on any win,
/// then this hangs big in the centre of the world for
/// `BETTER_HOLD` seconds, and the hill restarts on the better
/// script. In the praise words' own case, because it answers them.
pub(crate) const BETTER_NOTE: &str = "YOU CAN DO BETTER";

/// How long the answer holds on the screen before the hill deals
/// again: two seconds and a half, a held breath — the measure the
/// tour's answer and both of the oops deck's admissions dance to.
pub(crate) const BETTER_HOLD: f32 = 2.5;

/// The answer's size: big — twice the HUD's banner — though it
/// answers in a sentence, not the splash's single word.
pub(crate) const BETTER_EM: f32 = 72.0;

// The deck that precedes the cold tour: First Cold makes the pupil
// make its mistakes for it. The card asks for the wrong step —
// right, away from everything the walk will want — and the rail,
// which has never once refused a lesson, refuses everything else;
// the moment the step lands the hill admits it, big and centred in
// the answer's own letters and hold, and when the words fade the
// undo mark goes up: a chevron pointing back the way the wrong step
// came, worded below. The Z takes the step back — and the trap
// re-baits itself: the same wrong step is asked again, and again,
// and this time the second landing earns the deeper admission and
// the rewind mark. The R that restarts the hill retires the deck
// for good and hands the fresh hill to the long walk — the
// answer's own words wait at the tour's end, where they are
// earned — so the undo is learned before the walk, the restart
// before the ruin, and both by making the mistake, never by being
// told.
pub(crate) const MISTAKE_CARD: Lesson = Lesson {
    act: game::Act::Move(game::Dir::Right),
    word: "move right",
};

/// The admission, in the praise words' case and size. The hill
/// owns the mistake — *I* made it — because a pupil who watched
/// the hill choose wrong still pressed the key, and being scolded
/// for obeying is how a lesson turns into a grudge.
pub(crate) const MISTAKE_NOTE: &str = "OOPS I MADE A MISTAKE";

/// The second card of the deck: the same trap re-baited. The word
/// knows it is a repeat now — the hill does not even pretend.
pub(crate) const DOUBLE_CARD: Lesson = Lesson {
    act: game::Act::Move(game::Dir::Right),
    word: "move right again",
};

/// The second admission, for the mistake twice as deep: two words,
/// because at this depth a sentence would be an optimism.
pub(crate) const NOT_AGAIN_NOTE: &str = "NOT AGAIN";

/// The word under the rewind — two chevrons stepped back, the
/// walk's own alphabet spelling the media-keys' rewind.
pub(crate) const RESTART_WORD: &str = "r to restart";

/// The word under the undo mark, lowercase like every foot word;
/// named here so the sulking and the Z agree on the spelling.
pub(crate) const UNDO_WORD: &str = "z for undo";

/// The same word in the pad's voice. Only the key-named words have
/// a second telling — every other card speaks verbs the pad shares
/// — and the deck test walks the scripts so a key-named word can
/// never ship without its pad telling.
pub(crate) fn pad_word(word: &str) -> &str {
    match word {
        "hold shift and push right" => "hold r1 and push right",
        "press space leave it behind" => "square leaves it behind",
        "z for undo" => "l1 to undo",
        "r to restart" => "l2 to restart",
        other => other,
    }
}

/// The send-off: the cheap dig watches the pupil take the dig for
/// the first time and says the last kind word before the cold
/// hills proper begin. It is a course well taught by now — four
/// hills, five decks, two keys, one dig — and the hill knows it.
pub(crate) const SEND_OFF_NOTE: &str = "I KNOW YOU CAN DO IT";

/// The cold tour: the walk that works before the clever one — and
/// after the oops deck, which deals first: a forced mistake, the
/// admission, the undo. The tour takes up only once the hill has
/// stumbled and been stepped back.
/// Grab `r` and walk right along the top — the wake carries the
/// ant home mid-stride, asleep on `R` while the walk goes on —
/// then take the long way yourself: the wall, down, the wall the
/// other side, and up to bed. Eighteen moves where nine will do;
/// the move past par says the walk is getting cold, and the
/// finish gets the praise splash like any win — then the hill
/// answers it, big and centred, and restarts on the better
/// script. The hill warms on the way, and a warm hill never
/// deals this deck again.
pub(crate) const COLD_TOUR_LESSON: [Lesson; 18] = [
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left collect the ant",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right over the bed",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right red warms behind",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right to the wall",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Down),
        word: "move down",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Down),
        word: "move down to the floor",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left to the wall",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Up),
        word: "move up",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Up),
        word: "move up to your bed",
    },
];

/// The cheap dig: the fourth hill, and the hill the dig belongs
/// to. The dirt cell in its wall is the lesson — walk into dirt
/// and it gives way, one move, one budget — so the hill deals
/// twice like the cold ring: first the honest fourteen that walk
/// around the dirt and miss it entirely, then the answer's words,
/// then the ten that dig.
pub(crate) const DIG_TUTOR_HILL: usize = 3;

/// The cheap dig's tour: fourteen moves that succeed — collect
/// `r`, march on to the east shaft, ride it to the trail, walk
/// home along the top while the wake carries `r` onto `R` — a
/// real win at four moves' waste, and the praise splash is
/// earned by it. Then the hill answers itself, big and centred,
/// and deals the cheap way beneath the words.
pub(crate) const DIG_TOUR_LESSON: [Lesson; 14] = [
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right collect the ant",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right to the shaft",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Up),
        word: "move up the shaft",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Up),
        word: "move up to the trail",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left red rides behind",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left red packs onto r",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left home to w",
    },
];

/// The cheap dig taught cheap: collect `r`, then walk into the
/// dirt cell in the wall — walking into dirt is the dig, one
/// move and one budget — step out onto `R`, and walk home while
/// the wake packs `r` onto its bed. Ten moves, and every one of
/// them needed.
pub(crate) const DIG_LESSON: [Lesson; 10] = [
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Right),
        word: "move right collect the ant",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Up),
        word: "walk up dig through the dirt",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Up),
        word: "move up onto the red bed",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left red packs on the way",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left",
    },
    Lesson {
        act: game::Act::Move(game::Dir::Left),
        word: "move left home to w",
    },
];

/// The descent's school hills and their scripts: every lesson the
/// descent teaches is here, in the hills that teach it. Anywhere
/// else the hill has no cards and keeps no rail. Two hills deal
/// double — First Cold and the cheap dig both show the long way
/// first and answer their own praise with the better script;
/// `Demo::script` picks the deck.
pub(crate) fn lesson_script(hill: usize) -> Option<&'static [Lesson]> {
    match hill {
        0 => Some(&FIRST_WARM_LESSON),
        1 => Some(&ATTUNEMENT_LESSON),
        2 => Some(&FIRST_COLD_LESSON),
        3 => Some(&DIG_LESSON),
        _ => None,
    }
}

impl Demo {
    /// The lesson card the hill underfoot is teaching right now: the
    /// script's next card while the played log spells the script's
    /// prefix — and nothing at all elsewhere. Not on the forty-six
    /// hills that keep no script, not behind a page, not during a
    /// showing (whose walk is the ledger's, not a pupil's), not once
    /// the hill sleeps. The rail below makes the prefix check a
    /// formality on a played hill — no act but a card's ever reaches
    /// the log — yet the card reads the log, not the rail, so a hand
    /// at the code stays as untrusted as a hand at the file.
    pub(crate) fn lesson(&self) -> Option<&'static Lesson> {
        if self.page != Page::Game || self.replay.is_some() {
            return None;
        }
        // The oops deck first: First Cold, un-warmed and un-humbled,
        // opens on the wrong foot — the card asks for the step the
        // hill must not take; once taken, it stands there sulking
        // with no card at all, and the marks it shows then are the
        // undo's, drawn outside any card. The Z that steps back
        // lands on the re-baited trap: the same wrong step asked
        // again, and the second landing is the deeper sulk — the
        // restart's, not the undo's.
        if self.oopsy_visit() {
            let game = self.game.as_ref()?;
            return match game.log.as_slice() {
                [] => Some(&MISTAKE_CARD),
                [a] if *a == game::Act::Move(game::Dir::Right) && self.mistake_shown => {
                    Some(&DOUBLE_CARD)
                }
                _ => None,
            };
        }
        let script = self.script()?;
        let game = self.game.as_ref()?;
        if game.won || game.log.len() >= script.len() {
            return None;
        }
        game.log
            .iter()
            .zip(script)
            .all(|(a, l)| *a == l.act)
            .then_some(&script[game.log.len()])
    }

    /// Has the ledger ever held this hill?
    pub(crate) fn warmed(&self, hill: usize) -> bool {
        self.progress.scores.iter().any(|(h, _)| *h == hill)
    }

    /// The cold tour: First Cold's first crossing, taught the long
    /// way round. The tour ends the moment the ledger holds the hill
    /// — however it was warmed — and never returns to it.
    pub(crate) fn cold_tour(&self) -> bool {
        self.idx == FIRST_COLD_HILL && !self.warmed(FIRST_COLD_HILL)
    }

    /// The cheap dig's tour: the same first-crossing bargain as
    /// the cold ring — the long way taught first, the better
    /// script dealt beneath the answer's words.
    pub(crate) fn dig_tour(&self) -> bool {
        self.idx == DIG_TUTOR_HILL && !self.warmed(DIG_TUTOR_HILL)
    }

    /// A tour is live underfoot: a hill whose first crossing it
    /// teaches the long way round, praise and all. Two hills keep
    /// such a deck — the cold ring and the cheap dig — and they
    /// share the machinery: no won page, one splash, the answer's
    /// words, a redeal on the better script.
    pub(crate) fn tour(&self) -> bool {
        self.cold_tour() || self.dig_tour()
    }

    /// The oops deck is live: First Cold before it has been finished
    /// *and* before the hill has been restarted out of its own
    /// double mistake. The retirement outlives every restart of the
    /// visit; nothing else here does.
    pub(crate) fn oopsy_visit(&self) -> bool {
        self.cold_tour() && !self.oops_retired
    }

    /// The forced mistake, standing on the board: the wrong step is
    /// the walk's whole story, the hill answers nothing — every
    /// rail-carrying input is refused, and only the Z, which needs
    /// no rail, is heard — and the undo that takes it back re-baits
    /// the trap.
    pub(crate) fn mistake_standing(&self) -> bool {
        self.oopsy_visit()
            && !self.mistake_shown
            && self
                .game
                .as_ref()
                .is_some_and(|g| g.log.len() == 1 && g.log[0] == game::Act::Move(game::Dir::Right))
    }

    /// The same mistake twice as deep: the pair of wrong steps has
    /// landed, and this time no undo will do — the hill sulks for
    /// the rewind, and only the R is heard.
    pub(crate) fn disaster_standing(&self) -> bool {
        self.oopsy_visit()
            && self.mistake_shown
            && self.game.as_ref().is_some_and(|g| {
                g.log.len() == 2
                    && g.log
                        .iter()
                        .all(|a| *a == game::Act::Move(game::Dir::Right))
            })
    }

    /// Either sulk: the hill standing on a mistake it forced,
    /// admitting it big, and answering nothing until the taught key
    /// comes — the undo's, or the restart's.
    pub(crate) fn sulking(&self) -> bool {
        self.mistake_standing() || self.disaster_standing()
    }

    /// The lesson script for the hill underfoot: on the cold ring
    /// before it has ever been finished, the cold tour — the long
    /// walk, taught first; on a warmed ring, and on every other
    /// school hill, the hill's own script. The cheap dig deals on
    /// the same bargain: its tour until the ledger holds the hill,
    /// the dig ever after.
    pub(crate) fn script(&self) -> Option<&'static [Lesson]> {
        if self.cold_tour() {
            return Some(&COLD_TOUR_LESSON);
        }
        if self.dig_tour() {
            return Some(&DIG_TOUR_LESSON);
        }
        lesson_script(self.idx)
    }

    /// The rail: does the tutorial let this act through? On a school
    /// hill, while a card is open, only the card's own act answers;
    /// every verb the script does not call for — the wrong way to
    /// walk, a walk where a shove is asked, a shove where the card
    /// says walk, a leave-behind nobody asked for — is refused at the
    /// gate. No card, no rail: the other forty-six hills answer to
    /// everything, and so does a hill whose lesson is finished. One
    /// exception sulks outside any card: the hill standing on its
    /// own mistake admits nothing at all — the Z, or the R, which
    /// need no rail are the only voices it waits for. And while one
    /// of the held admissions still hangs at the centre, the hill
    /// under it waits quietly too: no act answers until the words
    /// have had their breath.
    pub(crate) fn rail_opens(&self, a: game::Act) -> bool {
        !self.sulking() && self.better_t <= 0.0 && self.lesson().is_none_or(|c| c.act == a)
    }

    /// The card made visible: the mark (or marks) of its act and
    /// its word, drawn by the same painter the sulking hill borrows
    /// for the undo.
    pub(crate) fn draw_lesson(&self, ctx: &mut Context, card: &Lesson) {
        let dir = match card.act {
            game::Act::Move(d) | game::Act::Push(d) => face_of(d),
            // A leave-behind aims nowhere: its mark points down —
            // the ant stepping off the line, out of the wake, and
            // left where it stands.
            game::Act::Detach => [0.0, -1.0],
        };
        // One chevron walks, two shove — the alphabet's own case
        // distinction, told in marks instead of letters.
        let marks = usize::from(matches!(card.act, game::Act::Push(_))) + 1;
        self.draw_marks(ctx, dir, marks, self.word_for(card.word));
    }

    /// The marks and the foot word, whoever calls: a chevron aimed
    /// the given way hangs over the queen's head — riding the hill,
    /// since it follows her gliding `player_at` — doubled for a
    /// shove, the second mark stepped back along the aim; the word
    /// sits on the note's floor-side seat, standing a line higher
    /// when a note is up with it, the same courtesy the showing
    /// pays.
    pub(crate) fn draw_marks(&self, ctx: &mut Context, dir: [f32; 2], marks: usize, word: &str) {
        let (_, h) = self.view(ctx);
        let [sx, sy] = self.screen([self.player_at[0], self.player_at[1] + TILE * 0.95]);
        let ang = dir[1].atan2(dir[0]) - std::f32::consts::FRAC_PI_2;
        let (s, c) = (ang.sin(), ang.cos());
        for k in 0..marks {
            let at = [sx - dir[0] * k as f32 * 26.0, sy - dir[1] * k as f32 * 26.0];
            // The base chevron points up; spin it onto the aim.
            let spin = |p: [f32; 2]| [p[0] * c - p[1] * s + at[0], p[0] * s + p[1] * c + at[1]];
            ctx.scene().root.children.push(Box::new(SceneNode {
                order: UI + 1.0,
                modulate: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.9,
                },
                shape: Some(frost::Shape::Polyline {
                    points: vec![spin([-17.0, -9.0]), spin([0.0, 9.0]), spin([17.0, -9.0])],
                    width: 5.0,
                    color: C_TEXT,
                }),
                ..Default::default()
            }));
        }
        let seat = if self.note_t > 0.0 { 64.0 } else { 26.0 };
        ctx.text(
            0.0,
            -h / 2.0 + seat,
            &self.font,
            word.to_string(),
            34.0,
            600.0,
            C_NOTE,
            UI + 1.0,
        );
    }
}

/// The deck's words told to both hands.
#[cfg(test)]
mod lesson_tests {
    use super::*;

    #[test]
    fn every_key_named_word_has_a_pad_telling() {
        let deck: Vec<&[Lesson]> = vec![
            &FIRST_WARM_LESSON,
            &ATTUNEMENT_LESSON,
            &FIRST_COLD_LESSON,
            &COLD_TOUR_LESSON,
            &DIG_TOUR_LESSON,
            &DIG_LESSON,
        ];
        // The words that name a keyboard key: each must change
        // voice for the pad, or the pad's player is read a key
        // their hands will never touch. "onto r" and "home to w"
        // name beds, not keys — only these four strings speak keys.
        let key_words = [
            "hold shift and push right",
            "press space leave it behind",
            UNDO_WORD,
            RESTART_WORD,
        ];
        let mut seen = std::collections::HashSet::new();
        let cards = deck
            .iter()
            .flat_map(|script| script.iter())
            .chain([MISTAKE_CARD, DOUBLE_CARD].iter());
        for card in cards {
            seen.insert(card.word);
            if key_words.contains(&card.word) {
                assert_ne!(
                    pad_word(card.word),
                    card.word,
                    "{:?} ships without a pad telling",
                    card.word
                );
            } else {
                assert_eq!(
                    pad_word(card.word),
                    card.word,
                    "{:?} names no key and must not be retold",
                    card.word
                );
            }
        }
        // And the tellings themselves.
        assert_eq!(
            pad_word("hold shift and push right"),
            "hold r1 and push right"
        );
        assert_eq!(
            pad_word("press space leave it behind"),
            "square leaves it behind"
        );
        assert_eq!(pad_word(UNDO_WORD), "l1 to undo");
        assert_eq!(pad_word(RESTART_WORD), "l2 to restart");
        // The whole deck is font-safe either way.
        for word in seen {
            for w in [word, pad_word(word)] {
                assert!(
                    w.chars().all(|c| c.is_ascii_alphanumeric() || c == ' '),
                    "{w:?} must wear only glyphs the font owns"
                );
            }
        }
    }
}
