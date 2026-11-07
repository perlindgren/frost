//! Isolated from nice-plug's debug allocator, as in nice-tuner's RT tests.
#![cfg(not(windows))]
#[path = "../src/dsp.rs"]
mod dsp;
#[allow(unused_imports)]
#[path = "../src/settings.rs"]
mod settings;
use dsp::*;
use settings::NamPreferences;
#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> Fixture {
        let root = frost::scratch::claim("nice-nam-dsp");
        Fixture(root)
    }
    fn model(root: &std::path::Path) -> NamPreferences {
        let path = root.join("gain.nam");
        // Single-layer H=3 LSTM: zero recurrent/input weights and a 0.125
        // output bias. Its constant output distinguishes inference from bypass.
        let mut weights = vec![0.0f32; 70];
        *weights.last_mut().unwrap() = 0.125;
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "version": "0.5.4", "architecture": "LSTM",
                "config": { "num_layers": 1, "hidden_size": 3 },
                "weights": weights, "sample_rate": 48000
            }))
            .unwrap(),
        )
        .unwrap();
        NamPreferences {
            model_name: Some("gain".into()),
            model_path: Some(path),
            ..Default::default()
        }
    }
    #[test]
    fn neural_model_processes_variable_blocks_without_allocating() {
        let root = fixture();
        let paths = model(&root.0);
        let (mut chain, specs) = Chain::load(&paths, 48000).unwrap();
        assert!(specs.contains("LSTM"));
        assert_eq!(chain.rate, 48000);
        chain.reset();
        let input = [0.25; BLOCK];
        let mut output = [0.0; BLOCK];
        chain.process(&input, &mut output);
        assert!(output.iter().all(|v| (*v - 0.125).abs() < 1e-5));
        let allocations = allocation_counter::measure(|| {
            for n in [1, 32, 48, 96, 256] {
                chain.process(&input[..n], &mut output[..n]);
            }
        });
        assert_eq!(allocations.count_total, 0);
    }
    #[test]
    fn mismatched_rate_and_missing_files_are_reported() {
        let root = fixture();
        let mut paths = model(&root.0);
        assert!(Chain::load(&paths, 44100).err().unwrap().contains("48000"));
        paths.model_path = Some(root.0.join("missing.nam"));
        assert!(Chain::load(&paths, 48000).is_err());
    }
    #[test]
    fn cabinet_convolves_and_reports_its_latency_without_allocating() {
        let root = fixture();
        let mut paths = NamPreferences::default();
        let path = root.0.join("identity.wav");
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend(40u32.to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(3u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(48000u32.to_le_bytes());
        wav.extend(192000u32.to_le_bytes());
        wav.extend(4u16.to_le_bytes());
        wav.extend(32u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(4u32.to_le_bytes());
        wav.extend(1f32.to_le_bytes());
        std::fs::write(&path, wav).unwrap();
        paths.cabinet_path = Some(path);
        let (mut chain, _) = Chain::load(&paths, 48000).unwrap();
        let latency = chain.latency() as usize;
        let mut input = [0.0; BLOCK];
        input[0] = 1.0;
        let mut output = [0.0; BLOCK];
        let mut captured = Vec::new();
        for _ in 0..4 {
            chain.process(&input, &mut output);
            captured.extend_from_slice(&output);
            input.fill(0.0);
        }
        assert!((captured[latency] - 1.0).abs() < 1e-4);
        let allocations = allocation_counter::measure(|| chain.process(&input, &mut output));
        assert_eq!(allocations.count_total, 0);
    }
}
