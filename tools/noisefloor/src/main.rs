//! Ask the machine how loud it is, and let the answer gate a measurement.
//!
//! ```text
//! cargo run --release -p noisefloor -- --seconds 10
//! cargo run --release -p noisefloor && cargo test --release --lib draw_cost -- --ignored --nocapture
//! ```
//!
//! Exit code 0 means the floor was quiet enough to judge by, 1 means it was
//! not, 2 means the arguments made no sense — so `noisefloor && <bench>` reads
//! as it looks, and a campaign can refuse to start on a machine that is busy
//! being something else. The thresholds live in the library, beside the
//! measurements that set them; this program reports against them instead of
//! owning them, so a bench and this tool can never disagree about what "loud"
//! meant today.

use std::process::ExitCode;
use std::time::Duration;

/// How long to watch. That is the whole interface, and it is parsed by hand
/// rather than by a dependency: this is the instrument, and an instrument
/// should not have a supply chain.
struct Args {
    seconds: f64,
}

const USAGE: &str = "usage: noisefloor [--seconds N]\n\
                    the floor is how late a wake landed and how long a fixed \
                    quantum of arithmetic took; the thresholds it is judged \
                    against are in tools/noisefloor/src/lib.rs";

fn parse_args() -> Result<Args, String> {
    let mut args = Args { seconds: 5.0 };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--seconds" => {
                let value = words.next().ok_or("--seconds wants a value")?;
                args.seconds = value
                    .parse()
                    .map_err(|err: std::num::ParseFloatError| format!("not a number: {err}"))?;
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    if !args.seconds.is_finite() || args.seconds <= 0.0 {
        return Err("--seconds wants a positive number of seconds".to_string());
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("noisefloor: {err}");
            return ExitCode::from(2);
        }
    };
    println!("noisefloor: watching for {} s", args.seconds.trunc());
    let canary = noisefloor::Canary::start();
    std::thread::sleep(Duration::from_secs_f64(args.seconds));
    let floor = canary.report();
    println!("noisefloor: {floor}");
    if !floor.too_loud() {
        println!("noisefloor: quiet enough to judge");
        return ExitCode::SUCCESS;
    }
    // Say which reading tripped, because they are different things to do
    // something about: a late wake is something else running, a stretched
    // quantum is a core that is not really there.
    let mut why = String::new();
    if floor.wake_loud() {
        why.push_str(&format!(
            "wake p99 {:.2} ms past the {:.1} ms limit",
            floor.p99_ms,
            noisefloor::WAKE_EXCESS_LIMIT_MS
        ));
    }
    if floor.burn_loud() {
        if !why.is_empty() {
            why.push_str(", and ");
        }
        why.push_str(&format!(
            "burn p99 {:.0} µs is {:.1}× its own median, past the {:.1}× limit",
            floor.burn_p99_us,
            floor.burn_spread(),
            noisefloor::BURN_SPREAD
        ));
    }
    println!("noisefloor: too loud to judge — {why}");
    ExitCode::from(1)
}
