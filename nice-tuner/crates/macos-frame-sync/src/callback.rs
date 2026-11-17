//! Per-editor callback state, independent of native object ownership.
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};

pub(super) struct Callback {
    call: RefCell<Box<dyn FnMut() -> bool>>,
    pub(super) stopped: Cell<bool>,
    pub(super) panicked: Cell<bool>,
}
impl Callback {
    pub(super) fn new(call: impl FnMut() -> bool + 'static) -> Self {
        Self {
            call: RefCell::new(Box::new(call)),
            stopped: Cell::new(false),
            panicked: Cell::new(false),
        }
    }
    pub(super) fn invoke(&self) -> bool {
        if self.stopped.get() {
            return false;
        }
        // A nested run-loop dispatch must not recursively render the same UI.
        let Ok(mut call) = self.call.try_borrow_mut() else {
            return true;
        };
        let keep_running = match catch_unwind(AssertUnwindSafe(&mut *call)) {
            Ok(keep_running) => keep_running,
            Err(_) => {
                self.panicked.set(true);
                false
            }
        };
        // The callback may have dropped its driver while it was running.
        let keep_running = keep_running && !self.stopped.get();
        self.stopped.set(!keep_running);
        keep_running
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    #[test]
    fn stopping_and_panics_prevent_future_callbacks() {
        let calls = Rc::new(Cell::new(0));
        let counter = calls.clone();
        let callback = Callback::new(move || {
            counter.set(counter.get() + 1);
            false
        });
        assert!(!callback.invoke());
        assert!(!callback.invoke());
        assert_eq!(calls.get(), 1);
        let panicking = Callback::new(|| panic!("test callback"));
        assert!(!panicking.invoke());
        assert!(panicking.panicked.get());
        assert!(!panicking.invoke());
    }
    #[test]
    fn reentrant_dispatch_is_skipped_without_stopping_the_driver() {
        let callback = Callback::new(|| true);
        let borrow = callback.call.borrow_mut();
        assert!(callback.invoke());
        assert!(!callback.stopped.get());
        drop(borrow);
        assert!(callback.invoke());
    }

    #[test]
    fn cancellation_from_inside_a_callback_cannot_restart_it() {
        let callback: Rc<Callback> = Rc::new_cyclic(|weak: &std::rc::Weak<Callback>| {
            let weak = weak.clone();
            Callback::new(move || {
                weak.upgrade().unwrap().stopped.set(true);
                true
            })
        });
        assert!(!callback.invoke());
        assert!(!callback.invoke());
    }
}
