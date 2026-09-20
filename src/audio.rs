//! Native audio: one-shot and looping playback of sounds decoded at load
//! time.
//!
//! A [`Sound`] is an in-memory buffer of f32 samples decoded from an audio
//! file (WAV, FLAC, MP3, OGG Vorbis, AAC, or M4A) at creation time: a
//! missing file or an undecodable format fails with [`AudioError`] up
//! front, not at play time, and once loaded a `Sound` can be played any
//! number of times without touching the disk or re-decoding. The samples
//! sit behind an `Arc`, so re-triggering a sound shares one buffer.
//!
//! An [`Audio`] owns the output device. One-shots mix in parallel: every
//! [`Audio::play_once`] adds its copy to the device's mixer, so the same
//! sound can be retriggered as fast as you like and the copies overlap
//! freely. The loop is a single sequential player: [`Audio::play_loop`]
//! starts or replaces the one loop, and [`Audio::stop_loop`] silences it.
//! The master volume, in `0.0..=1.0`, applies to one-shots when they are
//! triggered and to the loop live, so [`Audio::set_volume`] can be called
//! at any time. Dropping the `Audio` stops everything it started.
//!
//! This module is native-only: it is excluded from
//! `wasm32-unknown-unknown` builds, whose audio backend has no device
//! layer. The tests decode without opening a device, so `cargo test` runs
//! headless.

use std::io::{Read, Seek};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use rodio::buffer::SamplesBuffer;
use rodio::decoder::Decoder;
use rodio::source::Source;

/// The audio output: one open device, a shared mixer, and the single
/// looping player.
///
/// Open one with [`Audio::new`] at startup — it opens the default output
/// device and fails with [`AudioError::Device`] if there is none — keep it
/// in the demo state, and drive it from the [`Process`](crate::Process).
/// Dropping the `Audio` stops all of its sounds.
pub struct Audio {
    /// Owns the output device and its mixer; dropping it stops playback.
    sink: rodio::MixerDeviceSink,
    /// The single loop: a sequential player, never mixed with one-shots.
    loop_player: rodio::Player,
    /// The master volume, the bit representation of an f32 in `0.0..=1.0`.
    volume: AtomicU32,
}

impl Audio {
    /// Opens the default output device and prepares an `Audio` at full
    /// volume.
    ///
    /// # Errors
    ///
    /// [`AudioError::Device`] if there is no default output device or it
    /// cannot be opened.
    pub fn new() -> Result<Self, AudioError> {
        let mut sink = rodio::DeviceSinkBuilder::open_default_sink()
            .map_err(|err| AudioError::Device(Box::new(err)))?;
        sink.log_on_drop(false);
        let loop_player = rodio::Player::connect_new(sink.mixer());
        Ok(Self {
            sink,
            loop_player,
            volume: AtomicU32::new(1.0f32.to_bits()),
        })
    }

    /// Plays a one-shot: a copy of the sound's samples is added to the
    /// mixer at the current master volume.
    ///
    /// One-shots mix in parallel, so the same sound can be retriggered as
    /// fast as you like and the copies overlap freely; each copy runs to
    /// the end of the sound on its own.
    pub fn play_once(&self, sound: &Sound, volume: Option<f32>) {
        self.sink.mixer().add(
            sound
                .buffer
                .clone()
                .amplify(volume.unwrap_or_else(|| self.volume())),
        );
    }

    /// Starts looping a sound, or replaces the current loop with it.
    ///
    /// Only one sound can loop at a time: calling this while something is
    /// looping stops the old loop and starts the new one from the
    /// beginning. The master volume applies to the loop live, so later
    /// [`Audio::set_volume`] calls are heard on it.
    pub fn play_loop(&self, sound: &Sound) {
        self.loop_player.clear();
        self.loop_player
            .append(sound.buffer.clone().repeat_infinite());
        self.loop_player.play();
        self.loop_player.set_volume(self.volume());
    }

    /// Stops the current loop, if any.
    ///
    /// One-shots already in the mixer are not touched; they run out.
    pub fn stop_loop(&self) {
        self.loop_player.clear();
    }

