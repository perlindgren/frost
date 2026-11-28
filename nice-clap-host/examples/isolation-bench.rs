//! Reproducible, opt-in host performance comparison. No audio devices or new
//! dependencies. Each case runs in a fresh process with production CLAP paths.
#![deny(unsafe_op_in_unsafe_fn)]
#[path = "isolation_bench/case.rs"]
mod case;
#[path = "isolation_bench/gui.rs"]
mod gui;
#[path = "isolation_bench/model.rs"]
mod model;
use model::*;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--clap-helper") {
        let result = match (args.get(2), args.get(3)) {
            // SAFETY: this executable only loads explicitly selected benchmark
            // fixtures. Native plugin code must obey the CLAP ABI.
            (Some(address), Some(token)) => unsafe {
                nice_clap_host::isolated::run_helper(address, token)
            },
            _ => Err("Missing helper arguments".into()),
        };
        if let Err(error) = result {
            eprintln!("helper: {error}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("--gui-helper") {
        let result = (|| {
            let job: Job = read(Path::new(args.get(2).ok_or("Missing GUI job")?))?;
            let ready = PathBuf::from(args.get(3).ok_or("Missing GUI readiness path")?);
            let index = args
                .get(4)
                .ok_or("Missing GUI index")?
                .parse::<usize>()
                .map_err(|e| e.to_string())?;
            let result = case::gui_only(job, ready.clone(), index);
            if let Err(error) = &result {
                let _ = gui::publish_ready(&ready, &Err(error.clone()));
            }
            result
        })();
        if let Err(error) = result {
            eprintln!("GUI-only helper: {error}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("--case") {
        let result = (|| {
            let input = Path::new(args.get(2).ok_or("Missing job file")?);
            let output = Path::new(args.get(3).ok_or("Missing result file")?);
            let job: Job = read(input)?;
            let row = match case::run(job.clone()) {
                Ok(row) => row,
                Err(error) => unavailable(job, error),
            };
            nice_clap_host::isolated::finish_shutdown();
            write(output, &row)
        })();
        if let Err(error) = result {
            eprintln!("benchmark case: {error}");
            nice_clap_host::isolated::finish_shutdown();
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        help();
        return;
    }
    if let Err(error) = run(&args[1..]) {
        eprintln!("isolation-bench: {error}");
        std::process::exit(1);
    }
}
fn help() {
    println!(
        "\
Compare production in-process and isolated CLAP hosting (release build required).
  --plugin PATH       Repeat for each plugin in order; selects its first class
  --copies N          Repeat the plugin list N times (default 1)
  --frames N,N,...     Buffer sizes (default 64)
  --rate HZ           Sample rate (default 48000)
  --seconds N         Measured time per case (default 3)
  --warmup N          Warmup seconds (default 1)
  --trials N          Paired trials with alternating mode order (default 3)
  --topology NAME     serial, parallel, or both (default both)
  --editors           Open all native editors; needs a real desktop
  --gui-isolate       Three-way test using inactive editor replicas in ALL modes
                      Without --editors, helpers are headless controls (not GUI timing)
  --out PATH          RON report (default target/isolation-bench.ron)
  --machine LABEL     Optional machine description recorded in report
  --timeout N         Hard case timeout seconds (default 60)
No audio device is opened. GUI results measure host heartbeat delays, NOT editor FPS.
Cases start with default plugin state; a NAM without a loaded model is not a heavy NAM test.
"
    );
}
struct Options {
    plugins: Vec<PathBuf>,
    copies: usize,
    frames: Vec<usize>,
    rate: f64,
    seconds: f64,
    warmup: f64,
    trials: usize,
    topologies: Vec<Topology>,
    editors: bool,
    gui_isolate: bool,
    output: PathBuf,
    machine: String,
    timeout: f64,
}
fn options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        plugins: Vec::new(),
        copies: 1,
        frames: vec![64],
        rate: 48000.,
        seconds: 3.,
        warmup: 1.,
        trials: 3,
        topologies: vec![Topology::Serial, Topology::Parallel],
        editors: false,
        gui_isolate: false,
        output: PathBuf::from("target/isolation-bench.ron"),
        machine: String::new(),
        timeout: 60.,
    };
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if flag == "--gui-isolate" {
            o.gui_isolate = true;
            continue;
        }
        if flag == "--editors" {
            o.editors = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {flag}"))?;
        let number = || {
            value
                .parse::<f64>()
                .map_err(|_| format!("Invalid {flag}: {value}"))
        };
        let integer = || {
            value
                .parse::<usize>()
                .map_err(|_| format!("Invalid {flag}: {value}"))
        };
        match flag.as_str() {
            "--plugin" => o.plugins.push(
                PathBuf::from(value)
                    .canonicalize()
                    .map_err(|e| format!("{value}: {e}"))?,
            ),
            "--copies" => o.copies = integer()?,
            "--frames" => {
                o.frames = value
                    .split(',')
                    .map(|s| {
                        s.parse::<usize>()
                            .map_err(|_| "Invalid buffer size".to_owned())
                    })
                    .collect::<Result<_, _>>()?
            }
            "--rate" => o.rate = number()?,
            "--seconds" => o.seconds = number()?,
            "--warmup" => o.warmup = number()?,
            "--trials" => o.trials = integer()?,
            "--out" => o.output = value.into(),
            "--machine" => o.machine = value.clone(),
            "--timeout" => o.timeout = number()?,
            "--topology" => {
                o.topologies = match value.as_str() {
                    "serial" => vec![Topology::Serial],
                    "parallel" => vec![Topology::Parallel],
                    "both" => vec![Topology::Serial, Topology::Parallel],
                    _ => return Err("Unknown topology".into()),
                }
            }
            _ => return Err(format!("Unknown option {flag}")),
        }
    }
    if o.plugins.is_empty() || o.copies == 0 || o.plugins.len().saturating_mul(o.copies) > 32 {
        return Err("Supply 1..=32 plugin instances using --plugin and --copies".into());
    }
    if o.frames.is_empty()
        || o.frames.len() > 16
        || o.frames.iter().any(|n| !(1..=65536).contains(n))
    {
        return Err("Invalid buffer sizes".into());
    }
    if !o.rate.is_finite()
        || !(8000. ..=384000.).contains(&o.rate)
        || !o.seconds.is_finite()
        || !(0.05..=120.).contains(&o.seconds)
        || !o.warmup.is_finite()
        || !(0. ..=30.).contains(&o.warmup)
        || !(1..=20).contains(&o.trials)
        || !o.timeout.is_finite()
        || o.timeout < o.seconds + o.warmup + 2.
        || o.timeout > 600.
    {
        return Err("Invalid rate/duration/trials/timeout limits".into());
    }
    if o.frames
        .iter()
        .any(|&f| (o.seconds + o.warmup) * o.rate / f as f64 > 1_000_000.)
    {
        return Err("More than one million blocks per case".into());
    }
    Ok(o)
}
fn run(args: &[String]) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("Build/run this benchmark with --release".into());
    }
    let o = options(args)?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let scratch = Scratch::new()?;
    let plugins = o
        .plugins
        .iter()
        .cycle()
        .take(o.plugins.len() * o.copies)
        .cloned()
        .collect::<Vec<_>>();
    let rustc = Command::new("rustc")
        .arg("-vV")
        .output()
        .ok()
        .map(|v| String::from_utf8_lossy(&v.stdout).into_owned())
        .unwrap_or_default();
    let mut report = Report {
        architecture: std::env::consts::ARCH.into(),
        os: std::env::consts::OS.into(),
        machine: o.machine,
        rustc,
        compile_flags: option_env!("CARGO_ENCODED_RUSTFLAGS")
            .unwrap_or("not captured")
            .into(),
        rows: Vec::new(),
        comparisons: Vec::new(),
    };
    println!(
        "mode       graph     frames trial completed/attempted  DSP median/p99/worst-call us  misses  host p99/max late ms  status"
    );
    for frames in o.frames {
        for &topology in &o.topologies {
            for trial in 1..=o.trials {
                // Rotate all three positions for the GUI-only experiment;
                // the ordinary two-mode test retains its AB/BA ordering.
                let mut modes = if o.gui_isolate {
                    vec![Mode::InProcess, Mode::Isolated, Mode::GuiIsolated]
                } else {
                    vec![Mode::InProcess, Mode::Isolated]
                };
                let count = modes.len();
                if ((trial - 1) / count) % 2 == 1 {
                    modes.reverse();
                }
                modes.rotate_left((trial - 1) % count);
                let first_row = report.rows.len();
                for mode in modes {
                    let job = Job {
                        plugins: plugins.clone(),
                        mode,
                        topology,
                        frames,
                        rate: o.rate,
                        seconds: o.seconds,
                        warmup: o.warmup,
                        editors: o.editors,
                        editor_replicas: o.gui_isolate,
                        trial,
                    };
                    let input = scratch.0.join("job.ron");
                    let output = scratch.0.join("result.ron");
                    write(&input, &job)?;
                    let _ = std::fs::remove_file(&output);
                    let mut child = Command::new(&executable)
                        .arg("--case")
                        .arg(&input)
                        .arg(&output)
                        .stdin(Stdio::null())
                        .spawn()
                        .map_err(|e| e.to_string())?;
                    let deadline = Instant::now() + Duration::from_secs_f64(o.timeout);
                    let error = loop {
                        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                            break if status.success() {
                                None
                            } else {
                                Some(format!("Case exited {status}"))
                            };
                        }
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            break Some("Case timed out; results unavailable".into());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    };
                    let external_failure = error.is_some();
                    let row = match error {
                        Some(error) => unavailable(job, error),
                        None => read(&output).unwrap_or_else(|error| {
                            unavailable(job, format!("Missing/invalid case result: {error}"))
                        }),
                    };
                    print_row(&row);
                    report.rows.push(row);
                    // Save after every case, so later native crashes/hangs cannot erase
                    // previous trials. A failed row is recorded, not treated as zero cost.
                    write(&o.output, &report)?;
                    if external_failure {
                        // A killed case cannot drain its own helper reapers.
                        // Let their two-second parent-loss watchdog expire before
                        // measuring another case on this machine.
                        std::thread::sleep(Duration::from_millis(2200));
                    }
                }
                let rows = &report.rows[first_row..];
                if let Some(baseline) = rows
                    .iter()
                    .find(|row| matches!(row.job.mode, Mode::InProcess))
                {
                    for row in rows
                        .iter()
                        .filter(|row| !matches!(row.job.mode, Mode::InProcess))
                    {
                        let pair = compare(baseline, row);
                        if let Some(reason) = &pair.unavailable_reason {
                            println!("  {:?}/baseline unavailable: {reason}", pair.mode);
                        } else {
                            println!(
                                "  {:?}/baseline DSP median {:.2}x, p99 {:.2}x; host p99 lateness delta {:+.3} ms",
                                pair.mode,
                                pair.dsp_median_ratio.unwrap_or(f64::NAN),
                                pair.dsp_p99_ratio.unwrap_or(f64::NAN),
                                pair.host_p99_delta_ms.unwrap_or(f64::NAN)
                            );
                        }
                        report.comparisons.push(pair);
                    }
                }
                write(&o.output, &report)?;
            }
        }
    }
    println!("Report: {}", o.output.display());
    if report
        .rows
        .iter()
        .any(|r| r.error.is_some() || r.audio.as_ref().is_none_or(|a| a.error.is_some()))
        || report
            .comparisons
            .iter()
            .any(|pair| pair.unavailable_reason.is_some())
    {
        return Err(
            "One or more comparisons failed or lack sufficient coverage; inspect the report".into(),
        );
    }
    Ok(())
}
fn unavailable(job: Job, error: String) -> ResultRow {
    ResultRow {
        job,
        classes: Vec::new(),
        setup_ms: None,
        editor_open_ms: None,
        heartbeat_lateness: Distribution::default(),
        heartbeat_gap: Distribution::default(),
        missed_heartbeats: 0,
        audio: None,
        error: Some(error),
    }
}
fn print_row(row: &ResultRow) {
    let a = row.audio.as_ref();
    let d = a.map(|a| &a.successful).filter(|d| d.count > 0);
    let status = row
        .error
        .as_deref()
        .or_else(|| a.and_then(|a| a.error.as_deref()))
        .unwrap_or("ok");
    let ui = if row.heartbeat_lateness.count == 0 {
        "n/a".into()
    } else {
        format!(
            "{:.2}/{:.2}",
            row.heartbeat_lateness.p99_us / 1000.,
            row.heartbeat_lateness.max_us / 1000.
        )
    };
    println!(
        "{:10?} {:8?} {:5} {:5} {:5}/{:<5} {:8.1}/{:8.1}/{:8.1} {:6} {:>15} {}",
        row.job.mode,
        row.job.topology,
        row.job.frames,
        row.job.trial,
        d.map_or(0, |d| d.count),
        a.map_or(0, |a| a.attempted),
        d.map_or(f64::NAN, |d| d.median_us),
        d.map_or(f64::NAN, |d| d.p99_us),
        a.map_or(f64::NAN, |a| a
            .failed_call_us
            .map_or(a.successful.max_us, |failed| failed
                .max(a.successful.max_us))),
        a.map_or(0, |a| a.deadline_misses + a.warmup_deadline_misses),
        ui,
        status
    );
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    ron::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn write<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default())
        .map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "nice-isolation-bench-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_or_invalid_workloads_before_loading_code() {
        for (flag, value) in [
            ("--frames", "0"),
            ("--seconds", "NaN"),
            ("--trials", "0"),
            ("--copies", "33"),
            ("--timeout", "1"),
        ] {
            let args = vec![
                "--plugin".into(),
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                flag.into(),
                value.into(),
            ];
            assert!(options(&args).is_err(), "{flag}");
        }
    }
}
