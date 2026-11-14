//! Windows-only carrier of the ASIO transport, switched on by `standalone`.
//!
//! This crate contains no code. It exists because of one cargo rule: a
//! feature's implications are identical on every platform, so `standalone`
//! cannot mean "and ASIO" here but not there. A *dependency* can be scoped to
//! a platform, so the implication names a dependency instead — declared under
//! `[target.'cfg(windows)'.dependencies]` (`Cargo.toml:12`), optional, and
//! pulled in by `standalone`. Enabling this crate enables
//! `nice-common/asio-host`, which is the `devices` driver queries and
//! `nice_common::asio_io` built on `azo`: no cpal involvement, because a
//! driver carries both directions itself.
//!
//! The four graphs this produces, each measured on this tree with
//! `cargo tree -e features -p nice-tuner --features standalone`, the same
//! command with no features and with
//! `--target x86_64-unknown-linux-gnu`, and `-p asio-host` in place of `-p`:
//!
//! ```text
//! windows, `standalone`  → asio-host → nice-common/asio-host → devices,
//!                                      azo (feature "host") → azo-sys
//! windows, no features   → this crate absent, and so is every driver query:
//!                          a plugin cdylib with no device code in it
//! linux,   `standalone`  → absent, cpal and jack present instead, and cargo
//!                          does not object to a feature naming a dependency
//!                          that target lacks
//! this crate as a member → an empty crate on any platform whose target the
//!                          one dependency below excludes, so `cargo test
//!                          --workspace` on Linux has no ASIO code to reach
//! ```
//!
//! Nothing here needs an SDK, which is the reason this crate's existence is
//! worth restating at all. azo-sys declares the ASIO COM interfaces by hand —
//! one `lib.rs` and one `utils.rs`, no build script, no C++ — so there is no
//! Steinberg tree to obtain, no `CPAL_ASIO_DIR` to point at one, and no reason
//! left for a build of this feature set to be refused on a machine that has
//! never heard of ASIO. While the host was cpal's, that was the case: the SDK
//! was a licence and a C++ compile, and both the gate and the hooks left
//! `--features standalone` out on Windows because of it.
//!
//! The name is deliberate: an optional dependency creates an implicit feature
//! of the same name, so `cfg(feature = "asio-host")` at the call sites means
//! exactly what it meant when `asio-host` was a hand-written feature alias, and
//! those sites did not change.
