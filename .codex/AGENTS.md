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

## Know the machine before you gate on it

A build failure that names a CPU capability is data about a *compile flag*
until it has been shown to be data about the silicon. Read the code that
printed the message, and read the machine, before either is reported.

- Establish the host before choosing gates or tests: `uname -m`,
  `grep -m1 'model name' /proc/cpuinfo`, that line's `flags`, `nproc`,
  `rustc -vV`, `rustup target list --installed`. This tree is built for more
  than one architecture — the host target, `wasm32-unknown-unknown`, and
  riscv/thumb embedded targets — so the answer changes which legs are even
  meaningful.
- Keep three questions apart, because a refusal rarely says which it means:
  what the CPU *has* (CPUID, the `/proc/cpuinfo` flags), what rustc was
  *told to emit* (`CARGO_CFG_TARGET_FEATURE`; a default `x86_64` target
  answers `fxsr,sse,sse2` on a machine whose CPU has AVX2 and AVX-512), and
  what the artifact was *built with*.
- A refusal on target features is a flag, not a limitation.
  `RUSTFLAGS="-Ctarget-cpu=x86-64-v3"` is what `nice-nam/README.md` tells a
  Linux builder and what the `nice-nam` steps in `.github/workflows/gate.yml`
  set. Keep it out of `.cargo/config.toml`: that raises the instruction-set
  floor for every member, `antifrost` included, which is a release decision.
- Gate against what the change can reach, and say which legs were not run and
  why. A leg that was skipped is not a leg that passed, and a leg that failed
  for a reason outside this repository is worth naming as such — with the
  evidence, after the two reads above.
- `.githooks/pre-commit` asks for the `nice-nam` graph without that prefix, so
  on Linux a commit through the hooks needs `RUSTFLAGS` exported, or
  `--no-verify`. The hooks document that way out themselves, and the runner's
  `gate` is unconditional; state plainly which one was used.

## How to talk to the human

- Write in English.
- Short, and to the point: the answer first, then only the evidence that
  carries it. No preamble, no restating the question, no recap of work the
  human has already seen.
- Prefer plain sentences over headings when three lines would do.
- When you need something from the human, ask for one thing, and write it as
  steps they can follow in order: what to run, where, what to paste back.
  Number them if there is more than one.
- Close with the three facts that matter — what you did, what you did not do,
  what you need — in that order.
