//! Commands from an editor to the audio owner of a standalone application, and
//! the failure that owner reports back. Nothing here waits on the GUI thread:
//! [`Control::request`] hands the configuration over a bounded channel and
//! returns the reply endpoint, so the panel stays responsive while one owner
//! destroys the running streams and opens replacements.
//!
//! A command carries the application's own settings, and those differ per
//! plugin, so the type is generic and each application keeps one static of it.
//! What is shared is the part worth sharing — one command in flight, a refusal
//! to overlap, and an error slot read on the next frame — and it is written once
//! here, over the unit type in the test. Each plugin's `standalone::control` is
//! then a static and the handful of functions its panel calls.

use crossbeam::channel::{Receiver, Sender, bounded};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// One configuration, with the channel its owner answers on.
pub struct Command<S> {
    pub settings: S,
    pub reply: Sender<Result<(), String>>,
}

struct Endpoint<S> {
    sender: Sender<Command<S>>,
    pending: Arc<AtomicBool>,
}

/// The editor's side of one audio owner.
pub struct Control<S> {
    endpoint: Mutex<Option<Endpoint<S>>>,
    error: Mutex<Option<String>>,
}

impl<S> Control<S> {
    /// An endpoint with no owner registered yet. Const so a plugin can hold one
    /// in a `static`.
    pub const fn new() -> Self {
        Self {
            endpoint: Mutex::new(None),
            error: Mutex::new(None),
        }
    }

    /// Whether an audio owner is listening. A panel that shows APPLY without
    /// this would let the user submit a change nobody is there to apply.
    pub fn available(&self) -> bool {
        self.endpoint.lock().unwrap().is_some()
    }

    /// Take the last failure the owner reported, leaving none behind.
    pub fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap().take()
    }

    /// Record a failure for the panel to show on its next frame. Called by the
    /// owner, which must not reach into the GUI to say anything.
    pub fn report_error(&self, message: String) {
        *self.error.lock().unwrap() = Some(message);
    }

    /// Hand a configuration to the owner without waiting for it to be applied.
    ///
    /// A second request while one is outstanding is refused rather than queued:
    /// opening a device is not idempotent, and two half-applied configurations
    /// are worse than a message telling the user to wait.
    pub fn request(&self, settings: S) -> Result<Receiver<Result<(), String>>, String> {
        let endpoint = self.endpoint.lock().unwrap();
        let endpoint = endpoint
            .as_ref()
            .ok_or("Live audio configuration is unavailable")?;
        if endpoint.pending.swap(true, Ordering::AcqRel) {
            return Err("Audio configuration is already pending".into());
        }
        let (reply, receiver) = bounded(1);
        if endpoint
            .sender
            .try_send(Command { settings, reply })
            .is_err()
        {
            endpoint.pending.store(false, Ordering::Release);
            return Err("The audio worker has stopped".into());
        }
        Ok(receiver)
    }

    /// Register the owner. The guard clears the endpoint when it is dropped,
    /// which is how closing the window retires the panel's APPLY.
    pub fn register(
        &self,
        sender: Sender<Command<S>>,
        pending: Arc<AtomicBool>,
    ) -> Registration<'_, S> {
        self.error.lock().unwrap().take();
        *self.endpoint.lock().unwrap() = Some(Endpoint { sender, pending });
        Registration(self)
    }
}

impl<S> Default for Control<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// Clears the endpoint when the owner stops.
pub struct Registration<'a, S>(&'a Control<S>);

impl<S> Drop for Registration<'_, S> {
    fn drop(&mut self) {
        *self.0.endpoint.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_is_asynchronous_and_does_not_accept_overlapping_commands() {
        let control = Control::new();
        let (sender, commands) = bounded(1);
        let pending = Arc::new(AtomicBool::new(false));
        let registration = control.register(sender, pending.clone());
        let result = control.request(()).unwrap();
        assert!(result.try_recv().is_err());
        assert!(control.request(()).unwrap_err().contains("already pending"));
        commands
            .recv()
            .unwrap()
            .reply
            .send(Err("system device error".into()))
            .unwrap();
        assert_eq!(result.recv().unwrap(), Err("system device error".into()));
        pending.store(false, Ordering::Release);
        let result = control.request(()).unwrap();
        commands.recv().unwrap().reply.send(Ok(())).unwrap();
        assert_eq!(result.recv().unwrap(), Ok(()));
        drop(registration);
        assert!(!control.available());
    }

    #[test]
    fn a_request_before_any_owner_is_registered_names_the_reason() {
        let control = Control::new();
        assert_eq!(
            control.request(()).unwrap_err(),
            "Live audio configuration is unavailable"
        );
        // A failure the owner reported is shown once, then gone.
        control.report_error("no such device".into());
        assert_eq!(control.take_error().as_deref(), Some("no such device"));
        assert!(control.take_error().is_none());
    }
}
