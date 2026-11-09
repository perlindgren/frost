//! Explicitly trusted, in-process diagnostic; never an automatic scanner.
#![deny(unsafe_op_in_unsafe_fn)]
use nice_clap_host::Entry;
fn main() {
    if let Err(error) = run() {
        eprintln!("clap-probe: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() < 2 || args.len() > 3 || args[0] != "--trust-plugin" {
        return Err("Usage: clap-probe --trust-plugin <entry-path> [plugin-id]\nLoads executable native code in this process; use only a trusted CLAP plugin.".into());
    }
    let path = std::path::Path::new(&args[1])
        .canonicalize()
        .map_err(|e| e.to_string())?;
    // SAFETY: this diagnostic is deliberately an explicit native-code trust
    // boundary. Its operator must supply a trusted, conforming CLAP entry; the
    // command-line flag documents that precondition, it does not validate code.
    let entry = unsafe { Entry::load(&path)? };
    for plugin in entry.descriptors()? {
        println!("{}\t{}", plugin.id, plugin.name);
    }
    if let Some(id) = args.get(2) {
        let id = id.to_str().ok_or("Plugin ID must be UTF-8")?;
        let report = entry.probe(id, 48000.0, 256)?;
        let peak = report
            .audio
            .iter()
            .flatten()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        println!(
            "Processed {} frames: {} inputs, {} outputs, {} samples latency, peak {peak}; state supported: {}, saved bytes: {:?}",
            report.frames,
            report.inputs,
            report.outputs,
            report.latency,
            report.state_supported,
            report.state_bytes_after
        );
    }
    Ok(())
}
