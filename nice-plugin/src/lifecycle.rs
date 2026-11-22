//! The standalone follows the hosted plugin's activate -> reset -> process order.
use nice_plug::prelude::*;

pub fn activate<P: Plugin>(
    plugin: &mut P,
    rate: f32,
    max_frames: usize,
    context: &mut impl ActivateContext<P>,
    error: &str,
) -> Result<(), String> {
    plugin.deactivate();
    if !plugin.activate(
        &P::AUDIO_IO_LAYOUTS[0],
        &BufferConfig {
            sample_rate: rate,
            min_buffer_size: None,
            max_buffer_size: max_frames as u32,
            process_mode: ProcessMode::Realtime,
        },
        context,
    ) {
        return Err(error.into());
    }
    plugin.reset();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[derive(Default, Params)]
    struct Parameters {}
    struct Ui;
    impl frost::Process for Ui {
        fn process(&mut self, _: &mut frost::Context, _: f32) {}
    }
    #[derive(Default)]
    struct Instance {
        events: Vec<&'static str>,
        refuse: bool,
        config: Option<(f32, u32)>,
    }
    impl Plugin for Instance {
        const NAME: &'static str = "Test";
        const VENDOR: &'static str = "frost";
        const URL: &'static str = "";
        const EMAIL: &'static str = "";
        const VERSION: &'static str = "0.1.0";
        const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout::const_default()];
        type Editor = crate::Editor<Ui>;
        type SysExMessage = ();
        type BackgroundTask = ();
        fn params(&self) -> Arc<dyn Params> {
            Arc::new(Parameters::default())
        }
        fn activate(
            &mut self,
            _: &AudioIOLayout,
            config: &BufferConfig,
            _: &mut impl ActivateContext<Self>,
        ) -> bool {
            self.events.push("activate");
            self.config = Some((config.sample_rate, config.max_buffer_size));
            !self.refuse
        }
        fn deactivate(&mut self) {
            self.events.push("deactivate");
        }
        fn reset(&mut self) {
            self.events.push("reset");
        }
        fn process(
            &mut self,
            _: &mut Buffer,
            _: &mut AuxiliaryBuffers,
            _: &mut impl ProcessContext<Self>,
        ) -> ProcessStatus {
            ProcessStatus::Normal
        }
    }
    struct Context;
    impl ActivateContext<Instance> for Context {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Standalone
        }
        fn execute(&self, _: ()) {}
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }
    #[test]
    fn every_successful_activation_resets_before_processing_can_start() {
        let mut instance = Instance::default();
        for rate in [48000.0, 96000.0] {
            activate(&mut instance, rate, 64, &mut Context, "failed").unwrap();
            assert_eq!(instance.config, Some((rate, 64)));
        }
        assert_eq!(
            instance.events,
            vec![
                "deactivate",
                "activate",
                "reset",
                "deactivate",
                "activate",
                "reset"
            ]
        );
    }
    #[test]
    fn rejected_activation_does_not_reset_or_hide_the_error() {
        let mut instance = Instance {
            refuse: true,
            ..Default::default()
        };
        assert_eq!(
            activate(&mut instance, 48000.0, 32, &mut Context, "device refused"),
            Err("device refused".into())
        );
        assert_eq!(instance.events, vec!["deactivate", "activate"]);
    }
}
