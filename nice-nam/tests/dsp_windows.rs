//! The shared DSP suite is `#[cfg(not(windows))]`, so without this file the
//! platform that builds a shipping binary would have no DSP coverage at all.
//! The model below is inline JSON rather than a checked-in fixture because
//! nam-rs publishes its crate without its own fixtures.
#![cfg(windows)]
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
        // One directory per test, named after the test itself, as `src/tell.rs`
        // does. `claim` scans `{tag}-{pid}-{n}` upward from zero, so tests sharing
        // a tag all reach for the same sequence while the others' directories are
        // being deleted behind them — and a directory pending deletion answers
        // `CreateDirectory` with access denied, not already-exists, which `claim`
        // reads as "no writable folder" and panics. Measured: twice in seven runs
        // of this file against a real `%TEMP%`, each time in whichever tests
        // happened to be claiming at that moment.
        let this_thread = std::thread::current();
        let who = this_thread
            .name()
            .and_then(|name| name.rsplit("::").next())
            .unwrap_or("test");
        Fixture(frost::scratch::claim(&format!(
            "nice-nam-dsp-windows-{who}"
        )))
    }
    /// Single-layer H=3 LSTM: zero recurrent and input weights, a 0.125 output
    /// bias, so its constant output separates inference from bypass.
    ///
    /// `input_size` is mandatory here. The shared suite's equivalent model omits
    /// it and loads on the other platforms, so a file accepted by one engine can
    /// be rejected by the other — a divergence worth keeping visible in a test.
    fn lstm(root: &std::path::Path, name: &str, sample_rate: u32) -> std::path::PathBuf {
        let path = root.join(name);
        let weights = format!("{},0.125", vec!["0.0"; 69].join(","));
        std::fs::write(
            &path,
            format!(
                r#"{{"version":"0.5.4","architecture":"LSTM","config":{{"input_size":1,"hidden_size":3,"num_layers":1}},"weights":[{weights}],"sample_rate":{sample_rate}}}"#
            ),
        )
        .unwrap();
        path
    }
    fn model(path: std::path::PathBuf) -> NamPreferences {
        NamPreferences {
            model_name: Some("gain".into()),
            model_path: Some(path),
            ..Default::default()
        }
    }
    #[test]
    fn a_settled_model_processes_variable_blocks_without_allocating() {
        let root = fixture();
        let paths = model(lstm(&root.0, "gain.nam", 48000));
        let (mut chain, description) = Chain::load(&paths, 48000).unwrap();
        assert!(description.contains("LSTM"), "{description}");
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
    fn a_model_asking_for_another_rate_is_refused_rather_than_run_wrong() {
        let root = fixture();
        let error = Chain::load(&model(lstm(&root.0, "gain441.nam", 44100)), 48000)
            .err()
            .unwrap();
        assert!(
            error.contains("44100") && error.contains("48000"),
            "{error}"
        );
    }
    #[test]
    fn a_missing_model_is_reported_with_the_path_that_failed() {
        let root = fixture();
        let error = Chain::load(&model(root.0.join("missing.nam")), 48000)
            .err()
            .unwrap();
        assert!(error.contains("missing.nam"), "{error}");
    }
    #[test]
    fn an_unsupported_architecture_names_the_file_and_says_why() {
        let root = fixture();
        let path = root.0.join("linear.nam");
        std::fs::write(
            &path,
            r#"{"version":"0.5.4","architecture":"Linear","config":{},"weights":[]}"#,
        )
        .unwrap();
        let error = Chain::load(&model(path), 48000).err().unwrap();
        assert!(error.contains("linear.nam"), "{error}");
    }
    #[test]
    fn a_binary_container_is_named_rather_than_reported_as_broken_json() {
        // .namb loads on macOS and Linux, so the honest message is about the
        // container and this build, not about a file being invalid.
        let root = fixture();
        let path = root.0.join("packed.nam");
        let mut bytes = b"NAMB".to_vec();
        bytes.extend(std::iter::repeat_n(0u8, 512));
        std::fs::write(&path, bytes).unwrap();
        let error = Chain::load(&model(path), 48000).err().unwrap();
        assert!(error.contains(".namb"), "{error}");
        assert!(error.contains("packed.nam"), "{error}");
    }
    #[test]
    fn a_container_renamed_to_nam_still_gets_the_container_message() {
        // The check reads the file's first four bytes, so renaming does not turn
        // a clear message into a parse failure.
        let root = fixture();
        let path = root.0.join("looks-like-json.nam");
        std::fs::write(&path, *b"NAMB{}").unwrap();
        let error = Chain::load(&model(path), 48000).err().unwrap();
        assert!(error.contains(".namb"), "{error}");
    }
    #[test]
    fn a_weight_that_is_not_a_number_is_refused_before_it_can_run() {
        // nam-rs copies weights into the audio buffers without looking at them,
        // and `1e39` arrives as +∞ rather than as an error.
        let root = fixture();
        let path = root.0.join("rotten.nam");
        let mut weights = vec!["0.0"; 70];
        weights[41] = "1e39";
        std::fs::write(
            &path,
            format!(
                r#"{{"version":"0.5.4","architecture":"LSTM","config":{{"input_size":1,"hidden_size":3,"num_layers":1}},"weights":[{}],"sample_rate":48000}}"#,
                weights.join(",")
            ),
        )
        .unwrap();
        let error = Chain::load(&model(path), 48000).err().unwrap();
        assert!(error.contains("weight 41 of 70 is not a number"), "{error}");
    }
    #[test]
    fn a_calibration_level_beyond_an_audio_level_is_refused() {
        let root = fixture();
        let path = root.0.join("impossible.nam");
        std::fs::write(
            &path,
            format!(
                r#"{{"version":"0.5.4","metadata":{{"loudness":1000.0}},"architecture":"LSTM","config":{{"input_size":1,"hidden_size":3,"num_layers":1}},"weights":[{}],"sample_rate":48000}}"#,
                vec!["0.0"; 69].join(",") + ",0.125"
            ),
        )
        .unwrap();
        let error = Chain::load(&model(path), 48000).err().unwrap();
        assert!(
            error.contains("loudness") && error.contains("60"),
            "{error}"
        );
    }
    #[test]
    fn a_file_that_predates_the_sample_rate_field_says_what_was_assumed() {
        let root = fixture();
        let path = root.0.join("undated.nam");
        let weights = format!("{},0.125", vec!["0.0"; 69].join(","));
        std::fs::write(
            &path,
            format!(
                r#"{{"version":"0.5.4","architecture":"LSTM","config":{{"input_size":1,"hidden_size":3,"num_layers":1}},"weights":[{weights}]}}"#
            ),
        )
        .unwrap();
        let (mut chain, description) = Chain::load(&model(path.clone()), 48000).unwrap();
        assert!(description.contains("assumed 48 kHz"), "{description}");
        // An assumed rate is still a rate the host must match: at 44.1 kHz the
        // file is refused, exactly as a file that declared 48 kHz would be.
        assert!(Chain::load(&model(path), 44100).is_err());
        let mut output = [0.0; BLOCK];
        chain.process(&[0.25; BLOCK], &mut output);
        assert!(output.iter().all(|v| (*v - 0.125).abs() < 1e-5));
    }
    #[test]
    fn the_size_limit_refuses_on_both_sides_of_the_boundary() {
        assert!(dsp::size_is_plausible(48 * 1024));
        assert!(dsp::size_is_plausible(256 * 1024 * 1024));
        assert!(!dsp::size_is_plausible(256 * 1024 * 1024 + 1));
    }
    #[test]
    fn a_cabinet_path_is_ignored_and_says_so_instead_of_passing_for_dsp() {
        let root = fixture();
        let paths = NamPreferences {
            cabinet_path: Some(root.0.join("ir.wav")),
            ..model(lstm(&root.0, "gain.nam", 48000))
        };
        let (mut chain, description) = Chain::load(&paths, 48000).unwrap();
        assert!(description.contains("Cabinet IR ignored"), "{description}");
        // Nothing is convolved, so nothing may be claimed: a host that compensated
        // for a latency this build does not have would shift the dry signal.
        assert_eq!(chain.latency(), 0);
        let mut output = [0.0; BLOCK];
        chain.process(&[0.25; BLOCK], &mut output);
        assert!(output.iter().all(|v| (*v - 0.125).abs() < 1e-5));
    }
    /// The same LSTM with every gate weight and head weight live, so its output
    /// moves with the input. A test that scaled the constant-output model above
    /// would prove nothing about the input side, because that model ignores it.
    fn sensitive_lstm(root: &std::path::Path, name: &str, metadata: &str) -> std::path::PathBuf {
        let path = root.join(name);
        let mut weights = vec![0.0f32; 70];
        // Every input-to-gate and hidden-to-gate weight live, and every head weight:
        // which column of the four is the input is nam-rs's layout decision, so the
        // fixture does not depend on guessing it. What matters is that the output
        // moves with the input, which the guard below insists on.
        weights[..48].fill(0.5);
        weights[66..69].fill(1.0);
        let body = weights
            .iter()
            .map(f32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        std::fs::write(
            &path,
            format!(
                r#"{{"version":"0.5.4",{metadata}"architecture":"LSTM","config":{{"input_size":1,"hidden_size":3,"num_layers":1}},"weights":[{body}],"sample_rate":48000}}"#
            ),
        )
        .unwrap();
        path
    }
    const CALIBRATION: &str =
        r#""metadata":{"loudness":-20.5,"input_level_dbu":18.3,"output_level_dbu":12.3},"#;
    #[test]
    fn the_files_calibration_becomes_two_numbers_in_the_status_line() {
        let root = fixture();
        let path = sensitive_lstm(&root.0, "calibrated.nam", CALIBRATION);
        let (_, description) = Chain::load(&model(path), 48000).unwrap();
        assert!(description.contains("Calibration applied"), "{description}");
        assert!(
            description.contains("input 18.3 dBu (-6.3 dB)"),
            "{description}"
        );
        assert!(
            description.contains("loudness -20.5 dBFS (+2.5 dB)"),
            "{description}"
        );
        // `output_level_dbu` is in the file and is not part of NAM's staging, so
        // it must not be reported as though it were applied.
        assert!(!description.contains("12.3"), "{description}");
    }
    #[test]
    fn a_model_without_calibration_is_left_exactly_alone() {
        let root = fixture();
        let path = sensitive_lstm(&root.0, "plain.nam", "");
        let (mut chain, description) = Chain::load(&model(path), 48000).unwrap();
        assert!(!description.contains("Calibration"), "{description}");
        let mut output = [0.0; BLOCK];
        chain.process(&[0.25; BLOCK], &mut output);
        let mut same = [0.0; BLOCK];
        let (mut again, _) =
            Chain::load(&model(sensitive_lstm(&root.0, "plain2.nam", "")), 48000).unwrap();
        again.process(&[0.25; BLOCK], &mut same);
        assert!(
            output.iter().zip(&same).all(|(a, b)| (a - b).abs() < 1e-6),
            "the same file must load to the same chain"
        );
    }
    #[test]
    fn the_input_is_scaled_before_the_model_and_the_output_after() {
        // Both halves of the staging, proved end to end without assuming the
        // model is linear: run the uncalibrated model on the signal the
        // calibrated chain will hand it, scale that result by the output
        // multiplier, and require the calibrated chain to agree sample for sample.
        let root = fixture();
        let input_gain = 10f32.powf((12.0 - 18.3) / 20.0);
        let output_gain = 10f32.powf((-18.0 - -20.5) / 20.0);
        let (mut reference, _) =
            Chain::load(&model(sensitive_lstm(&root.0, "reference.nam", "")), 48000).unwrap();
        let (mut calibrated, _) = Chain::load(
            &model(sensitive_lstm(&root.0, "calibrated.nam", CALIBRATION)),
            48000,
        )
        .unwrap();
        // The guard: if this model does not move with its input the test below
        // would pass with the input scaling removed entirely.
        let mut quiet = [0.0; BLOCK];
        let mut loud = [0.0; BLOCK];
        let (mut other, _) =
            Chain::load(&model(sensitive_lstm(&root.0, "guard.nam", "")), 48000).unwrap();
        reference.process(&[0.1; BLOCK], &mut quiet);
        other.process(&[0.6; BLOCK], &mut loud);
        assert!(
            quiet[0] != loud[0],
            "the test model must be input-sensitive: {} vs {}",
            quiet[0],
            loud[0]
        );
        let mut expected = [0.0; BLOCK];
        let (mut fresh, _) =
            Chain::load(&model(sensitive_lstm(&root.0, "fresh.nam", "")), 48000).unwrap();
        fresh.process(&[0.25 * input_gain; BLOCK], &mut expected);
        for sample in &mut expected {
            *sample *= output_gain;
        }
        let mut got = [0.0; BLOCK];
        calibrated.process(&[0.25; BLOCK], &mut got);
        for (index, (got, expected)) in got.iter().zip(&expected).enumerate() {
            assert!(
                (got - expected).abs() < 1e-5,
                "sample {index}: {got} against the staged reference {expected}"
            );
        }
    }
    #[test]
    fn a_non_finite_sample_does_not_leave_the_chain() {
        let root = fixture();
        let (mut chain, _) = Chain::load(&model(lstm(&root.0, "gain.nam", 48000)), 48000).unwrap();
        let mut poisoned = [0.25; BLOCK];
        poisoned[7] = f32::NAN;
        let mut output = [0.0; BLOCK];
        chain.process(&poisoned, &mut output);
        assert!(output.iter().all(|v| v.is_finite()));
    }
}
