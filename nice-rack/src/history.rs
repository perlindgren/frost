//! Undo and redo.
//!
//! The stack itself knows nothing about racks: it holds a row of commands,
//! runs them backwards when asked to undo and forwards when asked to redo, and
//! nothing else. What a command *is* belongs to whoever uses the stack, which
//! is why the stack is generic and the vocabulary of the rack sits beside it in
//! this file rather than inside it. An effect parameter is a different command
//! in a different vocabulary — a card, a parameter index, the two values — and
//! it can be added to [`RackStep`] without touching a line of [`History`],
//! because the stack never learns what its commands mean.
//!
//! Two rules carry most of the weight.
//!
//! A command is stored as *both* of its values, never as something that has to
//! be worked out again at replay time. The obvious alternative — record "the
//! operator moved a card", then at undo time ask the rack what used to be
//! there — needs the rack to remember what it is about to erase, which is
//! exactly the thing a roster edit throws away. Storing the pair makes the
//! undo independent of the state of the world when it arrives, which is what
//! lets it be applied one hundred edits later, or after a load.
//!
//! And a command is what the operator did, not what the mouse did. A gain
//! drag is one undo, not one per frame; a click is one; a drag that comes back
//! to where it started is none. [`Command::gesture`] is how a command says
//! which continuous act it belongs to, [`Command::fold`] is how the tip of the
//! stack absorbs a later frame of the same act, and [`Command::is_void`] is how
//! a folded command that turned out to change nothing is taken back out.
//! Without the fold, three seconds of dragging a trim would need three hundred
//! presses of Ctrl-Z, which is not undo, and with the fold but without
//! `is_void`, a slip of the mouse that ends where it began would leave a step
//! that Ctrl-Z appears to have lost.

use crate::gui::short;
use crate::runtime::Card;

/// How many steps the rack remembers. A roster step carries two copies of the
/// card list, and a card is a name, a path and a few integers — the plugin
/// itself is never copied — so sixty-four steps is a few hundred kilobytes and
/// more depth than anyone rewinds. The point of a bound is not memory: it is
/// that the stack cannot grow without limit across a long set.
pub(crate) const DEPTH: usize = 64;

/// Which way a step is being run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Undo,
    Redo,
}

impl Direction {
    /// The word for the status line, so the line says which way the rack just
    /// went rather than leaving it to be guessed.
    pub(crate) fn word(&self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
        }
    }
}

/// One reversible act, in the only terms the stack needs.
pub(crate) trait Command: Sized {
    /// What was done, as a short phrase in the forward direction — "bypass
    /// Tube Drive" — since that is the act an undo takes back and the act a
    /// redo puts back. It names the act, not the direction: the direction comes
    /// from [`Direction::word`].
    fn label(&self) -> String;
    /// Which continuous act this belongs to, as a tag and two words, or `None`
    /// for an act that is already whole. Two commands with the same gesture
    /// fold into one, so a drag records its value a hundred times and the
    /// history sees a single step. `None` never matches anything, which is
    /// right for a click: two bypasses of two cards, or of the same card twice,
    /// are two acts, not one long one.
    fn gesture(&self) -> Option<(u8, u64, u64)>;
    /// Absorb a later command of the same gesture, keeping this command's
    /// starting value and taking the other's ending value. Callers only ever
    /// call this when the two gestures matched, so a variant that has no
    /// gesture has nothing to do.
    fn fold(&mut self, later: &Self);
    /// Whether this command leaves the rack exactly as it found it, which is
    /// how a gesture that came back to where it started disappears from the
    /// history instead of becoming a step that does nothing.
    fn is_void(&self) -> bool {
        false
    }
}

/// The stack: what has been done, and what has been taken back.
pub(crate) struct History<C> {
    past: Vec<C>,
    future: Vec<C>,
    depth: usize,
}

