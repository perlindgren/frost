# Instructions for all coding agents

These rules apply to the entire frost repository and all Cargo workspace members.

## Dependencies — mandatory rule

- NEVER modify third-party dependency source, including vendored copies,
  registry caches, or checked-out dependencies.
- Do not patch, fork, replace, or change dependency versions.
- Enabling existing dependency features in the application's Cargo.toml is
  allowed. This does not authorize editing the dependency itself.
- Implement changes in project-owned code using public dependency APIs.
- If those APIs cannot support the task, explain the limitation. Do not work
  around it by altering a dependency.
- Existing local dependency modifications do not grant permission to make
  further dependency changes.

## Performance-critical builds — mandatory rule

- Compile performance-critical tests and applications with `--release`:
  audio callbacks and devices (including `niceroute.dll`), benchmarks,
  latency/cadence/clock measurements, and any run used to judge how
  something sounds or how fast it is.
- A debug artifact is not a measurement and not a valid reproduction. No
  inlining, per-frame bounds checks and `assert!`/`debug_assert!` in
  per-block paths make it several times slower, so a defect seen there
  may exist only there, and one that real-time margins hide in release
  surfaces there as dropouts, clicks or distortion.
- Build first, then measure: `cargo build --release -p <crate>`,
  `cargo test --release -p <crate>`, `cargo bench`. Never report a number
  from `target/debug` as if it came from an optimized build.
- A host loads one fixed, registered file, so install the optimized build
  where the registration points (`.workspace\install-niceroute.ps1`
  installs `NiceRack ASIO (x64)`) instead of letting a later plain
  `cargo build` leave a debug DLL on that path.
