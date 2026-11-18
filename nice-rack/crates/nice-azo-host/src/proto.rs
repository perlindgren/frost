//! The bus protocol: header, slots, block rings, and the arithmetic both
//! sides of the mapping agree on. Pure data and pure math on purpose —
//! everything here is unit-testable without a mapping, an event, or a
//! second process, and the byte layout is the contract, so it is stated as
//! `#[repr(C)]` with assertions rather than hand-computed in comments.
//!
//! Addressing is absolute host frames everywhere: a slot publishes the
//! frame its first block starts on, every ring position is derived from
//! the block number, and the mix sums per-frame. A consumer that wakes
//! late never mixes blocks that belong to different moments in time —
//! the mistake a slot-per-tick scheme makes the first time anybody's
//! thread hiccups.

use std::sync::atomic::{AtomicU32, AtomicU64};

macro_rules! static_assert {
    ($t:ty, $size:expr) => {
        const _: () = assert!(std::mem::size_of::<$t>() == $size);
    };
}

macro_rules! static_assert_align8 {
    ($t:ty) => {
        const _: () = assert!(std::mem::align_of::<$t>() == 8);
    };
}

/// `"NICE"` — checked before anybody dereferences a slot. A stale mapping
/// from an older session can linger for as long as the last handle lives;
/// this is what says it is ours and this version of ours.
pub const MAGIC: u32 = 0x4E49_4345;
pub const VERSION: u32 = 1;

/// Enough seats for a host per person plus stragglers; the whole bus is
/// ~4 MiB at these numbers, so the ceiling costs nothing.
pub const SLOTS: usize = 8;
/// Blocks the ring holds. Two would let the producer lap the consumer
/// the moment either is descheduled; four buys slack without letting a
/// dead client's stale blocks age the audio.
pub const RINGS: usize = 4;
/// Bus channels: 0..HW_FIRST* stay the hardware's, the upper half is the
/// virtual side every client both hears and feeds.
pub const BUS_CH: usize = 16;
/// Where the virtual channels start; also the number of hardware channels
/// the bus can carry, and what a virtual-only master shows for `hw_*`.
pub const VIRT_FIRST: usize = 8;
/// The virtual side has one fixed size, so `getChannels` never has to
/// change its answer mid-session.
pub const VIRT_CH: usize = BUS_CH - VIRT_FIRST;
/// ASIO's granularity answer says powers of two up to here.
pub const MAX_BLOCK: usize = 1024;
pub const MIN_BLOCK: usize = 32;
/// Default master clock block when nothing hardware-paced exists.
pub const DEFAULT_QUANTUM: usize = 128;

pub const MAPPING_NAME: &str = r"Local\NiceRackAsio.bus.v1";
/// The election mutex. Holding it *is* being master; releasing it is the
/// only abdication, and dying releases it too, which is the point.
pub const ELECTION_NAME: &str = r"Local\NiceRackAsio.elect.v1";
/// Per-slot data-ready event, named after the ring it guards. Only the
/// input direction has one: the master consumes outputs on its own
/// clock, so nobody needs to wake it.
pub fn slot_event_in(slot: usize) -> String {
    format!(r"Local\NiceRackAsio.slot{slot}.in.v1")
}

/// Slot states. A zero slot is free; the mapping pages arrive zeroed, so
/// "free" is the state a fresh and an abandoned mapping share.
pub const SLOT_FREE: u32 = 0;
pub const SLOT_ACTIVE: u32 = 1;

/// Everything that describes the bus as a whole. Sized to 128 bytes so a
/// future header field needs no renumbering of the slots.
#[repr(C)]
pub struct Header {
    pub magic: AtomicU32,
    pub version: AtomicU32,
    /// Bumped by each new master. A client that sees the epoch move knows
    /// it has been talking to a dead master's leftovers.
    pub epoch: AtomicU32,
    pub master_pid: AtomicU32,
    pub rate_hz: AtomicU32,
    pub hw_in: AtomicU32,
    pub hw_out: AtomicU32,
    pub virt_ch: AtomicU32,
    pub bus_ch: AtomicU32,
    pub max_block: AtomicU32,
    /// Master's own clock block, for diagnostics and for the virtual
    /// clock thread's pacing.
    pub quantum: AtomicU32,
    pub _pad0: AtomicU32,
    /// The master's write cursor, in absolute frames. A client watches
    /// this to notice a dead master; hosts see it through
    /// `getSamplePosition`.
    pub heartbeat_frames: AtomicU64,
    pub _pad1: [u8; 72],
}

