//! Commands from the editor to nice-tuner's audio owner. No GUI-thread waits.
use crate::settings::Settings;
use crossbeam::channel::{Receiver, Sender, bounded};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Command {
    pub settings: Settings,
    pub reply: Sender<Result<(), String>>,
}
struct Endpoint {
    sender: Sender<Command>,
    pending: Arc<AtomicBool>,
}
static ENDPOINT: Mutex<Option<Endpoint>> = Mutex::new(None);
static ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn available() -> bool {
    ENDPOINT.lock().unwrap().is_some()
}
pub fn take_error() -> Option<String> {
    ERROR.lock().unwrap().take()
}
pub(super) fn report_error(message: String) {
    *ERROR.lock().unwrap() = Some(message);
}

pub fn request(settings: Settings) -> Result<Receiver<Result<(), String>>, String> {
    let endpoint = ENDPOINT.lock().unwrap();
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

pub(super) struct Registration;
pub(super) fn register(sender: Sender<Command>, pending: Arc<AtomicBool>) -> Registration {
    ERROR.lock().unwrap().take();
    *ENDPOINT.lock().unwrap() = Some(Endpoint { sender, pending });
    Registration
}
impl Drop for Registration {
    fn drop(&mut self) {
        *ENDPOINT.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn apply_is_asynchronous_and_does_not_accept_overlapping_commands() {
        let (sender, commands) = bounded(1);
        let pending = Arc::new(AtomicBool::new(false));
        let registration = register(sender, pending.clone());
        let result = request(Settings::default()).unwrap();
        assert!(result.try_recv().is_err());
        assert!(
            request(Settings::default())
                .unwrap_err()
                .contains("already pending")
        );
        commands
            .recv()
            .unwrap()
            .reply
            .send(Err("system device error".into()))
            .unwrap();
        assert_eq!(result.recv().unwrap(), Err("system device error".into()));
        pending.store(false, Ordering::Release);
        let result = request(Settings::default()).unwrap();
        commands.recv().unwrap().reply.send(Ok(())).unwrap();
        assert_eq!(result.recv().unwrap(), Ok(()));
        drop(registration);
        assert!(!available());
    }
}
