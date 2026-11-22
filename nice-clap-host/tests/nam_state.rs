//! Verify the application's revision workaround through the real CLAP ABI.
#![deny(unsafe_op_in_unsafe_fn)]
use clack_extensions::{params::*, state::PluginState};
use clack_host::{
    events::{
        Pckn,
        event_types::ParamValueEvent,
        io::{InputEvents, OutputEvents},
    },
    prelude::*,
};

struct Host;
impl HostHandlers for Host {
    type Shared<'a> = ();
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();
}

#[test]
#[ignore = "requires NICE_RACK_TEST_NAM pointing to a freshly built nice-nam CLAP"]
fn asset_revision_is_hidden_non_automatable_and_round_trips_through_state() {
    let path = std::env::var_os("NICE_RACK_TEST_NAM").expect("Set NICE_RACK_TEST_NAM");
    // SAFETY: explicit opt-in to this repository's freshly built NAM export.
    let entry = unsafe { PluginEntry::load(std::path::Path::new(&path)).unwrap() };
    let info = HostInfo::new(
        "NAM state regression",
        "frost",
        "https://github.com/perlindgren/frost",
        "0.1.0",
    )
    .unwrap();
    let mut instance =
        PluginInstance::<Host>::new(|_| (), |_| (), &entry, c"com.frost.nice-nam", &info).unwrap();
    let handle = instance.plugin_handle();
    let params = handle.get_extension::<PluginParams>().unwrap();
    let state = handle.get_extension::<PluginState>().unwrap();
    let mut scratch = ParamInfoBuffer::new();
    assert_eq!(params.count(&handle), 1);
    let revision = params.get_info(&handle, 0, &mut scratch).unwrap();
    assert_eq!(revision.name, b"Asset revision");
    assert!(revision.flags.contains(ParamInfoFlags::IS_HIDDEN));
    assert!(!revision.flags.contains(ParamInfoFlags::IS_AUTOMATABLE));
    let id = revision.id;
    let mut initial = Vec::new();
    state.save(&handle, &mut initial).unwrap();
    let event = ParamValueEvent::new(0, id, Pckn::match_all(), 7.0);
    params.flush(
        &mut instance.inactive_plugin_handle().unwrap(),
        &InputEvents::from_buffer(&event),
        &mut OutputEvents::void(),
    );
    let handle = instance.plugin_handle();
    assert!((params.get_value(&handle, id).unwrap() - 7.0).abs() < 1e-4);
    let mut saved = Vec::new();
    state.save(&handle, &mut saved).unwrap();
    assert_ne!(saved, initial);
    state.load(&handle, &mut initial.as_slice()).unwrap();
    assert_eq!(params.get_value(&handle, id), Some(0.0));
    state.load(&handle, &mut saved.as_slice()).unwrap();
    assert!((params.get_value(&handle, id).unwrap() - 7.0).abs() < 1e-4);
}
