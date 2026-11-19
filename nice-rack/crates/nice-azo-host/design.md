# NiceRack ASIO — the design in one page

## What it is

Our own answer to ReaRoute, seen from the side we control. Every ASIO host
(REAPER, nice-rack itself, anything) loads `niceroute.dll`; every instance is
a *client* of one shared-memory bus. Whichever instance grabs the election
mutex first additionally becomes the **master**: it owns the clock, serves
every client's blocks, and — if configured to — hosts the *real* ASIO driver
with azo and lets hardware through. There is no ini file to find, no
second process that must be "the server", and `ASIOGetChannels` answers with
real numbers before anything else is up: the 0-in-0-out failure class that
killed the ReaRoute experiment cannot be expressed here.

```
host A (REAPER)          host B (nice-rack)         master's extra duty
┌────────────────┐       ┌────────────────┐        ┌───────────────────┐
│ niceroute.dll  │       │ niceroute.dll  │        │ azo ⇄ real driver │
│   client role ─┼─shm+──┼─ client role   │        │  clock + mix +    │
│                │ events│ (also master?) │◄───────┤  hw passthrough   │
└────────────────┘       └────────────────┘        └───────────────────┘
```

## Roles

* **Every instance** tries election in `init()`: `CreateMutexA` + a
  zero-timeout wait. First live instance wins and holds the mutex until the
  process ends; an abandoned mutex (master crashed) is winnable, and clients
  re-elect when the heartbeat stalls.
* **Client role** = what `driver.rs` does: allocate the half-open buffers the
  host asks for, pump `bufferSwitch` from the bus (directProcess = true, so
  the host's return means "outputs are filled"), and mirror the bus's sample
  rate/position answers.
* **Master role** = `master.rs`: a service loop advanced either by the real
  driver's own callback (clock = hardware) or by a paced timer thread (clock
  = internal), stamping each active slot's blocks in and out, mixing client
  outputs onto hardware outputs, and copying hardware inputs to everyone.

## The bus (v1)

One pagefile mapping, `Local\NiceRackAsio.bus.v1`, session-local by purpose:
only apps of this user in this session are the audience. A 128-byte header
(magic/version/epoch, rate, hw channel counts, heartbeat in frames) precedes
eight slots. Each slot is SPSC in both directions: four block-sized rings of
16 bus channels — channels 0–7 are hardware pass-through slots, 8–15 the
virtual ones — plus two sequence counters (`in_seq` written by master,
`out_seq` written by the client). Named auto-reset events per slot
(`…slot{i}.in` / `.out`) do the signaling; every counter is an aligned atomic
u64, every ring index is `block_number % 4`, and silence is just "no block
there yet". Frames are addressed absolutely: a slot stamps its
`start_frame` when the master sees it activate, so block *n* of a client
covers `[start + n·B, start + (n+1)·B)` — the mixer reads any client's ring
at any position without a per-pair FIFO.

### Clocking and mix order

One `step()` on the master thread does three passes in a fixed order, and
the order is the protocol:

1. **Zero frontier.** Extend the mix's zeroed-ahead window; every cell any
   reader can reach this step is either real audio or a guaranteed zero,
   never somebody's wrap-around leftovers.
2. **Consume.** Sum every slot's newly published output blocks into the
   frame-addressed mix. A block that merely *reached* the clock edge is
   live audio and sums normally; only blocks more than `BACKLOG_FRAMES`
   behind the live edge are history — their backlog is dropped and the
   slot resyncs at the edge. (An earlier draft dropped every block whose
   frames had *touched* the edge, which silently discarded all of a healthy
   lockstep client's audio: the pump writes block *n* exactly as the clock
   passes its last frame.)
3. **Publish.** Write each slot's input blocks — but only once **every
   other live slot's output has been summed through the block's first
   frame**, and carrying the mix delayed by one block. This one block of
   deliberate cross-client latency is what breaks the simultaneous-
   first-block deadlock (clients are paced by this very store, so waiting
   for a peer's block *n* to reach the mix before publishing block *n+1*
   is self-limiting), and frames before a peer's `start_frame` need no
   summing — the frontier keeps them silence, the honest sound of a tenant
   that has not started. A peer stranded more than `BACKLOG_FRAMES` behind
   the live edge is treated as silent rather than waited on: one crashed
   client must never hold the whole bus hostage. As blocks are summed, the
   consume pass also copies each slot's newest block into a master-side
   snapshot (`own_last`); publishing subtracts that snapshot back out of
   the frames it covers, which is what makes "a client never hears
   itself" true of the mix and not merely of the patch-bay's intent. The
   snapshot is read rather than the live ring cell because the ring is
   SPSC: the client may already be writing its next block into the cell
   being subtracted. A reader catching up after a stall publishes blocks
   older than the snapshot and briefly carries its own echo until it is
   back at the live edge — the honest trade against subtracting torn
   cells. The loopback proves the steady state: a sender's own inputs
   measure exact zero while its peer receives its tone whole.