impl<C: Command> History<C> {
    /// A stack that remembers `depth` steps and has done nothing yet.
    pub(crate) fn new(depth: usize) -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            depth: depth.max(1),
        }
    }

    /// Say that the operator just did something.
    ///
    /// Anything that had been undone is dropped first: the history stops being
    /// a line at that moment, and the rack does not pretend that a branch
    /// nobody took can still be redone. A command that changes nothing is
    /// dropped without touching anything else — a slip of the mouse is not
    /// allowed to eat the redo stack. And a frame of a gesture already at the
    /// tip is folded into it rather than added.
    pub(crate) fn record(&mut self, command: C) {
        if command.is_void() {
            return;
        }
        self.future.clear();
        if let Some(tip) = self.past.last_mut()
            && let (Some(here), Some(thence)) = (tip.gesture(), command.gesture())
            && here == thence
        {
            tip.fold(&command);
            // A drag that wandered off and came back leaves a step whose two
            // values are equal. Taking the tip out is what makes Ctrl-Z mean
            // "the last thing that actually changed something".
            if tip.is_void() {
                self.past.pop();
            }
            return;
        }
        self.past.push(command);
        if self.past.len() > self.depth {
            self.past.remove(0);
        }
    }

    /// The step Ctrl-Z would take back, without taking it back.
    ///
    /// Reading and moving are kept apart on purpose. The rack answers a
    /// structural step with yes or no — a roster that no longer fits the
    /// jacks is refused, and the audio engine may still be busy with the last
    /// edit — and a step the rack refused must not have happened to the
    /// history either, or the second press of Ctrl-Z would quietly skip over
    /// the thing the operator is still trying to undo. So the caller reads the
    /// tip, tries it, and calls [`History::commit`] only once the rack has
    /// moved.
    pub(crate) fn undo(&self) -> Option<&C> {
        self.past.last()
    }

    /// The step Ctrl-Shift-Z would put back, without putting it back.
    pub(crate) fn redo(&self) -> Option<&C> {
        self.future.last()
    }

    /// Move the tip across, now that the rack has actually moved.
    pub(crate) fn commit(&mut self, direction: Direction) {
        match direction {
            Direction::Undo => {
                if let Some(command) = self.past.pop() {
                    self.future.push(command);
                }
            }
            Direction::Redo => {
                if let Some(command) = self.future.pop() {
                    self.past.push(command);
                }
            }
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.past.last().is_some()
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.future.last().is_some()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.past.len()
    }
}

impl<C: Command> Default for History<C> {
    fn default() -> Self {
        Self::new(DEPTH)
    }
}

// The rack's own vocabulary. Every way the operator changes a rack that is not
// the sound itself ends here, which is what makes the history trustworthy
// enough to be worth a keystroke: a step is recorded by the same three lines
// that write the value to its owners, so nothing can change without being
// written down, and nothing can be written down that did not change.
//
// A roster step carries the whole card list on both sides. That is the coarse
// answer, and it is the right one here. The rack's only door for a structural
// change is "here is the new card list", which is also how the graph editor,
// the picker and a dragged card all speak; recording the pair at that door
// means a rearrange, a local fork and a removal are all one shape and none of
// them needs a private grammar of inverse edits that could disagree with the
// editor's own.
//
// What the trims and switches record is the value, not the gesture, so a
// re-applied step needs nothing that the rack might have forgotten. What a
// roster step does not carry is the plugin's own saved state: the rack drops a
// card's bytes when the card leaves, so undoing a removal brings the card back
// as a fresh instance of the same plugin with its default parameters. Everything
// the rack itself holds for that card — its trims, its wire switches, its
// bypass and pause — survives, because those maps keep their entries.

/// One thing the rack can undo.
///
/// Cloned, because the caller reads the tip of the stack, hands it to the rack,
/// and only then tells the stack to move; a card list is a handful of names,
/// paths and integers, and the plugin behind it is never copied.
#[derive(Clone)]
pub(crate) enum RackStep {
    /// The card list, before and after.
    Roster { before: Vec<Card>, after: Vec<Card> },
    /// One wire's gain. `side` is the rack's own convention, `is_output`: the
    /// output side is `true` and the input side `false`, so the same word means
    /// the same wire everywhere in the rack. The two jacks keep their half of
    /// the pair — the wire leaving the input jack is on that jack's output
    /// side, the wire arriving at the master is on the master's input side —
    /// but they are named for themselves in the label, where the side would
    /// only add a word nobody reads.
    Trim {
        name: String,
        id: u64,
        side: bool,
        from: f32,
        to: f32,
    },
    /// One wire's switch: whether it stands open.
    Circuit {
        name: String,
        id: u64,
        side: bool,
        from: bool,
        to: bool,
    },
    /// Whether one card is crossed out and passing its dry signal through.
    Bypass {
        name: String,
        id: u64,
        from: bool,
        to: bool,
    },
    /// Whether one card is holding its output silent without letting go of the
    /// plugin.
    Pause {
        name: String,
        id: u64,
        from: bool,
        to: bool,
    },
}

