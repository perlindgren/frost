# FROST

A game engine experiment by N65 Game Research Station (N65 GRS).

Why?

- Primarily to gain experience of local hosted AI workflows.
- Sufficiently complex problem to challenge ability to reason on realistic use-cases.
- To gain human experience on inner workings of a games engine, starting from very limited, surface level understanding.
- If successful to provide the core mechanics of a library based games engine, following the Rust paradigms (unlike e.g., Bevy, that introduces an additional ECS abstraction to offer shared mutability.)

See [SETUP.md](SETUP.md) for AI based workflow.

## Examples

Run the examples with:

```shell
RUST_LOG=info cargo run --example gizmos
```

or under Win11 with Powershell:

```powershell
$env:RUST_LOG="info"; cargo run --example gizmos
```

## Repository hooks

The versioned hooks in `.githooks/` gate commits (fmt + clippy) and pushes
(tests, plus a `wasm32-unknown-unknown` check of the web target — every other
gate compiles the host target only, so a `cfg`-gated import in the backend
walks through all of them and breaks `web/build.sh` later). Every push runs
every check, `main` included, and a clone missing the wasm target is refused
rather than warned, with the remedy printed beside the refusal:

```shell
rustup target add wasm32-unknown-unknown
```

The one exception is a push that ships nothing: deleting a ref carries no commits,
so no tests run for it, and the hook says so on one line rather than skipping in
silence. A push that mixes deletions with commits runs everything.

There is CI behind this repository now, and it is the reason the hooks stopped
being the last line. `.github/workflows/gate.yml` runs the same rules on a
runner — the dropper scan, `fmt`, clippy over every target, the packaging
assertions, the web build, the test suite — and the `main-gate` ruleset requires
that check before `main` may move. That is precisely what a hook cannot be: the
hooks live in per-clone config and `--no-verify` walks past them, while the
server refuses whatever it has not seen pass. Refusing is not theoretical:

```text
remote: error: GH013: Repository rule violations found for refs/heads/main.
remote: - Required status check "gate" is failing.
! [remote rejected] ci-refusal-probe -> main (push declined due to repository rule violations)
```

which is what pushing a commit whose gate had failed printed, verbatim. The
consequence for anyone working here: `main` takes a commit only after that commit
passed on another ref, so work arrives by branch.

Two things the CI does not do are worth knowing before relying on it. It runs on
Linux only, so the Windows code path — the platform this engine is developed and
shipped on — is compiled only on a machine like yours. And the GPU test tree
reports without judging: a runner with no framebuffer cannot ground a pixel
assertion, and a measurement that cannot be taken must refuse rather than come
back green (`docs/benchmarking.md` §37).

Git runs the hooks only once this clone points at them — the trigger is a
per-clone setting, never cloned itself. After every fresh clone, once:

```shell
sh .githooks/install.sh
```

(or clone with `git clone --config core.hooksPath=.githooks <url>` and skip
the step). On Windows from PowerShell or cmd, where `sh` is usually not on
the PATH, the script's whole job is the one command:
`git config core.hooksPath .githooks` — run it from any shell (from Git
Bash, `sh .githooks/install.sh` works as everywhere). They are a gate you
opt into, not a wall: `--no-verify` skips them by design. What it skips is the
local half — a commit that skips its hooks still has to pass `gate` somewhere
else before `main` will accept it.

## License

The engine is free: `frost`, `frost_shaders` and the tools under `tools/` are
MIT OR Apache-2.0, the same terms as the Rust project itself. The full texts are
in [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE), and
[assets/fonts/OFL.txt](assets/fonts/OFL.txt) carries the SIL Open Font License
under which Fira Code is redistributed.

What is built *with* the engine is not free in the same way. `antifrost/` and
`examples/immortal/` are the games, and they are under no licence: all rights
reserved, deliberately not inheriting the workspace terms, so "the engine is
open" and "a game built on it is open" are separate decisions. The assets under
`assets/` follow the same split — the handful the engine itself needs travel
with it, the rest belong to the games.

[docs/licensing.md](docs/licensing.md) is the ledger: which package carries
which terms, which assets the engine shares with the games, and what is still
unresolved — provenance for three fonts and every sound file, the partition of
the asset tree, and the packaging a real `cargo publish` would want.