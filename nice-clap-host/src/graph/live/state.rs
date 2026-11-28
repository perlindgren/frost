//! State capture stays on the plugin's main thread, independently of disk I/O.
use super::*;

// A failed batch returns no bytes, so all captures in that batch must stay
// pending. Acknowledgement never clears a newer notification.
struct PendingCapture {
    signals: Callbacks,
    committed: bool,
}
impl PendingCapture {
    fn begin(signals: &Callbacks, changed_only: bool) -> Option<Self> {
        let changed = signals.0.changed.swap(false, Ordering::AcqRel);
        if changed_only && !changed {
            return None;
        }
        Some(Self {
            signals: signals.clone(),
            committed: false,
        })
    }
    fn commit(mut self) {
        self.committed = true;
    }
}
impl Drop for PendingCapture {
    fn drop(&mut self) {
        if !self.committed {
            self.signals.0.changed.store(true, Ordering::Release);
        }
    }
}

impl LiveSession {
    /// Instances reporting non-parameter or parameter-value changes. Reading
    /// this does not acknowledge them: only successful capture does that.
    pub fn dirty_instances(&self) -> Vec<NodeId> {
        self.remote_owners
            .iter()
            .filter_map(|(&id, owner)| owner.dirty.then_some(id))
            .chain(self.owners.iter().filter_map(|(&id, owner)| {
                owner
                    .signals
                    .0
                    .changed
                    .load(Ordering::Acquire)
                    .then_some(id)
            }))
            .collect()
    }

    /// Capture local instances and collect confirmed isolated state.
    /// For a fresh full isolated save, first poll `isolated_snapshot_ready` until
    /// ready. This method never waits for remote callbacks. CLAP permits local
    /// state saving while active; this must run on the main thread.
    pub fn save_states(&mut self) -> Result<BTreeMap<NodeId, Vec<u8>>, String> {
        self.capture_states(false)
    }

    /// Capture only instances which reported a change. Callers must capture a
    /// new instance once and retain the resulting bytes between saves.
    pub fn save_changed_states(&mut self) -> Result<BTreeMap<NodeId, Vec<u8>>, String> {
        self.capture_states(true)
    }

    /// Recovery requires a stopped graph even though serialization itself does
    /// not. For isolated instances, await `isolated_snapshot_ready` across UI
    /// frames first; after a failure these bytes are only the last confirmed
    /// cache. Keep that lifecycle requirement explicit at the recovery entry point.
    pub fn snapshot(&mut self) -> Result<BTreeMap<NodeId, Vec<u8>>, String> {
        if !self.stopped {
            return Err("Stop the graph before saving state".into());
        }
        self.capture_states(false)
    }

    fn capture_states(&mut self, changed_only: bool) -> Result<BTreeMap<NodeId, Vec<u8>>, String> {
        let mut states = BTreeMap::new();
        let mut pending = Vec::new();
        let mut remote_pending = Vec::new();
        for (&id, owner) in &mut self.remote_owners {
            owner.poll();
            if !changed_only || owner.dirty {
                if let Some(bytes) = &owner.state {
                    states.insert(id, bytes.clone());
                }
                remote_pending.push(id);
            }
        }
        for (&id, owner) in &mut self.owners {
            let Some(capture) = PendingCapture::begin(&owner.signals, changed_only) else {
                continue;
            };
            pending.push(capture);
            if let Some(bytes) = save_state(&mut owner.instance)
                .map_err(|error| format!("Plugin {id:?}: {error}"))?
            {
                states.insert(id, bytes);
            }
        }
        for id in remote_pending {
            self.remote_owners.get_mut(&id).unwrap().dirty = false;
        }
        for capture in pending {
            capture.commit();
        }
        Ok(states)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_instances_do_not_need_capture() {
        let signals = Callbacks::default();
        assert!(PendingCapture::begin(&signals, true).is_none());
        signals.mark_dirty();
        PendingCapture::begin(&signals, true).unwrap().commit();
        assert!(PendingCapture::begin(&signals, true).is_none());
    }
    #[test]
    fn a_failed_batch_keeps_earlier_captures_pending_too() {
        let first = Callbacks::default();
        let second = Callbacks::default();
        first.mark_dirty();
        second.mark_dirty();
        let captures = vec![
            PendingCapture::begin(&first, true).unwrap(),
            PendingCapture::begin(&second, true).unwrap(),
        ];
        drop(captures); // same unwind path as returning a native save error
        assert!(first.0.changed.load(Ordering::Acquire));
        assert!(second.0.changed.load(Ordering::Acquire));
    }
    #[test]
    fn a_notification_during_capture_is_not_acknowledged_by_that_capture() {
        let signals = Callbacks::default();
        signals.mark_dirty();
        let capture = PendingCapture::begin(&signals, true).unwrap();
        signals.mark_dirty();
        capture.commit();
        assert!(signals.0.changed.load(Ordering::Acquire));
    }
}
