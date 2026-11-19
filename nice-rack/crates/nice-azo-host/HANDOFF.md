# Handoff — nice-azo-host is live in REAPER

Everything the design promised now works end to end, verified silently (no
audio was ever emitted; REAPER's UI was never touched). Registration is done,
and REAPER 7.78 opens the device, hosts Komplete through it, starts cleanly
and plays. Getting from "device refuses to start" to *correct* sound took
longer than it looked from here: an ASIO return code (bug 5) and the master
reading the zero frontier for its own DAC (bug 6) got sound out of the
speakers, and the listening test then found that sound to be wrong — the
shape of the samples themselves (bug 7), with a loop that outlived its host
(bug 8) and a real-time claim made by the wrong thread (bug 9) behind it.
Bug 7 is what "still the same messy sound" was, and the report that it
looked byte-order related was correct.

## Status — all green

| check | result |
|---|---|
| `cargo test --release -p nice-azo-host` | 11/11 (probes and the DLL under test are release builds — see "Real-time"); 4 of the 11 are the sample-shape conversions of bug 7, 2 are `fit_size` (see "The clock, and who names its size") |
| `cargo test -p nice-common` | all pass |
| `cargo test -p nice-rack` | 33 pass / 5 ignored (unchanged; copy card intact) |
| `cargo clippy -p nice-azo-host` | zero warnings, zero errors |
| `niceroute-probe loopback 3` | **PASS** — 1124/1125 and 1125/1125 callbacks in 3 s (= full 48 000/128 cadence), B hears A's 440 Hz at RMS 0.1767 (theory 0.1768), peak 0.2500, crosstalk 0.0, and **A's own inputs measure exact 0.0** (nobody hears themselves — see bug 4) |
| `niceroute-probe cocreate` | **PASS** — full host path: `CoCreateInstance(CLSID)` resolves via the registry, loads the DLL, answers 8 in / 8 out at 48 kHz |
| `niceroute-probe solo 3` | **PASS** — one instance, real Komplete hosted; latest run `solo 8 --tone 440` = 3001/3002 callbacks in 8 s, driver note "hosting 'Komplete Audio ASIO Driver' at 48000 Hz, block 128" — that run predates the follow-the-host rule below, when the floor was what spoke; re-run it after the next install and expect the size the probe asked for ("The clock, and who names its size") |
| `verify-hwout.ps1 6 440` | **PASS** — `master: hw-out peak 0.250000 over 256 callbacks at block 128`, repeated across the run: the mix demonstrably lands in the real driver's output buffers, muted so nothing was heard. Baseline with the bug-6 read restored (`out_g = self.g`) logs `0.000000` and **FAIL**s — the FAIL→PASS pair for the leg loopback cannot hear |
| `Engine::witness` on that same run | clean: `0 repeated blocks, 0 dropped, 0 stalls` in every two-second window (the two counters that used to cry wolf are described in "Real-time") |
| `nice-rack.exe --audio-probe 5 0` | silent regression green: 3751/3751 live blocks, copy card copying, 0 overloads |

The loopback selftest is the proof of the whole concept: two independent
processes each `LoadLibrary` niceroute.dll, both become clients, one becomes
master, a sine written by A's `bufferSwitch` arrives on B's input with exact
level — through the shared-memory bus, no hardware, no audio device opened.

**What that proof does *not* cover** is the last leg: client → bus → master →
the *real driver's* output buffers → DAC. No in-process test can hear a
speaker, and bug 6 lived precisely there. The pin in "Measuring the leg
nothing can hear" is the answer, and it is now part of the battery.

## What changed since "first build crashes / silent" (the real bugs)

1. **Factory bug** (`abi.rs`): `f_create_instance` QI'd through the factory,
   which only knows IUnknown/IClassFactory — every host's
   `CreateInstance(CLSID)` would have gotten E_NOINTERFACE. Now QIs the
   object itself.
2. **The silent bus** (`master.rs` step pass 2): the mix consumed a client
   block only if its frames ended *before* the clock edge — but a healthy
   lockstep client publishes block *n* exactly as the clock passes its last
   frame, so *every* block was classified stale and discarded. Fixed with a
   `BACKLOG_FRAMES` (¼ of the mix scratch) history window, plus one block of
   deliberate cross-client delay: an input block is published only once every
   *other* slot's output has been summed through its first frame (gate:
   `others_mixed_to`). Pre-adoption frames are frontier zeroes, which also
   breaks the simultaneous-first-block deadlock at adoption, and a stranded
   slot stops being waited on after the backlog window instead of freezing
   the bus. Rationale is in `design.md` → "Clocking and mix order".
