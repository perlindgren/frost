//! Isolated from nice-plug's debug allocator, as in nice-tuner's RT tests.
#[path = "../src/dsp.rs"]
mod dsp;
#[allow(unused_imports)]
#[path = "../src/settings.rs"]
mod settings;
use dsp::*;
use settings::NamPreferences;
#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> Fixture {
        // Named after the test, so no two tests in this binary scan the same
        // `{tag}-{pid}-{n}` sequence: a directory one test is deleting answers the
        // next test's `CreateDirectory` with access denied, which `claim` reports
        // as an unwritable machine.
        let this_thread = std::thread::current();
        let who = this_thread
            .name()
            .and_then(|name| name.rsplit("::").next())
            .unwrap_or("test");
        Fixture(frost::scratch::claim(&format!("nice-nam-dsp-{who}")))
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
    fn a_file_that_is_not_a_model_is_named_and_refused() {
        // What a loader owes someone who pointed the picker at the wrong file: the
        // name of it, and no chain. The reason is the engine's own words; the path
        // is this build's, and the path is the half worth asserting here.
        let root = fixture();
        let path = root.0.join("not-a-model.nam");
        std::fs::write(&path, b"nothing of a model here").unwrap();
        let paths = NamPreferences {
            model_path: Some(path),
            ..Default::default()
        };
        let error = Chain::load(&paths, 48000).err().unwrap();
        assert!(error.contains("not-a-model.nam"), "{error}");
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
    #[test]
    fn the_derivation_kept_in_the_chain_agrees_with_the_loader() {
        // Nothing at runtime uses this derivation: the chain takes the loader's
        // own multipliers. It is kept, and held against the loader on the same
        // file, so that the convention stays written down somewhere in this build
        // and so that a change to either side of it fails here rather than
        // showing up as a level difference nobody can point at.
        #[cfg(not(windows))]
        use nam_engine::prelude::{LoadOptions, SystemSnapshot, load_and_build_model};
        #[cfg(windows)]
        use nam_windows::prelude::{LoadOptions, SystemSnapshot, load_and_build_model};
        let root = fixture();
        let path = root.0.join("calibrated.nam");
        let mut weights = vec![0.0f32; 70];
        *weights.last_mut().unwrap() = 0.125;
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "version": "0.5.4",
                "metadata": { "loudness": -20.5, "input_level_dbu": 18.3 },
                "architecture": "LSTM",
                "config": { "num_layers": 1, "hidden_size": 3 },
                "weights": weights, "sample_rate": 48000
            }))
            .unwrap(),
        )
        .unwrap();
        let pair = load_and_build_model(
            &path,
            &SystemSnapshot::capture(),
            false,
            LoadOptions::default(),
        )
        .unwrap();
        let (input, output) = dsp::calibration(Some(18.3), Some(-20.5));
        // That loader converts through a 0.1 dB table, so exact arithmetic is
        // allowed to differ by a hair — and only by a hair.
        assert!(
            (input - pair.input_mult_adj).abs() < 1e-3,
            "input: {input} against the loader's {}",
            pair.input_mult_adj
        );
        assert!(
            (output - pair.output_mult_adj).abs() < 1e-3,
            "output: {output} against the loader's {}",
            pair.output_mult_adj
        );
    }
}
