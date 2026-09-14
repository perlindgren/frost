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