3. **Cadence at 1/6 speed**: `Sleep` runs on the 15.6 ms scheduler tick, which
   ate the 2.67 ms pacing period whole. `win::Timer` is a high-resolution
   waitable timer (plain timer + `timeBeginPeriod(1)` fallback), and the
   virtual clock now paces on absolute deadlines, rebasing after a miss
   instead of bursting.
4. **A client heard itself** (found by review before go-live, fixed same
   evening): the publish pass copied the global mix straight onto every
   slot's virtual inputs, so each client received its own output back — a
   REAPER monitoring the bus with anything live would have fed back. The
   consume pass now snapshots each slot's newest summed block (`own_last`)
   and publish subtracts it back out where the frames line up, so the mix
   each client hears is *everybody else*, exactly. The read is snapshot-not-
   ring-cell on purpose (SPSC: the client may be writing the cell right
   now), and the stale/adoption windows (`mixed_from`) make skipped,
   un-summed blocks exempt from subtraction. The loopback now pins this:
   A also measures its own input 1 and must read exact zero while B
   receives A's tone whole — verified failing at full level (peak 0.25)
   with the subtraction disabled, PASS with it in.
 5. **"Error starting ASIO" in REAPER, from a driver that started fine**
   (`abi.rs`): `v_start` returned `ASE_SUCCESS` (`0x3F4847A0`) on success.
   Both `ASE_OK` (0) and `ASE_SUCCESS` mean success in the SDK, and azo's
   own `ResultCode::ok` accepts either — but hosts overwhelmingly write
   `if (start()) error(...)`, so a huge positive number *is* a failure to
   REAPER. `start`/`stop`/`disposeBuffers` now return `ASE_OK`, and
   `ASIO_SUCCESS` is deleted from the crate so it cannot come back. A
   working device earns "Error starting ASIO" this way with nothing wrong
   inside it — hence the trace log below. While in there:
   `kAsioSelectorSupported` is `0x11`, not `0` (the old code answered
   selector 0, which no host asks for), and unknown selectors now answer
   `ASE_NotPresent` *and* leave a trace line with the selector in hex.
 6. **No sound out of REAPER while playing** (`master.rs`,
   `hardware_tick`): the master read the mix for the DAC at `[g, g+block)`
   — from the clock edge forward — but a client can only publish block *m*
   after the master released that block's inputs, so at edge `g` the newest
   frames in the mix are `[g-block, g)` and everything at or past `g` is
   the zero frontier the frontier pass armed on purpose. The hardware
   output was reading permanent silence while the bus itself was healthy —
   loopback proved the *client* side precisely because the publish pass
   reads one block back. Both directions now read the same window: the
   hardware takes `[g-block, g)`, and the hardware *input* publish window
   moved to the same `f0 - block` slice so a mic and the bus mix arrive on
   one callback in phase instead of one block apart. Cost: one block of
   output delay, which is what clients already pay; `getLatencies` now
   answers `2 * block` both ways to say so honestly.
 7. **The samples were the wrong shape** (`master.rs`, `HwFmt`): after
   bug 6 there was sound, and it was wrong in a way nothing inside the bus
   could see. `createBuffers` hands back `void**` and no format — the
   shape of what lives in those buffers is whatever each channel answered
   from `getChannelInfo`, and the Komplete Audio driver answers
   `PCM_I32_LSB` (`0x12`) on all four of its channels. `open_hardware` now
   asks, and logs the answer per channel (`master: hw out 0 is 'Output 1',
   type PCM_I32_LSB (0x00000012)`) before a single byte is trusted. We
   were writing native `f32` into int32 buffers. `0.25f32` is
   `0x3E800000`, read as an integer that is 1 048 576 000 — a quarter of
   the 2³² span either side of zero, which is 0.49 of full-scale
   *amplitude*, so this was never a gross level error: about 6 dB of one,
   riding on a waveform that had become a ramp of exponents jumping on
   every sign change, because most of the bits of a float say **how big**
   the sample is rather than what it is. Silence survived it, because zero
   is the one value both shapes agree on, which is why the device sounded
   merely wrong instead of refusing. And the pin could not see it either:
   `store_frames` reports the peak of the *float* frames it was handed, so
   `NICEASIO_SILENT_HW` went on answering `hw-out peak 0.250000` about
   buffers that no longer said 0.25 anywhere. The corruption sat one
   `memcpy` past the last place anything was being measured, which is the
   whole lesson. The report that it looked byte-order related was correct.
   The fix is per channel, not global: `Hw { in_fmt, out_fmt }` of
   `HwFmt::{F32, I32}`, converted on both legs by `load_frames` in,
   `store_frames` out (it hands back the peak, which is what
   `NICEASIO_SILENT_HW` reports) and `silence_frames` for either, since
   `HW_SAMPLE_BYTES` is 4 both ways. An unknown code is a driver to
   refuse rather than a format to guess at — `buffers are not 32-bit
   float or int: …` — and so is a driver that will not answer
   `getChannelInfo`. Two details worth keeping: int32 full scale is 2³¹,
   and `i32::MAX as f32` *rounds* to `2147483648.0`, so `+1.0` lands one
   count above the top of the range while `−1.0` lands exactly on the
   bottom — `store_frames` clamps both rails instead of trusting an
   `as i32` cast to saturate, or a full-scale square wave comes out loud
   on one half-cycle and one count short on the other. Nothing changed
   for our own hosts: `abi.rs` still answers `PCM_F32_LSB`, because the
   bus *is* float. Four tests pin it —
   `int32_is_not_float32_wearing_a_hat` (a quarter of full scale is
   `536870912`, asserted **unequal** to `f32::to_bits(0.25)`, plus a
   round trip), `over_doubles_saturate` (`[4.0, -9.0, -0.0, 1.0]` →
   `[i32::MAX, i32::MIN, 0, i32::MAX]`, because `2.0f32 as i32` is zero
   in Rust and a zero mid-waveform is a click),
   `float32_passes_through_exactly`, `silence_is_the_same_bytes_either_way`.
 8. **The sound kept looping after the host stopped** (report: "when
   stopping reaper, the sound continues looping"; `master.rs`, the parked
   hardware exit path): the master called `stop()` with whatever the last
   callback had written still in the driver's buffers, and a driver is
   free to keep cycling its own DMA ring after the host walks away — the
   room then hears the last loud block over and over. Both halves of
   every output buffer are zeroed and one period is slept before
   `handle.stop()`, so it stops on silence. `ENGINE` is nulled *before*
   `dispose_buffers()`, so a driver that calls back late finds a defined
   no-op instead of freed memory — the same reason the *first* callback
   writes silence rather than copying frames `[0, block)` into both
   halves, which was a click and a repeat besides.
 9. **The real-time claim was made by the wrong thread** (`master.rs`,
   `Engine::rt`): "the pump and the master thread register with MMCSS"
   was true and useless in hardware mode, because in hardware mode the
   master thread *parks* — the thread feeding the DAC is the real
   driver's `bufferSwitch` thread, which had never asked for anything. It
   now registers `"Pro Audio"` lazily on its first callback and logs
   `master: callback thread priority …`. Dressing the spectator and
   leaving the performer in the crowd is worth remembering: whoever
   answers the deadline is who has to hold the priority.

