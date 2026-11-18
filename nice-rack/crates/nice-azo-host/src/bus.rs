//! Typed views over the shared mapping: the header, the slots, the two
//! block rings of a slot, and the election. Everything that touches
//! bytes another process writes lives behind an atomics-only `&` or a
//! documented raw pointer; the rings themselves are plain `f32` arrays
//! whose race-freedom comes from the sequence protocol, not from `Cell`
//! bookkeeping that would be a lie across processes anyway.

use std::io;
use std::sync::Arc;

use crate::proto::{
    self, ELECTION_NAME, Header, MAPPING_NAME, MAPPING_SIZE, SLOT_ACTIVE, SLOT_FREE, Slot, ring_off,
};
use crate::win::{Event, Mapping, MasterClaim};

/// Which half of a slot's traffic: master→client inputs, client→master
/// mix contributions.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    In,
    Out,
}

const _: () = assert!(std::mem::offset_of!(Slot, in_ring) < std::mem::offset_of!(Slot, out_ring));

pub struct Bus {
    mapping: Mapping,
    /// Created alongside the mapping: `CreateEventA` on an existing name
    /// is a cheap handle duplicate, so both sides can create all eight
    /// at acquire time and never think about who got there first. Only
    /// the input direction needs one — the master consumes outputs on
    /// its own clock, so there is nothing for it to be woken for.
    events: Vec<Event>,
}

unsafe impl Send for Bus {}
unsafe impl Sync for Bus {}

pub type SharedBus = Arc<Bus>;

impl Bus {
    /// Attaches to the bus, creating it if this is the first instance on
    /// the machine. A mapping that exists but disagrees on magic or
    /// version is a hard error rather than a surprise: silently mixing
    /// protocol generations is how a driver ends up writing audio into
    /// somebody else's sample counts.
    pub fn acquire() -> io::Result<SharedBus> {
        let mapping = Mapping::open_or_create(MAPPING_NAME, MAPPING_SIZE)?;
        let header = unsafe { &*(mapping.base() as *const Header) };
        if mapping.created() {
            unsafe {
                let h = &mut *(mapping.base() as *mut Header);
                h.magic
                    .store(proto::MAGIC, std::sync::atomic::Ordering::Relaxed);
                h.version
                    .store(proto::VERSION, std::sync::atomic::Ordering::Relaxed);
                h.epoch.store(1, std::sync::atomic::Ordering::Relaxed);
                h.master_pid.store(0, std::sync::atomic::Ordering::Relaxed);
                h.rate_hz.store(0, std::sync::atomic::Ordering::Relaxed);
                h.hw_in.store(0, std::sync::atomic::Ordering::Relaxed);
                h.hw_out.store(0, std::sync::atomic::Ordering::Relaxed);
                h.virt_ch
                    .store(proto::VIRT_CH as u32, std::sync::atomic::Ordering::Relaxed);
                h.bus_ch
                    .store(proto::BUS_CH as u32, std::sync::atomic::Ordering::Relaxed);
                h.max_block.store(
                    proto::MAX_BLOCK as u32,
                    std::sync::atomic::Ordering::Relaxed,
                );
                h.quantum.store(
                    proto::DEFAULT_QUANTUM as u32,
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
        } else {
            use std::sync::atomic::Ordering::Relaxed;
            if header.magic.load(Relaxed) != proto::MAGIC
                || header.version.load(Relaxed) != proto::VERSION
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "NiceRack ASIO bus exists at an incompatible version",
                ));
            }
        }
        let mut events = Vec::with_capacity(proto::SLOTS);
        for i in 0..proto::SLOTS {
            events.push(Event::named(&proto::slot_event_in(i))?);
        }
        Ok(Arc::new(Bus { mapping, events }))
    }

    pub fn header(&self) -> &Header {
        unsafe { &*(self.mapping.base() as *const Header) }
    }

    pub fn slot(&self, index: usize) -> &Slot {
        assert!(index < proto::SLOTS);
        unsafe { &*(self.slot_ptr(index) as *const Slot) }
    }

    fn slot_ptr(&self, index: usize) -> *mut Slot {
        unsafe {
            self.mapping
                .base()
                .add(std::mem::size_of::<Header>() + index * std::mem::size_of::<Slot>())
                as *mut Slot
        }
    }

    /// First frame of block `n`, channel `ch`, of one slot's ring. Safe
    /// to *hand out*; legality of concurrent access is the caller's
    /// contract with the sequence protocol (SPSC, ring-depth gated).
    pub fn ring_block(&self, slot: usize, dir: Dir, n: u64, ch: usize) -> *mut f32 {
        assert!(slot < proto::SLOTS && ch < proto::BUS_CH);
        let field_off = match dir {
            Dir::In => std::mem::offset_of!(Slot, in_ring),
            Dir::Out => std::mem::offset_of!(Slot, out_ring),
        };
        unsafe {
            (self.slot_ptr(slot) as *mut u8)
                .add(field_off + ring_off(n, ch) * std::mem::size_of::<f32>())
                as *mut f32
        }
    }

    /// Producer-side view: `MAX_BLOCK` writable frames. Writing past the
    /// sequence the consumer has acknowledged is the one true bug this
    /// API cannot prevent; every caller gates on `proto::RINGS` first.
    ///
    /// # Safety
    /// Only the ring's producer may call this for a given block, and
    /// only after the depth gate has been checked.
    // The mapping is interior-mutable shared memory by nature; handing
    // out `&mut` views through `&self` is the whole point of the type.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn ring_block_mut(&self, slot: usize, dir: Dir, n: u64, ch: usize) -> &mut [f32] {
        unsafe {
            std::slice::from_raw_parts_mut(self.ring_block(slot, dir, n, ch), proto::MAX_BLOCK)
        }
    }

    /// The whole channel-major block, for the memcpy in and out paths.
    ///
    /// # Safety: as [`Self::ring_block_mut`].
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn in_block_mut(&self, slot: usize, n: u64, ch: usize) -> &mut [f32] {
        unsafe { self.ring_block_mut(slot, Dir::In, n, ch) }
    }

    pub fn signal_in(&self, slot: usize) {
        self.events[slot].signal();
    }

    pub fn wait_in(&self, slot: usize, ms: u32) -> bool {
        self.events[slot].wait(ms)
    }

    /// Takes the first free slot. The compare-exchange makes this safe
    /// against any number of instances in any number of processes.
    pub fn claim_slot(&self) -> Option<usize> {
        use std::sync::atomic::Ordering::SeqCst;
        (0..proto::SLOTS).find(|&i| {
            self.slot(i)
                .state
                .compare_exchange(SLOT_FREE, SLOT_ACTIVE, SeqCst, SeqCst)
                .is_ok()
        })
    }

    /// Frees a slot and scrubs its bookkeeping, so the next tenant — in
    /// this session or the next — sees zeroed sequences without the
    /// master having to remember which numbers the old one reached.
    pub fn release_slot(&self, index: usize) {
        use std::sync::atomic::Ordering::SeqCst;
        let slot = self.slot(index);
        slot.in_seq.store(0, SeqCst);
        slot.out_seq.store(0, SeqCst);
        slot.consumed_out.store(0, SeqCst);
        slot.read_in.store(0, SeqCst);
        slot.start_frame.store(0, SeqCst);
        slot.block.store(0, SeqCst);
        slot.state.store(SLOT_FREE, SeqCst);
    }

    pub fn try_become_master(&self) -> Option<MasterClaim> {
        MasterClaim::try_elect(ELECTION_NAME)
    }
}
