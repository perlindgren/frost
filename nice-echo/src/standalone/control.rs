//! The Nice Echo host's endpoint of the shared control logic.
//!
//! The logic — one command in flight, a refusal to overlap, an error slot the
//! panel reads on its next frame — lives once in [`nice_common::control`]. What
//! is per-plugin is the settings type a command carries, so what is left here
//! is a static of that type and the free functions the panel and the audio
//! owner call. nice-nam, nice-tuner and nice-wah keep their own statics beside
//! this one.
use crate::settings::Settings;
use crossbeam::channel::{Receiver, Sender};
use nice_common::control::Control;
pub use nice_common::control::{Command, Registration};
use std::sync::{Arc, atomic::AtomicBool};

static CONTROL: Control<Settings> = Control::new();

pub fn available() -> bool {
    CONTROL.available()
}

pub fn take_error() -> Option<String> {
    CONTROL.take_error()
}

pub(super) fn report_error(message: String) {
    CONTROL.report_error(message);
}

pub fn request(settings: Settings) -> Result<Receiver<Result<(), String>>, String> {
    CONTROL.request(settings)
}

pub(super) fn register(
    sender: Sender<Command<Settings>>,
    pending: Arc<AtomicBool>,
) -> Registration<'static, Settings> {
    CONTROL.register(sender, pending)
}