## Measuring the leg nothing can hear

Bug 6's leg ends at a DAC, so a test either plays sound out loud (not
acceptable) or cannot see it at all. Two pieces close that hole:

* `NICEASIO_SILENT_HW=1` (master): measure the samples about to go to the
  real driver, append `master: hw-out peak 0.250000 over N callbacks at
  block B` to the trace log several times a second, and send **silence** to
  the hardware. Nothing else about the bus changes; only the last copy.
* `niceroute-probe solo [SECONDS] [--tone HZ] [--block N]`: opens the device
  once, for real, hosting `RealDriver` — the REAPER shape. `--tone` puts a
  0.25 sine on output 1 and switches that pin on itself, so the run stays
  inaudible by construction. `--block` picks the *client's* block size,
  which is the knob for "does a host clocked faster than the bus still get
  every callback" (`--block 64` against a 128-frame bus: 5998 of 6000).
* `niceroute-probe hwblock [FRAMES]` reads and writes `HwBlock` — the bus
  clock, not the client's block. Writing needs a shell that may write HKCU;
  where it cannot (any sandboxed run: `hwblock: Access is denied. (os error
  5)`), `NICEASIO_HW_BLOCK=FRAMES` outranks the registry for one run and
  `hwblock` prints it as `(from NICEASIO_HW_BLOCK)` so a measurement cannot
  be read back as a registry setting that was never made.

