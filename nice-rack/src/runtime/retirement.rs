//! Main-thread ownership outlives an unexpectedly dropped rack while its DSP
//! call finishes. No plugin instance/library moves to a cleanup thread.
use super::*;
use std::cell::RefCell;

pub(super) struct Pending {
    pub session: Option<LiveSession>,
    pub retired: Option<LiveSession>,
    pub worker: Option<JoinHandle<Endpoints>>,
    pub stopping: Option<JoinHandle<()>>,
    pub io: Option<DeviceStreams>,
    pub idle: Option<Endpoints>,
}
impl Pending {
    pub(super) fn finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
            && self.stopping.as_ref().is_none_or(JoinHandle::is_finished)
    }
    fn service(&mut self) {
        if let Some(session) = &mut self.session {
            let _ = session.service();
        }
    }
    fn finish(mut self) {
        debug_assert!(self.finished());
        if let Some(worker) = self.worker.take() {
            self.idle = worker.join().ok();
        }
        if let Some(worker) = self.stopping.take() {
            let _ = worker.join();
        }
        self.service();
        // Native editors/instances must be destroyed on this owning thread,
        // after the audio worker has returned its stopped processing handles.
        drop(self.session.take());
        drop(self.retired.take());
        if let Some(io) = self.io.take() {
            let idle = self.idle.take();
            std::thread::spawn(move || {
                drop(idle);
                drop(io);
            });
        }
    }
}
#[derive(Default)]
struct Queue(Vec<Pending>);
impl Drop for Queue {
    fn drop(&mut self) {
        // At thread exit there is no legal main-thread executor left. Unmapping
        // an instance/library still in use would be unsound. Retain these rare
        // forced-shutdown resources for process lifetime rather than join a
        // hung foreign call or destroy it on the wrong thread.
        for pending in self.0.drain(..) {
            std::mem::forget(pending);
        }
    }
}
thread_local! { static QUEUE: RefCell<Queue> = RefCell::new(Queue::default()); }

pub(super) fn defer(pending: Pending) {
    QUEUE.with(|queue| queue.borrow_mut().0.push(pending));
}
pub(super) fn poll() {
    // Take the queue out before calling plugins: callbacks may reenter host code.
    let pending = QUEUE.with(|queue| std::mem::take(&mut queue.borrow_mut().0));
    let mut waiting = Vec::new();
    for mut pending in pending {
        if pending.finished() {
            pending.finish();
        } else {
            pending.service();
            waiting.push(pending);
        }
    }
    QUEUE.with(|queue| queue.borrow_mut().0.extend(waiting));
}

#[cfg(test)]
pub(super) fn empty() -> bool {
    QUEUE.with(|q| q.borrow().0.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_pending_worker_is_polled_and_joined_only_after_completion() {
        let (release, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            wait.recv().unwrap();
        });
        let (capture, input) = rtrb::RingBuffer::new(2);
        let (output, _playback) = rtrb::RingBuffer::new(2);
        let pending = Pending {
            session: None,
            retired: None,
            worker: None,
            stopping: Some(worker),
            io: None,
            idle: Some(Endpoints { input, output }),
        };
        assert!(!pending.finished());
        defer(pending);
        poll();
        assert_eq!(QUEUE.with(|q| q.borrow().0.len()), 1);
        assert!(
            !capture.is_abandoned(),
            "resources dropped before worker completion"
        );
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while QUEUE.with(|q| !q.borrow().0.is_empty()) {
            poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(capture.is_abandoned());
    }
}
