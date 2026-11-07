# Review of cpal 0.18.2 and asio-sys 0.4.0 as nice-tuner's ASIO host

Reviewed 2026-11-06, in the session that replaced cpal's device enumeration on the
ASIO path with direct `asio-sys` queries.

**Recommendation: nice-tuner should own the ASIO host side of the stream in a crate
of its own, and neither dependency should be patched, forked or carried in a
`[patch]` section.** Neither crate is wrong about ASIO in general. Both are
unusable for the one thing nice-tuner needs — a stream that names the ports it
wants — and the reason is structural rather than a missing convenience.

Everything below was read in the vendored sources at the paths given, or measured
on this machine the same day. Where something is only *consistent* with the
evidence and not established, it says so.

## Artifact and scope

- `cpal` 0.18.2, `src/host/asio/` — `device.rs`, `host.rs`, `stream.rs`.
- `asio-sys` 0.4.0, `src/bindings/mod.rs` and `build.rs`. Same crate cpal pins;
  reached through `nice-common`' `asio-host` feature, which the `asio-host`
  carrier crate turns on for a Windows `standalone` build
  ([`crates/asio-host`](../crates/asio-host/Cargo.toml)).
- The ASIO SDK as fetched by `asio-sys`'s build script from
  <https://www.steinberg.net/asiosdk>; its `common/asio.cpp` and `host/asiodrivers.cpp`
  are compiled into every Windows build that enables this feature.
- Scope: capability queries, opening a duplex stream, choosing ports, and where the
  SDK comes from at build time. Not a security audit, and not a review of cpal's
  other backends, which stay as they are.

## What is established