`.workspace\verify-hwout.ps1 [seconds] [toneHz]` runs it, greps the log and
FAILs below 0.2. Run on 2026-11-17 evening: with the fix the peak reads
`0.250000` for every window of a 6 s run; with the frontier read put back
(`out_g = self.g`) the same run reads `0.000000`. That is the FAIL→PASS pair
for bug 6, measured rather than listened for.

Note the script runs the probe from a shell, so its trace lines land in the
*shell's* temporary directory, not the one REAPER writes to.

## Real-time: the sound arrived, and it was ruined

The first listening test after bug 6 settled one thing and upset the
other: there was sound, and it was distorted. Two causes showed themselves
immediately, and neither was the bus:

* **The device under test was a debug build.** Every measurement up to
  that point came from `target\debug\niceroute.dll` — no inlining, a
  bounds check per frame, `assert!` in the per-block path — on a 64-frame
  callback, with the host's entire graph running on our side of the ABI.
  `.codex/AGENTS.md` now forbids judging performance or *sound* from a
  debug artifact, and `.workspace\install-niceroute.ps1` copies
  `target\release\niceroute.dll` to whatever path the registration names
  (refusing politely when a host has it mapped), so a later plain
  `cargo build` cannot leave a debug device where REAPER reads it.
* **Not one of our threads had real-time privileges.** The host's
  `bufferSwitch` is called from our pump thread, and that thread was an
  ordinary normal-priority one: a disk flush or a browser repaint wins
  every argument against a 1.3 ms deadline, and a block that misses it is
  dropped — which is exactly what distortion is made of. The pump and the
  master thread now register with MMCSS (`AvSetMmThreadCharacteristicsW`
  with `"Pro Audio"`, then `AvSetMmThreadPriority(HIGH)`), falling back to
  `THREAD_PRIORITY_TIME_CRITICAL` if the service refuses, and the trace
  log records which was granted:
  `start: pump on slot 0, priority MMCSS 'Pro Audio'`. In hardware mode
  that turned out to be the wrong thread too — see bug 9.

**And it was still messy after both of those.** The meters inside the bus
were still clean, which is what finally made the format question
interesting rather than exotic: bug 7. Nothing in the system measured the
bytes the driver was given — `NICEASIO_SILENT_HW` measures the float frames
it is about to convert, so it answered `0.250000` whether the shape on the
far side of the copy was right or not.

Whether the bus keeps up is no longer a matter of opinion:
`Engine::witness` costs a few counter reads per callback, says nothing
while the bus is healthy, and logs one line every two seconds when it is
not (release build, `solo --tone`, no host but the probe):

```text
master: 751 ticks, work max 91 us of 2666 us period, 4 late/0 dry (worst 4050 us), 0 repeated blocks, 0 dropped, 0 stalls (worst 0 blocks deep), hw-in peak 0.029041
```

That reads as: the master's own work is 3 % of the period; the `late`
callbacks are the *real driver* delivering its own half-buffer more than
1.5 periods after the last one, which the ring absorbs; `dry` counts gaps
over 2.5 periods; `repeated` byte-compares what was handed to the driver
against the previous callback; `dropped` counts client blocks that became
silence, which is the audible event itself; `stalls` counts episodes of a
tenant owing more than one block, and `hw-in peak` is the loudest sample
the real inputs delivered. Next time somebody says "distorted", this line
names who missed the deadline and by how much.

Two of those counters were lying when they were written, and both lied in
the direction that teaches the wrong lesson:

* `repeated` byte-compared silent channels too, so an idle output counted
  as a repeat forever — 1501 per window on a run with nothing but a tone
  on one channel. It now skips the comparison when the block's peak is 0:
  a channel holding silence is not a channel repeating itself.
* `stalls` measured a slot's debt against the raw clock, so it fired on
  the one block of pipeline depth the protocol *asks* for — we publish a
  client's inputs, then it publishes its outputs — and read 749 per
  window, every other callback. It now measures against that slot's
  releases (`in_written[c]` versus `out_seq + 1`), counts an episode once
  per slot (`stalled_now`), and reports how deep it went.

A clean window reads `0 repeated blocks, 0 dropped, 0 stalls`, which is
what every window of the runs below does.

## The clock, and who gets to pick its size

The hardware's block size is the bus's clock, and every client is paced by
it whatever its own host asked for — a client's block and the hardware's
quantum are independent, because the publish gate is `f0 + block >
committed` rather than "in step with the DAC". So REAPER at 512 and a host
at 64 both ride the same DAC clock, and "it works better with a larger
blocksize" is a statement about the *hardware* block, not the one in a
host's preferences dialog. A client whose block is smaller than the clock
simply gets several `bufferSwitch`es per hardware tick: native rate, coarser
spacing.

