//! The wah host's endpoint of the shared control logic.
//!
//! The logic — one command in flight, a refusal to overlap, an error slot the
//! panel reads on its next frame — lives once in [`nice_common::control`].
//! What is per-plugin is the settings type a command carries, so what is
//! left here is a static of that type and the free functions the panel and
//! the audio owner call. nice-nam and nice-tuner keep their own statics
//! beside this one.
use crate::settings::Settings;
use crossbeam::channel::Receiver;
use nice_common::control::Control;

pub(super) static CONTROL: Control<Settings> = Control::new();

pub fn available() -> bool {
    CONTROL.available()
}

pub fn take_error() -> Option<String> {
    CONTROL.take_error()
}

pub fn request(settings: Settings) -> Result<Receiver<Result<(), String>>, String> {
    CONTROL.request(settings)
}