/// Which of the two sides of a card a wire belongs to, in words.
fn side_word(side: bool) -> &'static str {
    if side { "output" } else { "input" }
}

/// Where a trim or a switch sits, named the way the operator sees it. The two
/// runs that hang on nothing instead of a card say so themselves.
fn wire_word(name: &str, side: bool) -> String {
    match (name, side) {
        ("Output", _) => "output wire".into(),
        ("Input", _) => "input wire".into(),
        (name, side) => format!("{name} {} wire", side_word(side)),
    }
}

/// The seat a card holds in the rack: which stage it stands on, which chain it
/// chains along, which columns its lane runs between, and which card it is.
/// Two card lists that agree on this, card for card, describe the same rack,
/// which is what makes it safe to call such a step empty.
type Seat = (u64, u64, Option<u64>, Option<(u64, u64)>);
/// Whether two sets of seats describe one rack. The order the card list is
/// stored in is not part of a rack — the rack is the seats, not the vector —
/// so the two lists are sorted before they are compared, and a step that only
/// shuffles the storage order is recognised as nothing. Sorting works because
/// the card's own id is part of the seat: no two seats of one rack are equal,
/// so an order is enough to line the two lists up.
fn same_seats(mut before: Vec<Seat>, mut after: Vec<Seat>) -> bool {
    before.sort();
    after.sort();
    before == after
}
fn seats(cards: &[Card]) -> Vec<Seat> {
    cards.iter().map(seat).collect()
}
fn same_rack(before: &[Card], after: &[Card]) -> bool {
    same_seats(seats(before), seats(after))
}
fn seat(card: &Card) -> Seat {
    (card.id.0, card.stage, card.branch, card.span)
}

impl Command for RackStep {
    fn label(&self) -> String {
        match self {
            Self::Roster { before, after } => roster_label(before, after),
            Self::Trim { name, side, to, .. } => format!(
                "trim {} to {:+.1} dB",
                wire_word(name, *side),
                f64::from(*to)
            ),
            Self::Circuit { name, side, to, .. } => format!(
                "{} {}",
                if *to { "open" } else { "close" },
                wire_word(name, *side)
            ),
            Self::Bypass { name, to, .. } => {
                format!("{} {}", if *to { "bypass" } else { "unbypass" }, name)
            }
            Self::Pause { name, to, .. } => {
                format!("{} {}", if *to { "pause" } else { "resume" }, name)
            }
        }
    }

    fn gesture(&self) -> Option<(u8, u64, u64)> {
        match self {
            // A gain drag is one act along one wire. The tag keeps a trim and
            // a switch on the same wire from folding into each other, which
            // would be one step doing two things.
            Self::Trim { id, side, .. } => Some((0, *id, u64::from(*side))),
            // Everything else is already whole: a click, a drop, a load.
            _ => None,
        }
    }

    fn fold(&mut self, later: &Self) {
        if let (Self::Trim { to, .. }, Self::Trim { to: next, .. }) = (self, later) {
            *to = *next;
        }
    }

    fn is_void(&self) -> bool {
        match self {
            // Two lists that describe the same rack in the same seats are the
            // same rack, however the list happens to be stored: a step like
            // that is not something the operator did, and recording it would
            // put a keystroke on the stack that does nothing.
            Self::Roster { before, after } => same_rack(before, after),
            Self::Trim { from, to, .. } => from == to,
            Self::Circuit { from, to, .. }
            | Self::Bypass { from, to, .. }
            | Self::Pause { from, to, .. } => from == to,
        }
    }
}