The size itself comes from, in order of authority:

1. **`HwBlock`** (DWORD, `HKCU\Software\NiceRackAsio`), or
   `NICEASIO_HW_BLOCK`, which outranks the registry because a sandboxed probe
   cannot write HKCU at all; `niceroute-probe hwblock [FRAMES]` reads and
   writes it. Honoured exactly.
2. **The first host to start.** A seat goes active only from
   `Instance::start`, and the block published beside it is the one that host
   passed to our own `createBuffers` — so waiting for a seat is waiting for
   the settings page on the other side of the boundary, and the number the
   user chose in NiceRack's Output device row is now the number the DAC runs
   at. It wins over the driver's preference because the host is the one with
   deadlines to keep: holding the clock at twice the period a DAW told its
   plugins to finish inside adds latency and moves the driver's jitter onto
   every seat, and buys nobody extra time.
3. **`CLOCK_FLOOR = 128`**, which only speaks when nobody started — the case
   where the only opinion left is the driver's own `preferred`.

Whatever speaks, its number is fitted *upward* into the driver's advertised
set (`fit_size`: clamp to `min..max`, then next multiple of a linear step or
next power of two, because a host asked for this much and less is not an
answer). One size is refused outright: one outside the rings' own
`MIN_BLOCK..=MAX_BLOCK`. `createBuffers` is the only call in the ASIO vtable
that names a size — there is no `setBufferSize` in `azo-sys-0.4.1/src/lib.rs`
— so a refusal is retried once at the driver's own floored preference, and if
even that is refused the device goes virtual-only and puts the refusal in the
note and the log.

The consequence worth knowing: **the hardware is no longer opened into
buffering at `ASIOInit`.** `open_hardware` initializes the driver and reads
its channels, formats and range, then the master waits (5 ms polls, up to 30
s, cancellable by teardown) for a host to start, and only then creates
buffers and calls `start`. `header.quantum` reads `0` until then, on purpose:
a placeholder that looks like an answer is the one thing a diagnostic header
must not be. Nobody playing means the converters are untouched — which is
most of what the "a copy card that also grabs the converters" item below was
asking for, and why a second output no longer lands on the DAC the instant a
host opens the device.

The measurements behind the floor, on this machine, tone muted at the driver
door (`solo --tone 440`, no host but the probe):

| hw quantum | period | late callbacks | worst gap | work max | dropped/stalls/repeated |
|---|---|---|---|---|---|
| 64 | 1.33 ms | 255 of 1501 (17 %) | 2.08 ms | 54 us | 0 / 0 / 0 |
| 128 | 2.67 ms | 4 of 751 | 4.03 ms | 91 us | 0 / 0 / 0 |
| 256 | 5.33 ms | 6 of 376 | 8.05 ms | 59 us | 0 / 0 / 0 |

Komplete's preferred size is 64. Nothing was ever dropped at 64, but the
driver wakes on 1 ms USB frames and 1.33 ms does not divide them, so a
sixth of its callbacks came up to 2.08 ms apart and the two halves of the
DMA ring were the *entire* margin. At 128 and above the same jitter is
simply smaller than the period, and the late count drops by an order of
magnitude. So the floor is insurance for the case with no deadline attached;
a host that asked for 64 gets 64 and gives up jitter tolerance, not safety —
nothing in those rows is a dropped frame, and nobody is holding a microphone
to a speaker through this path. The log says which voice spoke:
`master: clocking the hardware at 64 frames (seat 0 asked for 64, this driver
takes 64)`.

## The trace log

`%TEMP%\niceroute.log`, one line per startup-path event
(`{secs} pid {pid} {msg}`), opened lazily, best-effort, deletable any time.
Lines cover the election, which real driver was opened and what it answered,
bus geometry, `createBuffers`/`start`/`stop`, refused sample rates, refused
QIs, unknown `future` selectors, and panic payloads from the master thread.
`niceroute::trace::path()` prints it — **per process**: a sandboxed run has
its own temporary directory, so a probe's lines and REAPER's lines can sit
in two different files with the same name. Worse, each sandboxed shell gets
a *fresh* one, so a measurement that runs the probe and then reads "the"
log has to pin `$env:TMP`/`$env:TEMP` to a fixed directory first —
`.workspace\logs` here, which is why the probe invocations in this file all
start with that. Nothing on the audio path logs,
except under `NICEASIO_SILENT_HW`, which is opt-in for exactly that reason.