The virtual clock paces itself on a high-resolution waitable timer (plain
timer plus a raised system tick as fallback — `Sleep`'s 15.6 ms scheduler
cadence will eat a 128-frame period whole) and rebases on missed deadlines
instead of firing bursts of expired steps.

### The one-window rule

A client publishes block *m* (frames `f0 … f0+block`, `f0 = start + m·block`)
only after the master has released that block's inputs, so at clock edge `g`
the newest frames the mix can possibly contain are `[g-block, g)`; everything
at or past `g` is frontier zero **by construction**. Every reader of bus time
therefore reads one block back from the edge it was handed:

* the publish pass writes a slot's input block from the mix at `f0 - block`
  (that is the cross-client delay of step 3, not an accident of ordering);
* the hardware *output* copies the mix at `g - block` into the real driver's
  half-buffer;
* the hardware *input* is published at `f0 - block` too, so a mic and the mix
  reach a client's two buffers from the same slice of bus time.

Read the edge itself instead of one block behind it and you are reading
silence: which is precisely how a bus that mixed perfectly ended up playing
nothing at all out of the speakers. The cost is one block of output delay on
every path, so `getLatencies` answers `2 · block` in and out (one buffer of
the device's own double buffer, one bus relay), and both directions stay in
phase with each other — a monitor path that sums the two stays comb-free.
The one exception is the *first* hardware callback, where `g - block` does
not exist: it writes silence, because copying frames `[0, block)` there puts
the same samples in both halves — a click, plus a repeat the witness would
then have to be told to excuse.

## Patch bay (v1, console semantics)

Channel *k* is a bus. A client's **output k** is summed into every *other*
client's **virtual input** (bus channel 8+k) and, when the master has
hardware, into hardware output k — **only while k is under `HwOut`**, so an
output above the converter count is a conversation between tenants and
never reaches the DAC. **Input** channels presented to hosts are the
hardware inputs first (`Line 1…H` — absent when the master runs
virtual-only) and then eight virtual channels (`Nice 1…8`): with `HwIn =
HwOut = 2` a host is handed 10 inputs and 8 outputs, and input index `i`
reads bus channel `i` while `i < HwIn`, `VIRT_FIRST + (i - HwIn)` after.
Presentation outputs *are* the virtual channels: output `p` is bus channel
8+p. A client never hears itself; there is no matrix and no gain in v1,
which keeps the rack's own routing the single place levels are decided.

That is exactly the rig this crate was demanded by, and the machine is
wired that way now: nice-rack runs `backend = asio` with
`NiceRack ASIO (x64)` as both its devices and `asio_second_output = 3`,
which is one driver with two groups — both asked for in the one
`ASIOCreateBuffers`, because that call takes the whole buffer list — its
live pair on outputs 1–2, which the DAC plays, and its copy on outputs 3–4.
Outputs 3–4 are bus planes 2–3, which REAPER — on the same device, joined
as a client — finds on inputs 5–6, and records; its own playback still
reaches the speakers because the master sums its outputs 1–2 onto the real
interface. Either direction works without the other app running: the bus is
up the moment any host initializes the driver.

## Hardware hosting rules

The master opens its real driver by name from
`HKCU\Software\NiceRackAsio\RealDriver` (empty = stay virtual-only, which is
the safe default for tests). It never changes that driver's sample rate — it
adopts it, header included — and on any refusal runs virtual-only with the
refusal as `getErrorMessage` rather than failing the host. Hw channel caps
are `HwIn`/`HwOut` (default 2/2), clamped to what the real driver has.

### What a hardware buffer is made of

`createBuffers` hands back `void**` and no format. The shape of the samples
is whatever each channel answered from `getChannelInfo`, and that answer is
not a formality: the Komplete Audio driver says `PCM_I32_LSB` for every
channel it has. The master therefore asks every channel it is about to use,
logs the answer, and keeps a per-channel `HwFmt` — `F32` or `I32`, both four
bytes a frame — converting on both legs (`load_frames`, `store_frames`,
`silence_frames`). Anything else is refused with `buffers are not 32-bit
float or int: …`, including a driver that will not answer `getChannelInfo`:
a format is not something to guess at, and guessing wrong is silent in the
only way that matters — `0.0f32` and the integer `0` are the same bytes, so
a host writing floats into int buffers plays correct silence and everything
else as fuzz, while every measurement *inside* the bus keeps reporting a
perfect mix. Two details: full scale is 2³¹, and `i32::MAX as f32` rounds to
`2147483648.0`, so `+1.0` is one count out of range while `−1.0` is exactly
on the bottom — both rails are clamped explicitly rather than left to the
saturation of a cast; and the conversion belongs to the hardware channel,
never to the bus, because what *our* hosts are handed is still
`PCM_F32_LSB` (`abi.rs`) — the bus is float.

### The clock, and who names its size

The hardware's block is the bus's clock, and it is independent of any
client's: the publish gate is `f0 + block > committed`, not lockstep with
the DAC, so a host at 512 and a host at 64 ride the same quantum — the
smaller one simply gets several `bufferSwitch`es per hardware tick. What
names the clock, in order of authority: `HwBlock` (DWORD under the same key)
or `NICEASIO_HW_BLOCK` for one run — it outranks the registry because a
sandboxed probe cannot write HKCU, and an A/B of two clock sizes should be
two command lines — then the block the **first host to start** published in
its own `createBuffers`, and only if nobody ever starts, the driver's
`preferred` floored at `CLOCK_FLOOR = 128`. `niceroute-probe hwblock
[FRAMES]` reads and writes the registry value.

The host's number beats the driver's preference because the host holds the
deadlines: doubling the period a DAW told its graph to finish inside buys no
extra time, adds latency, and moves the driver's own jitter onto every seat.
`CLOCK_FLOOR` is insurance for the one case with no deadline attached.

`createBuffers` is the only call in the vtable that names a size, so the
question is answered at the first moment anybody has asked: the driver is
initialized and read at election time, but its buffers are created and
`start` called only after a seat goes active, up to 30 s later (5 ms polls,
cancellable — `ASIOInit` never blocks on the wait, and nobody playing leaves
the converters alone). A wanted size is fitted *upward* into the driver's
own set (`fit_size`: clamp to `min..max`, then the next multiple of a linear
step, else the next power of two), and a size outside `MIN_BLOCK..=MAX_BLOCK`
— the rings' own span — is refused by name rather than accepted quietly. One
retry at the driver's floored preference, then virtual-only with the refusal
in the note. `header.quantum` is `0` until a clock exists.

Measured here, on a driver whose preference is 64: at 64 (1.33 ms) 255 of
1501 callbacks landed more than 1.5 periods apart, worst 2.08 ms, work max
54 us; at 128 (2.67 ms), 4 of 751, worst 4.03 ms, work max 91 us; at 256
(5.33 ms), 6 of 376, worst 8.05 ms, work max 59 us — nothing dropped,
stalled or repeated at any size. The driver wakes on 1 ms USB frames and
1.33 ms does not divide them, so at 64 the two halves of the DMA ring were
the entire margin; what following a host's 64 gives up is that margin, not a
frame.

The MMCSS claim is made by the thread that actually answers the callback —
the real driver's own, which registers lazily on its first call, because
the master thread parks in hardware mode and a priority claimed by a parked
thread dresses the spectator.

### Leaving go

On the parked-hardware exit path both halves of every output buffer are
zeroed and one period is slept before `handle.stop()`: a driver keeps
cycling its own DMA ring after the host walks away, and given a block of
music it will happily replay it at the room forever — which is what
"the sound continues looping after I stopped" was. `ENGINE` is nulled
before `dispose_buffers()`, so a driver that calls back late finds a
defined no-op rather than freed memory.

## Registration

Per-user and reversible: `HKCU\Software\ASIO\NiceRack ASIO (x64)` (the
classic name → clsid convention every ASIO host walks, HKCU included) plus
`HKCU\Software\Classes\CLSID\{7A4E1C93-6D2B-4F58-A0B7-5C31D9E84216}\
InprocServer32` for COM. `nice-rack --register-asio [--register-real NAME]`
writes them; `--unregister-asio` removes them. azo's own enumerator reads
HKLM only, so `devices.rs` merges HKCU into the rack's list as well.

## Boundaries of v1 (stated, not forgotten)

* Fixed format contract, in both directions and stated as two: hosts are
  always handed Float32LSB, because the bus is float; the hosted driver is
  spoken to in whatever shape it asked for, Float32LSB or Int32LSB, per
  channel. 24-bit, DSD and anything else is refused rather than guessed at.
  44.1 kHz hosts must ask the master instance's real driver to live there;
  no time code.
* Master death is survived by re-election, not by sample-continuous handoff:
  expect one gap while the next host adopts the clock.
* Client clocks are steered by delivery events, so they follow the master
  exactly; a host that free-runs faster than its delivery is dropped a block,
  and a starved one repeats silence, never garbage.
* OBS still cannot load any ASIO driver — WASAPI loopback capture of the
  rack's monitor remains the OBS story.
