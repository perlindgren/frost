//! attune — a guest that listens.
//!
//! No window, no drawing: the debugger's view of the telling
//! folder. Which applications are running, what each told its
//! being to be, how it changes, and when its card dies. This is
//! the proof, in software, that the telling is readable: the same
//! `frost::Teller` machinery `sprite_util` attaches with, wearing
//! a console. Tools read beings; they write documents only — this
//! one writes nothing at all.

use std::collections::HashMap;

/// The well-known folder, the same path every app tells into.
const TELL_DIR: &str = "target/frost/live";

fn main() {
    // What each living pid last told: its rev and its planted
    // beds — the facts a listener speaks aloud. Rev 0 is the
    // sentinel of an app seen running but not yet telling.
    let mut known: HashMap<u32, (u64, usize)> = HashMap::new();
    println!(
        "[attune] listening to {TELL_DIR} — start an application (say `cargo run --example immortal`) and it will speak here."
    );
    loop {
        let cards = frost::Teller::announce(TELL_DIR);
        let living: Vec<u32> = cards.iter().map(|card| card.pid).collect();
        for card in &cards {
            let said = match frost::Teller::tiding(card) {
                Some(said) => said,
                None => {
                    // Alive, not yet telling — worth one line, or
                    // the listener looks deaf exactly when an app
                    // is born.
                    if let std::collections::hash_map::Entry::Vacant(free) = known.entry(card.pid) {
                        println!(
                            "[attune] {} (pid {}) is running — nothing told yet",
                            card.name, card.pid
                        );
                        free.insert((0, usize::MAX));
                    }
                    continue;
                }
            };
            let beds = planted_of(&said.being);
            let names = spoken_count(&said.being, "ships");
            match known.get(&card.pid) {
                // New, or seen running but never heard telling:
                // its first telling is an arrival, not a change.
                Some((0, _)) | None => {
                    println!(
                        "[attune] attached: {} (pid {}) rev {} — {names} names, {beds} beds planted",
                        card.name, card.pid, said.rev
                    );
                }
                Some((rev, was)) if *rev != said.rev || *was != beds => {
                    println!(
                        "[attune] {} (pid {}) tells rev {} — {names} names, {beds} beds (was rev {rev}, {was} beds)",
                        card.name, card.pid, said.rev
                    );
                }
                Some(_) => {}
            }
            known.insert(card.pid, (said.rev, beds));
        }
        let gone: Vec<u32> = known
            .keys()
            .copied()
            .filter(|pid| !living.contains(pid))
            .collect();
        for pid in gone {
            println!("[attune] pid {pid} is gone — its card died with it");
            known.remove(&pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

/// How many beds the being says are planted: every entry of the
/// `beds` record that wears a `true`.
fn planted_of(being: &frost::ron::Val) -> usize {
    match frost::ron::field(being, "beds") {
        Some(frost::ron::Val::Struct { fields, .. }) => fields
            .iter()
            .filter(|(_, item)| item.val.inline().contains("true"))
            .count(),
        _ => 0,
    }
}

/// How many entries a record of spoken names holds.
fn spoken_count(being: &frost::ron::Val, what: &str) -> usize {
    match frost::ron::field(being, what) {
        Some(frost::ron::Val::Struct { fields, .. }) => fields.len(),
        _ => 0,
    }
}