The corollary bites when reading a *host's* session: the file is chosen from
the temp directory of whichever process loaded the DLL, which is inherited
from whatever started it. A NiceRack launched from a build shell logged to
`target\build-tmp\niceroute.log` while the same app launched from Explorer
wrote to `%LOCALAPPDATA%\Temp`, and a tail of the wrong one reads exactly
like a device that went silent. Do not guess which file is live — list every
`niceroute.log` modified in the last few minutes and read the one whose pids
match `Get-Process`:

```powershell
Get-ChildItem C:\Users\$env:USERNAME\AppData\Local, . -Recurse -Filter niceroute.log -EA SilentlyContinue |
  Where-Object LastWriteTime -gt (Get-Date).AddMinutes(-10) | Select-Object FullName,LastWriteTime
Get-Process nice-rack,reaper | Select-Object Id,ProcessName   # match the pid column
```

## Two behaviours worth knowing before the next debug session

* `init()` **returns false** when it cannot join the bus. It used to answer
  "yes, open, silent", which lets a host paint itself into a corner with a
  device that never clocks. A sandboxed process cannot open kernel objects
  created outside its sandbox, so `init: bus failed: Access is denied.
  (os error 5)` from a probe while REAPER is running is that case: a
  harness limit, not a product bug. Multi-instance tests must live in one
  sandbox, and `solo`/`--tone` need REAPER closed.
* REAPER's *"Close audio device when stopped and application is inactive"*
  (on by default) makes it release the device after a while and the corner
  reads `[audio devices closed]`. Benign — the log shows a clean `stop:`
  and a clean reopen. But while REAPER is the master its abdication stops
  the bus clock, and another client needs ~3 s (heartbeat timeout) to take
  over, so turn it off while developing against this device.

## Registration — DONE on 2026-11-17 evening (with a caveat)

The device **is registered**: `HKCU\Software\ASIO\NiceRack ASIO (x64)`,
`HKCU\Software\Classes\CLSID\{7A4E1C93-…}\InprocServer32` → `target\debug\niceroute.dll`,
and `HKCU\Software\NiceRackAsio` (`RealDriver = Komplete Audio ASIO Driver`, HwIn/HwOut 2/2).
`niceroute-probe list` shows it; `cocreate` proves COM activates it from the registry.

**Caveat: the machine's security software blocks registry writes from the
unsigned dev binaries** — `--register-asio`/`niceroute-probe register`
(same for `unregister`) die with `Access is denied (os error 5)` both in the
agent sandbox and from a plain terminal, while `reg.exe` writes HKCU freely
and the ACLs grant the user FullControl. So:

* register again (after a `cargo clean`) with `reg.exe` commands or
  `.workspace\tmp\register-niceroute.reg` (double-click);
* **REAPER (v7.78) only scans `HKLM\SOFTWARE\ASIO`** — an HKCU-only
  registration is invisible to it (every other driver in its list is an
  HKLM entry). Machine-wide keys are mirrored by
  `.workspace\tmp\register-niceroute-hklm.reg` (one UAC prompt; done
  2026-11-17 evening, verified). Keep both trees: hosts differ, the DLL
  itself reads its config from HKCU either way;
* unregister: `reg delete "HKCU\Software\ASIO\NiceRack ASIO (x64)" /f` and
  `reg delete "HKCU\Software\Classes\CLSID\{7A4E1C93-6D2B-4F58-A0B7-5C31D9E84216}" /f`;
* runtime is unaffected — the DLL only *reads* config — but whitelisting
  `nice-rack.exe`/`niceroute-probe.exe` in the AV would restore the CLI.

## Who plugs into what

Every host loads the *same* `niceroute.dll`. The first instance to grab the
election mutex becomes master: it hosts the real driver, owns the clock and
mixes. Everyone else is a client and cannot tell the difference from the
outside — REAPER's dropdown says `NiceRack ASIO (x64)` whichever role it
lands in, and the order the hosts start in does not matter.

What a client is handed, with `HwIn = HwOut = 2` (the `instance.rs`
`channel_counts` answer, so it is a real number before anything else is
up): **10 inputs and 8 outputs**.

* Input index `i < hw_in` is the physical line (`Line 1`, `Line 2`).
* Input index `i >= hw_in` is a tap: bus channel
  `VIRT_FIRST + (i - hw_in)` (`Nice 1`…`Nice 8`) — the mix of every *other*
  tenant's outputs, delayed one block.
