//! Main-thread adapter for automatable settings owned by the plugin host.
use crate::TunerParams;
use nice_plug::context::gui::GuiContext;
use std::{rc::Rc, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HostParameter {
    A4,
    Gate,
    InTune,
    GaugeCurve,
}

#[derive(Clone)]
pub(crate) struct HostControls {
    pub params: Arc<TunerParams>,
    pub api: String,
    setter: Rc<dyn Fn(HostParameter, f32)>,
}

impl HostControls {
    pub fn new(params: Arc<TunerParams>, context: GuiContext) -> Self {
        let api = context.plugin_api().to_string();
        let owned = Arc::clone(&params);
        Self {
            params,
            api,
            setter: Rc::new(move |parameter, value| {
                let setter = context.param_setter();
                let parameter = match parameter {
                    HostParameter::A4 => &owned.a4_ref,
                    HostParameter::Gate => &owned.gate_db,
                    HostParameter::InTune => &owned.in_tune_cents,
                    HostParameter::GaugeCurve => &owned.gauge_knee_cents,
                };
                setter.begin_set_parameter(parameter);
                setter.set_parameter(parameter, value);
                setter.end_set_parameter(parameter);
            }),
        }
    }

    pub fn set(&self, parameter: HostParameter, value: f32) {
        (self.setter)(parameter, value);
    }

    #[cfg(test)]
    pub fn for_test(
        params: Arc<TunerParams>,
        setter: impl Fn(HostParameter, f32) + 'static,
    ) -> Self {
        Self {
            params,
            api: "Test host".into(),
            setter: Rc::new(setter),
        }
    }
}
