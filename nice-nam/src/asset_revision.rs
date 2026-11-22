//! Compatibility notification for persisted, non-parameter NAM preferences.
//! Paths stay in instance state. The hidden numeric parameter only tells hosts
//! to save that state; it has no DSP meaning and must not be automated.
use crate::settings::NamPreferences;
use nice_plug::prelude::{IntParam, IntRange};
use std::sync::{
    Mutex,
    atomic::{AtomicU32, Ordering},
};

pub(crate) const MAX: i32 = 65_535;

pub(crate) fn parameter() -> IntParam {
    IntParam::new("Asset revision", 0, IntRange::Linear { min: 0, max: MAX })
        .hide()
        .non_automatable()
}

/// Stay distinct from both the restored/current parameter and any GUI edits
/// still queued for audio. A bounded integer range stays exact through f32
/// normalization; wrapping has no meaning beyond another change notification.
pub(crate) fn next(sequence: &AtomicU32, current: i32) -> i32 {
    loop {
        let value = (sequence.fetch_add(1, Ordering::Relaxed) & MAX as u32) as i32;
        if value != current {
            return value;
        }
    }
}

pub(crate) fn edit(
    paths: &Mutex<NamPreferences>,
    change: impl FnOnce(&mut NamPreferences),
    notify: impl FnOnce(),
) -> bool {
    let changed = {
        let mut paths = paths.lock().unwrap();
        let previous = paths.clone();
        change(&mut paths);
        *paths != previous
    };
    // Host calls can synchronously save state. Never enter them with the
    // persisted-fields mutex held, or that save could wait on this editor.
    if changed {
        notify();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use nice_plug::prelude::{Param, ParamFlags};
    use std::cell::Cell;
    #[test]
    fn revision_is_hidden_non_automatable_and_exactly_representable() {
        let param = parameter();
        assert!(param.flags().contains(ParamFlags::HIDDEN));
        assert!(param.flags().contains(ParamFlags::NON_AUTOMATABLE));
        for value in [0, 1, MAX / 2, MAX - 1, MAX] {
            assert_eq!(param.preview_plain(param.preview_normalized(value)), value);
        }
    }
    #[test]
    fn restored_values_queued_edits_and_wrapping_still_produce_changes() {
        let sequence = AtomicU32::new(0);
        assert_eq!(next(&sequence, 0), 1);
        assert_eq!(next(&sequence, 0), 2); // earlier edit not applied yet
        sequence.store(MAX as u32, Ordering::Relaxed);
        assert_eq!(next(&sequence, MAX), 0);
        sequence.store(u32::MAX, Ordering::Relaxed);
        assert_eq!(next(&sequence, MAX), 0);
    }
    #[test]
    fn all_persisted_edits_notify_after_releasing_the_lock_and_noops_do_not() {
        let paths = Mutex::new(NamPreferences::default());
        let notifications = Cell::new(0);
        let notify = || {
            assert!(paths.try_lock().is_ok());
            notifications.set(notifications.get() + 1);
        };
        assert!(edit(
            &paths,
            |p| {
                p.model_path = Some("model.nam".into());
                p.model_name = Some("model.nam".into());
            },
            notify
        ));
        assert!(edit(
            &paths,
            |p| p.cabinet_path = Some("cab.wav".into()),
            notify
        ));
        assert!(edit(&paths, |p| p.gate_db = Some(-40.0), notify));
        assert!(edit(&paths, |p| p.cabinet_path = None, notify));
        assert!(!edit(&paths, |p| p.cabinet_path = None, notify));
        assert_eq!(notifications.get(), 4);
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use crate::NamParams;
    use nice_plug::prelude::Params;
    #[test]
    fn paths_remain_private_instance_fields_and_legacy_fields_still_restore() {
        let source = NamParams::default();
        let preferences = NamPreferences {
            model_name: Some("amp.nam".into()),
            model_path: Some("models/amp.nam".into()),
            cabinet_path: Some("cabinets/speaker.wav".into()),
            gate_db: Some(-40.0),
        };
        *source.paths.lock().unwrap() = preferences.clone();
        // The persisted "assets" record is unchanged: older instances had
        // this same field and no asset_revision parameter.
        let fields = source.serialize_fields();
        assert_eq!(
            fields.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["assets"]
        );
        let restored = NamParams::default();
        restored.deserialize_fields(&fields);
        assert_eq!(*restored.paths.lock().unwrap(), preferences);
        assert_eq!(restored.asset_revision.value(), 0);
        assert_eq!(
            source
                .param_map()
                .iter()
                .map(|(id, _, _)| id.as_str())
                .collect::<Vec<_>>(),
            vec!["asset_revision"]
        );
    }
}