* Output index `p` lands on bus channel `VIRT_FIRST + p` (`Nice Out 1`…`8`).
  It reaches every other tenant, and it reaches the DAC **only while
  `p < hw_out`** — anything a client puts on an output above `HwOut` is a
  conversation between the tenants and never touches the converters.

Worked example, as actually wired and listening on this machine the night
the sample shape was fixed:

* nice-rack: **Input device** and **Output device** = `Komplete Audio ASIO
  Driver` — the real driver, both directions, so its live audio never
  touches this bus — and **Second device** = `NiceRack ASIO (x64)` starting
  at output 1. The status line reads `Audio running — copy to 'NiceRack
  ASIO (x64)' 1–2`, which `runtime.rs` only prints for a second driver that
  really opened: a refusal never reaches that line.
* The copy group is a client of the bus, and our DLL in that same process is
  also its master: `master: bus is 48000 Hz, hw 2+2` alongside nice-rack's
  own stream on the same converters. Nothing tells the DLL it was hired as a
  copy card — it reads `RealDriver` and hosts it like any other tenant that
  got there first. That is the signature of this shape in a log, and it is
  worth knowing before reading `hardware started, it is the clock` as an
  exclusive claim.
* The copy is client outputs 1–2, which are bus planes `VIRT_FIRST + 0/1` =
  8 and 9. REAPER, opened on `NiceRack ASIO (x64)`, joins as a second client
  and finds them on **inputs 3 and 4** — `i = 2` and `3`, so
  `VIRT_FIRST + (i - 2)` — because the two physical lines are numbered
  first. That is the wiring the user verified with ears.
* Before REAPER opens, the witness lines read `0 repeated blocks, 0 dropped,
  0 stalls (worst N blocks deep)` with N climbing by ~751 every two seconds.
  That is the copy being made and nobody claiming it: the depth is measured
  against the slowest reader, and with no reader it never advances. It is
  not a stall — nothing is late and nothing is dropped — but it is why the
  status line says where the copy is *going* rather than promising anybody
  hears it.

The other shape, `NiceRack ASIO (x64)` as nice-rack's own in *and* out device
with the copy as a second group on the same driver, is what the code was
first pointed at: a copy naming the driver it copies from is *not* a second
driver — `nice-common`'s `Wanted::second_output` asks the one driver for
both groups in the single `ASIOCreateBuffers` it gets, because that call
takes the whole buffer list and a group cannot be appended later — so
nice-rack would be one client with four outputs open, live on 1–2 and copy
on 3–4, the copy landing on bus planes 10–11 and REAPER reading it on inputs
5–6. It is unverified: through that shape nice-rack's watchdog
(`runtime.rs`, three seconds without both its input and output counters
moving) stopped the stream repeatedly while this device's own witness lines
were clean, which is still open. The topology above is the one that works
today; the one-channel-per-claim rule of the bus is what both shapes rest
on.

`niceroute-probe loopback` proves the same transport with no hardware in
the picture at all: A's 440 Hz at 0.25 on output 1 arrives at B's input 1
at RMS 0.1767 against the 0.1768 a quarter-scale sine should measure, B's
input 2 is exactly zero (no crosstalk), and A's own input 1 is exactly
zero — a tenant hears everybody else and never itself, which is what makes
it safe to monitor this bus at all (bug 4).

## Go live in REAPER

Two ways to wire it; both use **NiceRack ASIO (x64)** as picked in
REAPER → Preferences → Audio → Device (ASIO).

**A. Recording tap, speakers untouched — wired and heard on 2026-11-18.**
nice-rack's **Input device** and **Output device** stay
`Komplete Audio ASIO Driver`, so its live audio and its monitoring never
enter the bus; **Second device** = `NiceRack ASIO (x64)`, copy at output 1.
REAPER picks **NiceRack ASIO (x64)** and records **inputs 3/4** — the two
physical lines are numbered first, and the copy is the first tap after them.
Expect `master: bus is 48000 Hz, hw 2+2` in the trace log even though nobody
asked this device to be a driver: the DLL cannot tell it was hired as a copy
card, so it reads `RealDriver` and hosts Komplete a second time rather than
falling back to virtual-only. Two ASIO instances on one converter is not a
measured arrangement — say so before recommending it — but both streams ran
and the copy arrived.

