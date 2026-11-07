//! Windows-only carrier of CPAL's ASIO host, switched on by `standalone`.
//!
//! This crate contains no code. It exists because of one cargo rule: a
//! feature's implications are identical on every platform, so `standalone`
//! cannot mean "and ASIO" here but not there. A *dependency* can be scoped to
//! a platform, so the implication names a dependency instead — declared under
//! `[target.'cfg(windows)'.dependencies]`, optional, and pulled in by
//! `standalone`. Enabling this crate enables `nice-common/asio-host`, which
//! in turn enables `cpal/asio` and `asio-sys` on the unified graph, so the
//! consuming crate's own `cpal` gains the ASIO backend without saying so.
//!
//! The four graphs this produces, each measured with
//! `cargo tree -e features --target …`:
//!
//! ```text
//! windows, no features        → this crate absent; a plugin cdylib with no
//!                               device code and no need for the ASIO SDK
//! windows, `standalone`       → asio-host → nice-common/asio-host → devices
//! linux,   `standalone`       → absent, and cargo does not object to a feature
//!                               naming a dependency that target lacks
//! this crate as a member      → an empty crate; its only dependency is declared
//!                               under cfg(windows) too, so `cargo test
//!                               --workspace` on Linux never reaches asio-sys
//! ```
//!
//! The name is deliberate: an optional dependency creates an implicit feature
//! of the same name, so `cfg(feature = "asio-host")` at the call sites means
//! exactly what it meant when `asio-host` was a hand-written feature alias, and
//! those sites did not change. Building it needs the Steinberg ASIO SDK, whose
//! licence is proprietary and which stays outside the tree; `.cargo/config.toml`
//! explains why no `CPAL_ASIO_DIR` is set there.