/// Name a roster change the way the operator would: by the card that arrived,
/// left, or moved, and only by the rack when more than one card is involved.
fn roster_label(before: &[Card], after: &[Card]) -> String {
    let gone = before
        .iter()
        .filter(|was| !after.iter().any(|now| now.id == was.id))
        .collect::<Vec<_>>();
    let added = after
        .iter()
        .filter(|now| !before.iter().any(|was| was.id == now.id))
        .collect::<Vec<_>>();
    match (added.len(), gone.len()) {
        (1, 0) => format!("add {}", short(&added[0].name, 20)),
        (0, 1) => format!("remove {}", short(&gone[0].name, 20)),
        _ if before.len() == after.len() => {
            // Nobody came or went, so a card moved. The first one whose seat
            // differs is the one to name, and the rest is the rack's business.
            // Where every seat is still where it was, the list itself moved,
            // which is still worth the word "move": a step that left every
            // seat alone never reaches a label at all, because `is_void`
            // throws it away before it is recorded.
            let moved = before
                .iter()
                .zip(after.iter())
                .find(|(was, now)| seat(was) != seat(now))
                .map(|(was, _)| was)
                .or_else(|| {
                    before.iter().find(|was| {
                        after.iter().position(|now| now.id == was.id)
                            != before.iter().position(|now| now.id == was.id)
                    })
                })
                .map(|card| short(&card.name, 20))
                .unwrap_or_else(|| "the rack".into());
            format!("move {moved}")
        }
        (added, 0) => format!("add {added} cards"),
        (0, gone) => format!("remove {gone} cards"),
        _ => "rearrange the rack".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::audio::MASTER_WIRE_KEY;

    /// A command for the stack's own tests: a step from one number to another,
    /// tagged so a run of them on the same key can fold.
    struct Move {
        key: u64,
        from: u32,
        to: u32,
    }

    impl Command for Move {
        fn label(&self) -> String {
            format!("move {} to {}", self.key, self.to)
        }
        fn gesture(&self) -> Option<(u8, u64, u64)> {
            Some((0, self.key, 0))
        }
        fn fold(&mut self, later: &Self) {
            self.to = later.to;
        }
        fn is_void(&self) -> bool {
            self.from == self.to
        }
    }

    /// A seat, spelled the way the rack spells it: which card, which stage,
    /// which chain along it, which columns its lane runs between.
    fn seat_of(id: u64, stage: u64, branch: Option<u64>, span: Option<(u64, u64)>) -> Seat {
        (id, stage, branch, span)
    }

    fn move_(key: u64, from: u32, to: u32) -> Move {
        Move { key, from, to }
    }

    /// The two halves of one press, done the way the rack does them: read the
    /// tip, run it, and tell the stack to move only once the rack has.
    fn take(history: &mut History<Move>, direction: Direction) -> Option<Move> {
        let step = match direction {
            Direction::Undo => history.undo(),
            Direction::Redo => history.redo(),
        }?;
        let step = Move {
            key: step.key,
            from: step.from,
            to: step.to,
        };
        history.commit(direction);
        Some(step)
    }

    #[test]
    fn a_gesture_folds_into_one_step_that_reaches_the_final_value() {
        let mut history = History::new(DEPTH);
        history.record(move_(1, 0, 1));
        history.record(move_(1, 1, 7));
        history.record(move_(1, 7, 12));
        assert_eq!(history.len(), 1);
        let step = take(&mut history, Direction::Undo).expect("one step");
        assert_eq!((step.from, step.to), (0, 12));
        assert!(!history.can_undo(), "the whole drag is one undo");
        assert!(history.can_redo());
    }

    #[test]
    fn undo_walks_back_and_redo_walks_forward_in_order() {
        let mut history = History::new(DEPTH);
        history.record(move_(1, 0, 1));
        history.record(move_(2, 0, 5));
        assert_eq!(take(&mut history, Direction::Undo).expect("two").key, 2);
        assert_eq!(take(&mut history, Direction::Undo).expect("one").key, 1);
        assert!(
            take(&mut history, Direction::Undo).is_none(),
            "the past runs out quietly"
        );
        assert_eq!(take(&mut history, Direction::Redo).expect("one").key, 1);
        assert_eq!(take(&mut history, Direction::Redo).expect("two").key, 2);
        assert!(
            take(&mut history, Direction::Redo).is_none(),
            "and so does the future"
        );
    }

    #[test]
    fn doing_something_new_ends_the_redo() {
        let mut history = History::new(DEPTH);
        history.record(move_(1, 0, 1));
        history.record(move_(2, 0, 5));
        take(&mut history, Direction::Undo);
        history.record(move_(3, 0, 9));
        assert!(!history.can_redo(), "the road not taken is gone");
        assert_eq!(take(&mut history, Direction::Undo).expect("three").key, 3);
    }

    #[test]
    fn two_wires_are_two_gestures() {
        let mut history = History::new(DEPTH);
        history.record(move_(1, 0, 1));
        history.record(move_(2, 0, 1));
        assert_eq!(history.len(), 2, "one per wire, not one long drag");
    }

    #[test]
    fn a_gesture_that_ends_where_it_began_leaves_nothing_behind() {
        let mut history = History::new(DEPTH);
        history.record(move_(9, 0, 1));
        // The operator nudges one value and puts it straight back.
        history.record(move_(1, 4, 9));
        history.record(move_(1, 9, 4));
        assert_eq!(history.len(), 1, "a round trip is not a step");
        let step = take(&mut history, Direction::Undo).expect("the earlier act");
        assert_eq!(step.key, 9, "Ctrl-Z lands on the last thing that changed");
        assert!(!history.can_undo());
    }

    #[test]
    fn a_slip_does_not_eat_the_redo() {
        let mut history = History::new(DEPTH);
        history.record(move_(1, 0, 1));
        history.record(move_(2, 0, 5));
        take(&mut history, Direction::Undo);
        // A drag that goes nowhere is not a new direction for the history.
        history.record(move_(3, 2, 2));
        assert!(history.can_redo(), "the undone step is still there");
        assert_eq!(take(&mut history, Direction::Redo).expect("two").key, 2);
    }

    #[test]
    fn a_refused_step_has_not_happened() {
        let mut history = History::new(DEPTH);
        history.record(move_(1, 0, 1));
        // Read the tip and never commit it: the rack said no, and a step the
        // rack refused must not have happened to the history either.
        assert_eq!(history.undo().expect("one").to, 1);
        assert!(history.can_undo(), "nothing moved, so it is still next");
        assert!(!history.can_redo(), "and nothing has been taken back");
        assert_eq!(
            take(&mut history, Direction::Undo).expect("one again").to,
            1
        );
    }

    #[test]
    fn the_stack_stops_at_its_depth() {
        let mut history = History::new(3);
        for key in 0..10 {
            history.record(Move {
                key,
                from: 0,
                to: 1,
            });
        }
        assert_eq!(history.len(), 3);
        assert_eq!(take(&mut history, Direction::Undo).expect("nine").key, 9);
        assert_eq!(take(&mut history, Direction::Undo).expect("eight").key, 8);
        assert_eq!(take(&mut history, Direction::Undo).expect("seven").key, 7);
        assert!(!history.can_undo(), "the oldest steps have gone");
    }

    #[test]
    fn a_new_stack_has_nothing_to_take_back() {
        // A stack is born with the rack it belongs to, which is the whole of
        // what makes it true: a rack loaded from a file is a new piece of work,
        // and the steps that made the previous one cannot be applied to this
        // one. So there is nothing here to forget, and nothing that can be
        // taken back before the operator has done a single thing.
        let history = History::<Move>::default();
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn a_seat_is_the_rack_and_the_list_is_not() {
        let one = seat_of(1, 0, None, None);
        let two = seat_of(3, 0, None, Some((0, 2)));

        assert!(
            same_seats(vec![one, two], vec![two, one]),
            "the same cards in the same seats are one rack, however the list is ordered"
        );
        assert!(
            !same_seats(vec![one, two], vec![(two.0, 1, two.2, two.3), one]),
            "a card one stage along is a different rack"
        );
        assert!(
            !same_seats(vec![one], vec![one, two]),
            "a card that arrived is a different rack, whatever the order"
        );
    }

    #[test]
    fn a_step_names_what_was_done_in_the_forward_direction() {
        let trim = RackStep::Trim {
            name: "Tube Drive".into(),
            id: 7,
            side: true,
            from: 0.0,
            to: 6.5,
        };
        assert_eq!(trim.label(), "trim Tube Drive output wire to +6.5 dB");
        let bypass = RackStep::Bypass {
            name: "Tube Drive".into(),
            id: 7,
            from: false,
            to: true,
        };
        assert_eq!(bypass.label(), "bypass Tube Drive");
        assert_eq!(
            bypass.label(),
            "bypass Tube Drive",
            "the label names the act, and the direction is said elsewhere"
        );
    }

    #[test]
    fn the_two_runs_that_hang_on_no_card_name_themselves() {
        let out = RackStep::Trim {
            name: "Output".into(),
            id: MASTER_WIRE_KEY,
            side: false,
            from: 0.0,
            to: -3.0,
        };
        assert_eq!(out.label(), "trim output wire to -3.0 dB");
        let input = RackStep::Circuit {
            name: "Input".into(),
            id: 0,
            side: true,
            from: false,
            to: true,
        };
        assert_eq!(input.label(), "open input wire");
    }
}