    /// Sets the master volume, clamped to `0.0..=1.0`.
    ///
    /// The value applies to the current loop live and to one-shots from
    /// the next [`Audio::play_once`] on.
    pub fn set_volume(&self, volume: f32) {
        let volume = clamp01(volume);
        self.volume.store(volume.to_bits(), Ordering::SeqCst);
        self.loop_player.set_volume(volume);
    }

    /// The current master volume, in `0.0..=1.0`.
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::SeqCst))
    }
}

impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio")
            .field("volume", &self.volume())
            .finish_non_exhaustive()
    }
}

/// A sound decoded from an audio file: an in-memory buffer of f32 samples
/// that can be played any number of times.
///
/// Load one with [`Sound::load`] (or [`Sound::load_bytes`]) before the
/// first play: the file is read and decoded up front, so a bad path or
/// format is an [`AudioError`] at load time, not at play time.
#[derive(Debug)]
pub struct Sound {
    buffer: SamplesBuffer,
}

impl Sound {
    /// Reads and decodes a sound from a file.
    ///
    /// Supported formats are what the audio backend decodes: WAV, FLAC,
    /// MP3, OGG Vorbis, AAC, and M4A.
    ///
    /// # Errors
    ///
    /// [`AudioError::Io`] if the file cannot be read, [`AudioError::Decode`]
    /// if it is not a decodable audio format.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, AudioError> {
        let file = std::fs::File::open(path.as_ref()).map_err(AudioError::Io)?;
        Self::from_reader(file)
    }

    /// Decodes a sound from bytes already in memory, for example bytes
    /// embedded into the binary with `include_bytes!`.
    ///
    /// # Errors
    ///
    /// [`AudioError::Decode`] if the bytes are not a decodable audio
    /// format.
    ///
    /// # Example
    ///
    /// A tiny mono WAV built in code — no file, no device:
    ///
    /// ```
    /// let mut wav: Vec<u8> = Vec::new();
    /// let u16 = |v: &mut Vec<u8>, n: u16| v.extend_from_slice(&n.to_le_bytes());
    /// let u32 = |v: &mut Vec<u8>, n: u32| v.extend_from_slice(&n.to_le_bytes());
    /// wav.extend_from_slice(b"RIFF");
    /// u32(&mut wav, 36 + 441 * 2);
    /// wav.extend_from_slice(b"WAVEfmt ");
    /// u32(&mut wav, 16);
    /// u16(&mut wav, 1);
    /// u16(&mut wav, 1);
    /// u32(&mut wav, 44_100);
    /// u32(&mut wav, 44_100 * 2);
    /// u16(&mut wav, 2);
    /// u16(&mut wav, 16);
    /// wav.extend_from_slice(b"data");
    /// u32(&mut wav, 441 * 2);
    /// for i in 0..441u16 {
    ///     wav.extend_from_slice(&(i as i16).to_le_bytes());
    /// }
    /// let sound = frost::Sound::load_bytes(&wav).unwrap();
    /// assert_eq!((sound.channels(), sound.sample_rate()), (1, 44_100));
    /// assert!((sound.duration() - 441.0 / 44_100.0).abs() < 1e-3);
    /// ```
    pub fn load_bytes(bytes: impl AsRef<[u8]>) -> Result<Self, AudioError> {
        let owned = bytes.as_ref().to_vec();
        Self::from_reader(std::io::Cursor::new(owned))
    }

    fn from_reader<R: Read + Seek + Send + Sync + 'static>(reader: R) -> Result<Self, AudioError> {
        let decoder = Decoder::new(reader).map_err(AudioError::Decode)?;
        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();
        let samples: Vec<f32> = decoder.collect();
        Ok(Self {
            buffer: SamplesBuffer::new(channels, sample_rate, samples),
        })
    }

    /// The number of channels in the decoded sound: 1 for mono, 2 for
    /// stereo.
    pub fn channels(&self) -> u16 {
        self.buffer.channels().get()
    }

    /// The sample rate of the decoded sound in samples per second, such as
    /// 44_100.
    pub fn sample_rate(&self) -> u32 {
        self.buffer.sample_rate().get()
    }

    /// The sound's length in seconds.
    pub fn duration(&self) -> f32 {
        self.buffer
            .total_duration()
            .unwrap_or_default()
            .as_secs_f32()
    }
}

