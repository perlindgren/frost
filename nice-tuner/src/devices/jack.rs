//! JACK graph discovery. These are real mono audio ports, not CPAL devices.
use super::{JACK_BUFFER_CHOICES, Snapshot, StreamFormat};
use crate::settings::OutputChannels;
use jack::{Client, ClientOptions, PortFlags, PortSpec};

pub(crate) const CLIENT_PREFIX: &str = "nice-tuner-";

pub(super) fn snapshot() -> Snapshot {
    let Ok((client, _)) = Client::new("nice-tuner-discovery", ClientOptions::NO_START_SERVER)
    else {
        return Snapshot::default();
    };
    let audio = jack::AudioIn::default();
    let audio_type = audio.jack_port_type();
    let ports = |flags| {
        let mut names = client.ports(None, Some(audio_type), flags);
        names.retain(|name| !name.starts_with(CLIENT_PREFIX));
        names.sort();
        names.dedup();
        names
    };
    // Capture sources OUTPUT samples; playback destinations INPUT them.
    let inputs = ports(PortFlags::IS_OUTPUT);
    let outputs = ports(PortFlags::IS_INPUT);
    let physical_input = ports(PortFlags::IS_OUTPUT | PortFlags::IS_PHYSICAL);
    let physical_output = ports(PortFlags::IS_INPUT | PortFlags::IS_PHYSICAL);
    let rate = client.sample_rate();
    let frames = client.buffer_size();
    let format = StreamFormat {
        channels: 1,
        min_rate: rate,
        max_rate: rate,
        buffer_range: None, // alternatives are trial requests, not enumerated limits
    };
    Snapshot {
        def_in: physical_input.first().or(inputs.first()).cloned(),
        def_out: physical_output.first().or(outputs.first()).cloned(),
        inputs,
        outputs,
        rates: vec![rate as f32],
        output_rates: vec![rate as f32],
        driver_rate: Some(rate as f32),
        default_output_rate: Some(rate as f32),
        buffers: JACK_BUFFER_CHOICES.to_vec(),
        output_buffers: JACK_BUFFER_CHOICES.to_vec(),
        buffers_linked: true,
        buffer_pref: Some(frames),
        output_buffer_pref: Some(frames),
        input_channels: Some(1),
        default_input_channel: Some(1),
        input_formats: vec![format.clone()],
        output_formats: vec![format],
        output_channel_options: vec![OutputChannels::Mono(1)],
        default_output_channels: Some(OutputChannels::Mono(1)),
    }
}