**One loaded driver per process comes from the SDK's own host code, not from
Windows.** `common/asio.cpp` keeps a single global — `IASIO *theAsioDriver = 0;` at
line 44, and `ASIOInit` reads it with the comment `// must be loaded!` at line 85.
The non-Windows branch of the same function (lines 56-63) *deletes* whatever driver
is loaded before taking the next one, which is what settles the question: the rule
is identical on every platform the SDK covers. The layers above obey it and say so
— `asio-sys` at `mod.rs:438` ("Despite many requests from users, ASIO only supports
loading a single driver at a time") returning `DriverAlreadyExists` at `:456`, and
cpal at `device.rs:175` ("Drop the previously loaded driver before attempting to
load the next one") with the early return at `:230`.

**Port numbers cannot be expressed.** `prepare_buffer_infos(is_input, n_channels)`
at `mod.rs:1048-1057` builds the buffer descriptions as `channel_num: 0..n`. Every
public entry point funnels through it, so a caller can ask for *how many* buffers
and never *which* ones. That is the whole obstruction: on a six-input driver a
1-channel stream is bound to the driver's first input, and the way to reach the
third input is to open three channels and read the third — which cpal then refuses,
because its own device lists stop offering input at two channels.

**The public surface of `asio-sys` is larger than it looks, and the gap is
narrow.** Public: `driver_names` (`:395`), `load_driver` (`:442`),
`prepare_input_stream` (`:809`), `prepare_output_stream` (`:833`), `start` (`:860`),
`stop` (`:878`), `dispose_buffers` (`:849`), `streams()` (`:514`). Private:
`create_buffers` (`:679`), `create_streams` (`:740`), and — decisively — the
generated FFI module itself, `pub(crate) mod asio_import` at `mod.rs:1` aliased as
`ai` at `:2`. So `ASIOCreateBuffers` is unreachable, and there is no way to hand
`asio-sys` a buffer description we wrote.

An earlier reading in the same session claimed `create`, `start` and `stop` were
all private. `start` and `stop` are not; the blocker is specifically that the
buffer descriptions are constructed internally from a count and the raw module is
crate-private. Recorded here so nobody re-derives it wrongly a second time.

**Each direction must be carried across the other's creation, because ASIO
honours only the newest call.** The doc comment at `mod.rs:796-808` states it:
creating a stream "will destroy all past active buffers and recreate them. For this
reason we take the output stream if it exists." That is why `prepare_input_stream`
takes `Option<AsioStream>` and re-issues both directions in one
`ASIOCreateBuffers`. A caller that opens the two directions without carrying the
first into the second is not using the API as designed. Whether cpal does so on
its reconfigure path was left open here, and turned out not to matter: the live
failure this review was written around reproduced with no reconfigure at all, and
a channel-only change does not restart the streams — the cause was the handle
teardown above. Recorded so the next reader does not spend a session on this
paragraph's hypothesis instead.

**The guard against a second load is per handle, and the drop of a handle is a
teardown.** This is the trap in the crate, and it cost a debugging session. The
"already loaded" check reads a handle stored on the `Asio` value performing the
load (`mod.rs:445-458`), so `Asio::new().load_driver(name)` — a fresh handle,
which is what any capability query is — finds nothing stored and is *granted*.
The temporary `Driver` it returns then goes out of scope, and `Drop for
DriverInner` (`:1035`) runs `destroy_inner`: stop if running, dispose buffers,
`ASIOExit()`, `remove_current_driver()` (`:1017`, `:987-997`) — on the driver a
running stream is using, with no notification to that stream and no error
returned to the query. `DriverAlreadyExists` (`:456`) is unreachable across
handles, so nothing refuses. nice-tuner's settings page loaded a driver for
every capability question it asked; opening the page silenced live audio, which
is written up as [`BUGS.md`](../../BUGS.md) and is what the ownership guard in
`devices::asio_stream_live` now prevents.

**cpal expands an ASIO device across channel counts, and that is where the panel's
channel list comes from.** `configs_for` builds one config per rate and per count
`1..=default.channels` (`device.rs:111-125`), and `default_input_config` reports the
driver's *full* port count (`:74-81`). A six-port driver therefore presents six
input counts. nice-tuner's panel lists `1..=port_count` from that number while the
stream layer accepts only one or two channels
([`standalone/audio.rs`](../src/standalone/audio.rs) line 141), which is why
choosing channel 3 on ASIO offers a choice that cannot open.

**Rate discovery is the same question we ask.** cpal filters
`COMMON_SAMPLE_RATES` through `can_sample_rate` (`device.rs:206-210`); the direct
`asio_caps` query here mirrors that so the two paths cannot disagree.

**The SDK's build-time provenance is a chain of three fallbacks, one of which is
fragile.** `build.rs:248-291`: `CPAL_ASIO_DIR` if set — without checking that it
exists; else `<temp>/asio_sdk` if already unpacked; else download and unpack into
temp. The download interpolates `-OutFile <path>` **unquoted** into a PowerShell
command (`build.rs:303-311`), so a temp path under a user name containing a space
splits and dies looking like a missing SDK. Measured both ways on one machine and
recorded in [`TODO.md`](../../TODO.md) lines 356-372; the repository's refusal to
set the variable is argued in [`.cargo/config.toml`](../../.cargo/config.toml)
lines 12-51.

**What a fresh clone gets.** `default = []`, so `cargo build -p nice-tuner` touches
none of this, and on Linux and macOS `--features standalone` still pulls cpal
without its `asio` feature: the carrier dependency is declared under
`cfg(windows)`, so `asio-sys` cannot enter those graphs at all. On Windows
`--features standalone` does bring ASIO, so a Windows standalone build on a
machine with neither the SDK nor a working download fails instead of shipping
silently without ASIO — the deliberate trade — and only that build can meet the
download failure. Non-Windows targets are exempt at the source too:
`build.rs:27-33` emits stub bindings for any target that is not Windows.

## The cheap experiment that might be enough, and is unproven

`AsioStream.buffer_infos` is public, `AsioBufferInfo.channel_num` is a public field,
and the carried side of `prepare_input_stream` / `prepare_output_stream` is fed
straight into the single `ASIOCreateBuffers`. So a port could in principle be chosen
by creating the direction we care about *first*, rewriting the `channel_num` of its
infos to the port we want, and then calling the prepare function of the *other*
direction with the mutated stream carried in — the carried infos are used as given,
while the direction created last always gets `0..n`.

For nice-tuner that sequence lands exactly on the wanted shape: input on a chosen
port, stereo output on 0 and 1, which the count-based path produces for free.
**Unproven, in this order:** whether real drivers accept non-contiguous
`channel_num` values, whether `asio-sys`'s own state bookkeeping survives infos it
did not create, and whether the buffer pointers the driver writes back land where we
expect. This is the experiment to run before any FFI is written, and it needs a
driver with more than two inputs attached and someone able to put signal on input 3.

## Options for the durable fix

**Send the API upstream.** `asio-sys` needs buffer descriptions taken as a
parameter instead of a count — `prepare_buffer_infos` gains a channel-list variant,
and the two prepare functions get explicit-port siblings. Given how narrow the gap
is above, that is a small patch in the right place, and it leaves nice-tuner
depending on a public API. It waits on a release, and this repository does not
carry forks or `[patch]` entries, so it cannot be held locally in the meantime.

**Own it in a first-party crate** (`frost-asio` beside the four existing
`nice-tuner/crates/*` members): SDK discovery and compilation, driver load, one
`ASIOCreateBuffers` carrying both directions with the ports we chose, our own
`buffer_switch`, the sample-type conversions for whatever format the driver
reports, the caps queries currently done through `asio-sys`, and a `SAFETY.md`
like the other unsafe members carry. This is the only option that closes
three things at once — port selection, the SDK's build-time provenance under our
own control, and exactly one owner of the driver in the process. Its cost is
honest and not small: several hundred lines of unsafe FFI, `ASIOOutputReady`,
real-time callback rules, and `devices.rs` losing the habit of mapping `"asio"`
onto a cpal `HostId`, since enabling this feature would mean cpal's ASIO host is
compiled out.

The requirement to design against, stated concretely because "full ASIO
capability" is not a specification: *a guitar through a preamp on input 3, tuned
by nice-tuner, with the stream holding that one input port.* Under cpal that
costs a three-port capture with two ports read and discarded, and nice-tuner
does not offer it today — `standalone::audio` accepts input widths of one or two
only, and the plugin's IO layouts declare one and two, so a three-port capture
matches no layout. Port-exact binding is what removes the workaround rather than
widening it, for a reason worth verifying rather than assuming: a driver may
change its own behaviour when extra ports join — phantom and preamp routing are
where interfaces are known to differ — so "open 0..3 and ignore two of them" is
not free on hardware, though it is free on paper and unmeasured here. Until the
crate exists, the panel is honest about the limit rather than the hardware's:
channels beyond two are offered and refused at APPLY.

**Rejected, with the reason each fails.** A first-party crate whose only job is to
fetch the SDK: cargo orders builds by dependency edges and gives no edge from our
crate to `asio-sys`'s build script, so the two scripts race — a build that passes
when the fetch wins and fails when it does not. A `build.rs` in nice-tuner doing the
same: dependency build scripts run *before* it, and a build script spawning cargo
deadlocks on the target-directory lock, so at best it fixes the second build.
`[env] CPAL_ASIO_DIR` in `.cargo/config.toml`: it does reach dependency scripts, but
it selects step one without checking the path exists, which is the mistake the
repository already made and deleted, and nothing nice-tuner writes can improve the
resulting error because `asio-sys` fails first. A `cargo build-asio` alias into
`xtask` that ensures the SDK and then spawns cargo with the variable in its own
environment: sound, and worth having as an entry point — but an alias is invoked,
not triggered, so it makes the path comfortable rather than automatic.

## Related records

[`BUGS.md`](../../BUGS.md) carries the live reconfigure defect this review turned
up; [`TODO.md`](../../TODO.md) carries the unscheduled work and the upstream
quoting fix; [`.cargo/config.toml`](../../.cargo/config.toml) lines 12-51 holds the
decision not to configure the SDK; and the user-visible behaviour, with the numbers
measured on this machine, is in the **Windows + ASIO** section of
[`README.md`](../README.md).
