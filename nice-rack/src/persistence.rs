//! Save scheduling and the last successfully written rack document.
//! Native plugin state capture belongs to LiveSession, not to this controller.
use crate::{RackPreferences, Settings};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(crate) struct Persistence {
    pub(crate) last_attempt: Instant,
    saved: RackPreferences,
}
impl Persistence {
    pub(crate) fn new(saved: RackPreferences) -> Self {
        Self {
            last_attempt: Instant::now(),
            saved,
        }
    }
    pub(crate) fn due(&self) -> bool {
        self.last_attempt.elapsed() > Duration::from_secs(2)
    }
    /// Record attempts before foreign state callbacks, including failed ones.
    /// Otherwise a failure retries on every UI tick once the deadline expires.
    pub(crate) fn attempted(&mut self) {
        self.last_attempt = Instant::now();
    }
    pub(crate) fn save(
        &mut self,
        path: &Path,
        preferences: &RackPreferences,
    ) -> Result<bool, String> {
        if *preferences == self.saved {
            return Ok(false);
        }
        // Preserve audio settings owned by the settings panel.
        let (mut settings, warning) = Settings::load_from(path);
        if let Some(warning) = warning {
            return Err(warning);
        }
        settings.preferences = preferences.clone();
        settings.save_to(path)?;
        // Failed writes never acknowledge the new document.
        self.saved = preferences.clone();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_attempt_advances_the_deadline_even_when_no_save_succeeds() {
        let mut saves = Persistence::new(RackPreferences::default());
        saves.last_attempt = Instant::now() - Duration::from_secs(3);
        assert!(saves.due());
        saves.attempted();
        assert!(!saves.due());
    }
    #[test]
    fn failed_writes_are_retried_and_device_settings_are_preserved() {
        let dir = frost::scratch::claim("rack-persistence-controller");
        let path = dir.join("settings.ron");
        let settings = Settings {
            input_device: Some("Keep this device".into()),
            ..Settings::default()
        };
        settings.save_to(&path).unwrap();
        let mut saves = Persistence::new(settings.preferences.clone());
        let mut next = settings.preferences.clone();
        next.input_gain_db = -8.0;
        std::fs::write(&path, "invalid RON").unwrap();
        assert!(saves.save(&path, &next).is_err());
        settings.save_to(&path).unwrap();
        assert!(saves.save(&path, &next).unwrap());
        let (loaded, warning) = Settings::load_from(&path);
        assert!(warning.is_none());
        assert_eq!(loaded.input_device, settings.input_device);
        assert_eq!(loaded.preferences.input_gain_db, -8.0);
        assert!(!saves.save(&path, &next).unwrap());
    }
}
