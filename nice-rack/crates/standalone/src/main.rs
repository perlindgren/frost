//! Executable trust boundary; the nice-rack application library denies unsafe.
#![deny(unsafe_op_in_unsafe_fn)]
// A release download gets double-clicked, and a console-subsystem image makes Windows
// allocate a console window for it before `main` runs. That console is then the first
// window an operator sees, and it stays empty for the whole of start-up — settings,
// every saved CLAP entry, then the wgpu adapter and device — because this process
// creates its first real window only after all of that, in `frost::run_configured`.
// Saying GUI subsystem takes the console away, so the rack window is the first window
// there is.
//
// What that costs is stated rather than hidden: the report below, and any panic, no
// longer reaches a screen when the binary is double-clicked. Started from a terminal
// the process still inherits that terminal's console and prints as before, and that is
// the only place `--list-devices` is ever typed. The application's own helper spawn
// was never the blank window: it passes `CREATE_NO_WINDOW`
// (`nice_common::devices::snapshot_via_child`).
#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--list-devices") {
        let snapshot = nice_common::devices::snapshot(
            // A listing typed by hand names no backend; the parent half of
            // this protocol always does. The fallback is the platform's own
            // path, spelled out on the constant.
            args.get(2)
                .filter(|s| !s.is_empty())
                .map(String::as_str)
                .unwrap_or(nice_common::devices::LIST_BACKEND_WHEN_UNNAMED),
            args.get(3).filter(|s| !s.is_empty()).map(String::as_str),
            args.get(4).filter(|s| !s.is_empty()).map(String::as_str),
        );
        match snapshot.to_ron() {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    // One graphics API for the whole process, named before anything that could
    // pick another. Every plugin editor the operator opens draws on the thread
    // that owns the rack window, and a plugin whose device lives on a different
    // API than the rack's costs that thread a GPU context switch once a frame
    // per window: two editors, three APIs, and a window stops tracking the
    // mouse. `pin_gpu_backend` writes the rack's own pick where even a plugin we
    // do not ship will read it. It happens here, first, because it writes the
    // environment and no other thread may be reading it, and because the scan
    // for plugins that `run` performs would otherwise get its say first.
    frost::pin_gpu_backend();
    if let Err(error) = nice_rack::run(|path| {
        // SAFETY: dropping a CLAP entry into this explicitly in-process host
        // selects native code to execute. The operator must trust that entry
        // and its CLAP contract; paths/metadata cannot establish this property.
        // This is the same explicit executable boundary as clap-probe.
        unsafe { nice_clap_host::Entry::load(path) }
    }) {
        eprintln!("nice-rack: {error}");
        std::process::exit(1);
    }
}