**B. Everything through the bus — not working yet.** nice-rack's main device
`NiceRack ASIO (x64)` in both directions, so whoever opens first is master,
hosts Komplete and owns the speakers, and a second device on the *same*
driver carries the copy (which is one driver with two groups, not two
drivers: `asio_second_output` then has to sit above the live group's width).
Through that shape nice-rack's watchdog stops the stream a few seconds in —
`Audio devices stopped delivering input/output callbacks` — while this
device's own witness lines report a healthy 128-frame clock, `0 dropped`,
`0 stalls`, and `solo --block 64` proves the client pump delivers 5998 of
6000 callbacks at a client block the bus does not even use. Where nice-common
needs a callback this pump is not producing is still open; wire A until it is
closed.

Both: re-run `.workspace\tmp\register-niceroute.reg` after any
`cargo clean` (it deletes the DLL the registration points at). If the
master process is killed while streaming there is a ≥ 3 s heartbeat stall
before another client takes over — v1 gap, as designed. `NICEASIO_NO_HW=1`
forces virtual-only for experiments; `RealDriver` lives under
`HKCU\Software\NiceRackAsio` and is re-read at every election.

## Repo state

All work is **uncommitted** (branch tip = PR #49). `git status` says:
workspace `Cargo.toml` + lock (new member), `nice-common` (HKCU ASIO
enumeration in `devices.rs`, plus `asio_io.rs`, `audio_io.rs`, `lib.rs`,
`settings.rs` and its README), `standalone` (`--register-asio` /
`--unregister-asio` flags, and its `Cargo.toml`), `nice-rack/src/runtime.rs`
(the status line now names the copy card's live destination driver and
channel span), `.codex/AGENTS.md` (performance-critical work
must be built `--release`), plus the new crate `nice-rack/crates/nice-azo-host`
(`design.md` is the authoritative spec; this file is just the handoff).
Untracked helpers under `.workspace\`: `verify-hwout.ps1` (the muted
hardware-output proof) and `install-niceroute.ps1` (install the optimized
DLL at the registered path).
The wiring in "Who plugs into what" lives in `nice-rack-settings.ron` at
the repo root, which is **git-ignored** — it is this machine's state, not a
change to carry into a PR.
Suggested split: PR 1 = nice-common HKCU enumeration, PR 2 = the new crate +
standalone flags.

One trap that survives every one of these bullet points: the registered
path is `target\debug\niceroute.dll`, and it holds the **release** binary
because `install-niceroute.ps1` put it there. A plain `cargo build -p
nice-azo-host` overwrites it with a debug device and the sound goes back to
being bad for a reason that has nothing to do with the code.

## Open, found the night the copy card went live

* **The watchdog, when this device is the host's *main* driver.** nice-rack
  ran 4–6 s, three times, each ending in its own `stop: called`, and the UI
  said `Audio devices stopped delivering input/output callbacks` — while
  this device's witness lines in the same window were healthy (`quantum
  128`, `0 dropped`, `0 stalls`). The obvious suspect was disproved: a client
  block smaller than the bus quantum, which `solo --block 64` answers with
  5998 of 6000 callbacks. What the watchdog actually reads is
  `counters.input` **and** `counters.output` (`runtime.rs:542-560`), and both
  move together inside `impl Pump for Driver` (`audio_io.rs:334` and `:354`)
  — so the failure is upstream of the counters, in whatever nice-common does
  with a driver that hands it 10 inputs and 8 outputs. Reproduce with the
  device as the main driver both directions; do not debug it from this
  crate's log alone, because from here the stream looks fine.
* **A copy card that also grabs the converters.** Nothing tells the DLL it
  was opened as somebody's second output, so it hosts `RealDriver` like any
  first tenant and the interface ends up with two ASIO instances. Either
  give a non-first instance a reason to refuse the hardware, or accept it
  and say why. It is a design decision, not a bug found in the field.
  *Partly answered since:* the hardware is now buffered and started only
  after a host actually starts (`Instance::start` → the seat's published
  block → `HwOpen::arm`), so an opened-but-idle copy card leaves the
  converters alone. Two tenants *playing at once* still both hold the
  interface, which is the half that needs the decision.
* Register the **release** path instead of overwriting
  `target\debug\niceroute.dll`, so installing the fast device no longer
  depends on the trap two paragraphs above.

## Deferred backlog

* BOM tolerance question; `Runtime::edit()` drops cards while busy
  (`runtime.rs:390-401`); OPENING_LIMIT worker-wait miss; %TEMP% note
  `nice-tuner/README.md:387-389`; make
  `a_second_drop_while_the_devices_are_open_settles_again` fail loudly.
