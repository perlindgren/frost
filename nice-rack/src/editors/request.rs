//! One-shot UI requests. Poll once per tick; never wait for the worker.
use crossbeam::channel::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

pub(super) struct Request<T> {
    result: Receiver<Result<T, String>>,
    deadline: Instant,
    timeout: &'static str,
    disconnected: &'static str,
}
impl<T> Request<T> {
    pub fn new(
        result: Receiver<Result<T, String>>,
        limit: Duration,
        timeout: &'static str,
        disconnected: &'static str,
    ) -> Self {
        Self {
            result,
            deadline: Instant::now() + limit,
            timeout,
            disconnected,
        }
    }
    pub fn poll(&self) -> Result<Option<T>, String> {
        // Prefer a completed answer if graph rebuilding delayed UI polling.
        match self.result.try_recv() {
            Ok(result) => result.map(Some),
            Err(TryRecvError::Disconnected) => Err(self.disconnected.into()),
            Err(TryRecvError::Empty) if Instant::now() >= self.deadline => Err(self.timeout.into()),
            Err(TryRecvError::Empty) => Ok(None),
        }
    }
}

impl<T> Drop for Request<T> {
    fn drop(&mut self) {
        // Crossbeam can retain a buffered value while a sender still exists.
        // Dispose of an already-delivered resource on cancellation immediately.
        if let Ok(result) = self.result.try_recv() {
            drop(result);
        }
    }
}

/// Cancellation closes a not-yet-attached native frame, including when its
/// successful response was buffered but the UI never consumed it.
pub(super) struct Cancellable<T> {
    request: Request<T>,
    cancel: Option<Box<dyn FnOnce()>>,
}
impl<T> Cancellable<T> {
    pub fn new(request: Request<T>, cancel: impl FnOnce() + 'static) -> Self {
        Self {
            request,
            cancel: Some(Box::new(cancel)),
        }
    }
    pub fn poll(&mut self) -> Result<Option<T>, String> {
        let value = self.request.poll()?;
        if value.is_some() {
            self.cancel = None;
        }
        Ok(value)
    }
}
impl<T> Drop for Cancellable<T> {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request<T>(result: Receiver<Result<T, String>>) -> Request<T> {
        Request::new(result, Duration::from_secs(10), "timeout", "disconnected")
    }
    #[test]
    fn worker_failure_and_timeout_keep_the_cancellation_armed() {
        use std::{cell::Cell, rc::Rc};
        for timeout in [false, true] {
            let count = Rc::new(Cell::new(0));
            let cancel = count.clone();
            let (sender, receiver) = crossbeam::channel::bounded::<Result<(), String>>(1);
            let mut pending = request(receiver);
            if timeout {
                pending.deadline = Instant::now() - Duration::from_secs(1);
            } else {
                sender.send(Err("failed".into())).unwrap();
            }
            let mut guarded = Cancellable::new(pending, move || cancel.set(cancel.get() + 1));
            assert!(guarded.poll().is_err());
            drop(guarded);
            assert_eq!(count.get(), 1);
        }
    }
    #[test]
    fn cancellation_closes_only_frames_not_handed_to_the_ui() {
        use std::{cell::Cell, rc::Rc};
        for delivered in [false, true] {
            let cancellations = Rc::new(Cell::new(0));
            let counter = cancellations.clone();
            let (sender, result) = crossbeam::channel::bounded(1);
            let mut pending =
                Cancellable::new(request(result), move || counter.set(counter.get() + 1));
            sender.send(Ok(42)).unwrap();
            if delivered {
                assert_eq!(pending.poll(), Ok(Some(42)));
            }
            drop(pending);
            assert_eq!(cancellations.get(), usize::from(!delivered));
        }
    }
    #[test]
    fn cancellation_is_queued_after_creation_and_before_a_new_request() {
        let (commands, receiver) = crossbeam::channel::unbounded();
        let (_reply, result) = crossbeam::channel::bounded::<Result<(), String>>(1);
        commands.send("open").unwrap();
        let cancel = commands.clone();
        let pending = Cancellable::new(request(result), move || {
            cancel.send("close").unwrap();
        });
        drop(pending);
        commands.send("reopen").unwrap();
        assert_eq!(
            receiver.try_iter().collect::<Vec<_>>(),
            ["open", "close", "reopen"]
        );
    }
    #[test]
    fn pending_success_and_worker_errors_are_distinct() {
        let (sender, result) = crossbeam::channel::bounded(1);
        let pending = request(result);
        assert_eq!(pending.poll(), Ok(None));
        sender.send(Ok(42)).unwrap();
        assert_eq!(pending.poll(), Ok(Some(42)));
        sender.send(Err("worker failure".into())).unwrap();
        assert_eq!(pending.poll(), Err("worker failure".into()));
    }
    #[test]
    fn timeout_and_disconnection_never_wait_for_a_worker() {
        let (sender, result) = crossbeam::channel::bounded::<Result<(), String>>(1);
        let mut pending = request(result);
        pending.deadline = Instant::now() - Duration::from_secs(1);
        assert_eq!(pending.poll(), Err("timeout".into()));
        // A completed reply still belongs to the caller after a delayed tick.
        sender.send(Ok(())).unwrap();
        assert_eq!(pending.poll(), Ok(Some(())));
        drop(sender);
        assert_eq!(pending.poll(), Err("disconnected".into()));
    }
    #[test]
    fn cancellation_drops_buffered_resources_and_rejects_late_results() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Resource(Arc<AtomicUsize>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let (sender, result) = crossbeam::channel::bounded(1);
        let pending = request(result);
        sender.send(Ok(Resource(drops.clone()))).ok().unwrap();
        drop(pending);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        drop(sender.send(Ok(Resource(drops.clone()))));
        assert_eq!(drops.load(Ordering::Relaxed), 2);
    }
}