/// An error while creating an [`Audio`] or loading a [`Sound`].
#[derive(Debug)]
pub enum AudioError {
    /// The sound file could not be read.
    Io(std::io::Error),
    /// The file was read but is not a decodable audio format.
    Decode(rodio::decoder::DecoderError),
    /// The default output device could not be opened.
    Device(Box<rodio::DeviceSinkError>),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read the sound file: {err}"),
            Self::Decode(err) => {
                write!(f, "failed to decode the sound file as audio: {err}")
            }
            Self::Device(err) => write!(f, "failed to open the audio output device: {err}"),
        }
    }
}

impl std::error::Error for AudioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Decode(err) => Some(err),
            Self::Device(err) => Some(err),
        }
    }
}

/// Clamps a volume to `0.0..=1.0`.
fn clamp01(volume: f32) -> f32 {
    volume.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal PCM16 WAV from interleaved samples in `-1.0..=1.0`.
    fn wav_pcm16(channels: u16, sample_rate: u32, samples: &[f32]) -> Vec<u8> {
        let data_len = samples.len() * 2;
        let mut wav: Vec<u8> = Vec::new();
        let u16 = |v: &mut Vec<u8>, n: u16| v.extend_from_slice(&n.to_le_bytes());
        let u32 = |v: &mut Vec<u8>, n: u32| v.extend_from_slice(&n.to_le_bytes());
        wav.extend_from_slice(b"RIFF");
        u32(&mut wav, 36 + data_len as u32);
        wav.extend_from_slice(b"WAVEfmt ");
        u32(&mut wav, 16);
        u16(&mut wav, 1); // PCM
        u16(&mut wav, channels);
        u32(&mut wav, sample_rate);
        u32(&mut wav, sample_rate * channels as u32 * 2); // byte rate
        u16(&mut wav, channels * 2); // block align
        u16(&mut wav, 16); // bits per sample
        wav.extend_from_slice(b"data");
        u32(&mut wav, data_len as u32);
        for s in samples {
            let s = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
            wav.extend_from_slice(&s.to_le_bytes());
        }
        wav
    }

    /// A 44 Hz square wave, 800 interleaved mono samples (~0.1 s at 8 kHz).
    fn square_wave(frames: usize, sample_rate: u32) -> Vec<f32> {
        (0..frames)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                if (t * 44.0) % 1.0 < 0.5 { 0.5 } else { -0.5 }
            })
            .collect()
    }

    #[test]
    fn synthetic_wav_decodes() {
        let samples = square_wave(800, 8_000);
        let sound = Sound::load_bytes(&wav_pcm16(1, 8_000, &samples)).unwrap();
        assert_eq!(sound.channels(), 1);
        assert_eq!(sound.sample_rate(), 8_000);
        let frames = sound.duration() * sound.sample_rate() as f32 * sound.channels() as f32;
        assert!(
            (frames - 800.0).abs() < 1.0,
            "expected ~800 frames, got {frames}"
        );
    }

    #[test]
    fn demo_wav_decodes() {
        let sound =
            Sound::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/audio/swoof.wav"))
                .expect("the demo asset should decode");
        assert_eq!(sound.channels(), 2);
        assert_eq!(sound.sample_rate(), 44_100);
        let frames = sound.duration() * sound.sample_rate() as f32 * sound.channels() as f32;
        assert!(frames > 10_000.0, "expected >10_000 frames, got {frames}");
        assert!(sound.duration().is_finite());
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let err = Sound::load("definitely-not-here/sound.wav").unwrap_err();
        assert!(matches!(err, AudioError::Io(_)));
    }

    #[test]
    fn garbage_bytes_are_a_decode_error() {
        let err = Sound::load_bytes(b"not an audio file").unwrap_err();
        assert!(matches!(err, AudioError::Decode(_)));
    }

    #[test]
    fn volume_clamps_and_round_trips() {
        assert_eq!(clamp01(-0.5), 0.0);
        assert_eq!(clamp01(1.5), 1.0);
        assert_eq!(clamp01(0.25), 0.25);
        for v in [0.0f32, 0.25, 0.75, 1.0] {
            assert_eq!(f32::from_bits(v.to_bits()), v);
        }
    }
}
