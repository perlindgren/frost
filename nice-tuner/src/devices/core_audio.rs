//! Native defaults not exposed by CPAL's stream-configuration API.

#![forbid(unsafe_code)]

use core_audio_device::Device;

#[derive(Default)]
pub struct Defaults {
    pub input_channel: Option<u32>,
    pub buffer_frames: Option<u32>,
    pub output_channels: Option<[u32; 2]>,
}

pub fn defaults(name: &str) -> Defaults {
    let Ok(Some(device)) = Device::find_by_name(name) else {
        return Defaults::default();
    };
    // The first preferred stereo input channel is our mono-input policy.
    // Unsupported properties remain unset, independently for each query.
    Defaults {
        input_channel: device.preferred_input_channels().ok().map(|pair| pair[0]),
        buffer_frames: device.buffer_frames().ok(),
        output_channels: device.preferred_output_channels().ok(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_device_does_not_invent_channel_or_buffer_defaults() {
        let defaults = super::defaults("__nice_tuner_missing_device_default_query_test__");
        assert_eq!(defaults.input_channel, None);
        assert_eq!(defaults.buffer_frames, None);
        assert_eq!(defaults.output_channels, None);
    }
}