static_assert!(Header, 128);
static_assert_align8!(Header);

/// One client's seat. `in_*` is master→client (the inputs we present),
/// `out_*` is client→master (the mix contribution); each direction is a
/// single-producer/single-consumer ring guarded by the sequence pair and
/// one named event, so nobody ever waits on a mutex for audio.
#[repr(C)]
pub struct Slot {
    pub state: AtomicU32,
    /// Frames per block, set by the master when it adopts the slot.
    pub block: AtomicU32,
    pub _pad0: u32,
    /// Absolute frame of this slot's block 0. Zero until the master
    /// stamps it, which is also "not yet audible": the client publishes
    /// output blocks before the stamp lands, and they simply join the mix
    /// from the stamped moment.
    pub start_frame: AtomicU64,
    /// Blocks the master has *written* (consumer polls `> in_seq`).
    pub in_seq: AtomicU64,
    /// Blocks the client has *written* (master consumes `< out_seq`).
    pub out_seq: AtomicU64,
    /// Blocks the master has *consumed*; the client must not overwrite
    /// block `n` until `consumed_out > n - RINGS`.
    pub consumed_out: AtomicU64,
    /// Blocks the client has *finished reading*; the master must not
    /// write block `n` until `read_in > n - RINGS`. The mirror of
    /// `consumed_out`, and the reason neither direction can ever lap
    /// the other into overwritten audio.
    pub read_in: AtomicU64,
    pub _pad1: [u8; 8],
    /// `[RINGS][BUS_CH][MAX_BLOCK]`, indexed by [`ring_off`].
    pub in_ring: [f32; RINGS * BUS_CH * MAX_BLOCK],
    pub out_ring: [f32; RINGS * BUS_CH * MAX_BLOCK],
}

static_assert!(Slot, 64 + 2 * (RINGS * BUS_CH * MAX_BLOCK * 4));
static_assert_align8!(Slot);

/// Byte offset of the whole mapping.
pub const MAPPING_SIZE: usize = std::mem::size_of::<Header>() + SLOTS * std::mem::size_of::<Slot>();

/// Element offset of `(block_number, channel)`'s block start inside one
/// ring. The frame index runs along the tail so a block is one memcpy.
pub const fn ring_off(block_number: u64, ch: usize) -> usize {
    ((block_number % RINGS as u64) as usize * BUS_CH + ch) * MAX_BLOCK
}

/// `n` rounded up to the next multiple of `block` (power of two).
pub const fn align_up(n: u64, block: u64) -> u64 {
    n.div_ceil(block) * block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_slots_wrap_and_channels_are_contiguous() {
        // Consecutive channels of the same block sit MAX_BLOCK apart, and
        // block+RINGS lands back on the same ring position.
        assert_eq!(ring_off(0, 0), 0);
        assert_eq!(ring_off(0, 1), MAX_BLOCK);
        assert_eq!(ring_off(1, 0), BUS_CH * MAX_BLOCK);
        assert_eq!(ring_off(RINGS as u64, 3), ring_off(0, 3));
        // Every element of the ring array is reachable and nothing spills
        // past one direction's half of the slot.
        let last = ring_off(u64::MAX, BUS_CH - 1) + MAX_BLOCK;
        assert_eq!(last, RINGS * BUS_CH * MAX_BLOCK);
    }

    #[test]
    fn align_up_snaps_to_the_block_grid() {
        assert_eq!(align_up(0, 128), 0);
        assert_eq!(align_up(1, 128), 128);
        assert_eq!(align_up(128, 128), 128);
        assert_eq!(align_up(5000, 512), 5120);
    }

    #[test]
    fn mapping_holds_every_slot_whole() {
        assert_eq!(MAPPING_SIZE, 128 + SLOTS * std::mem::size_of::<Slot>());
        // ~4 MiB — comfortably a mapping, not something to be shy about.
        const { assert!(MAPPING_SIZE < 8 << 20) };
    }
}
