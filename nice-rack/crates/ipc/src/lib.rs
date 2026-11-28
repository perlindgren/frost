//! Shared-memory audio storage. No native plugin ever receives mapped pointers.
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]
use memmap2::{MmapMut, MmapOptions};
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
    sync::atomic::{AtomicU32, Ordering},
};

const MAGIC: u32 = 0x4e524950;
const VERSION: u32 = 1;
const HEADER: usize = 16;
const REQUEST: usize = 6;
const COMPLETE: usize = 7;
const FRAMES: usize = 8;
const FAILED: usize = 9;
const STOP: usize = 10;
const RESET: usize = 11;
const FLUSH: usize = 12;
const FLUSHED: usize = 13;

/// Immutable, locally validated shape. Never trust a helper-modified header
/// when choosing offsets or lengths in an already opened mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub inputs: usize,
    pub outputs: usize,
    pub max_frames: usize,
}
impl Layout {
    pub fn new(inputs: usize, outputs: usize, max_frames: usize) -> io::Result<Self> {
        if inputs > 64 || outputs == 0 || outputs > 64 || !(1..=65536).contains(&max_frames) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid IPC audio layout",
            ));
        }
        Ok(Self {
            inputs,
            outputs,
            max_frames,
        })
    }
    fn cells(self) -> usize {
        HEADER + (self.inputs + self.outputs) * self.max_frames
    }
    fn bytes(self) -> usize {
        self.cells() * size_of::<AtomicU32>()
    }
}

/// Mapping lifetime owns every atomic reference. Shared accesses, including
/// sample cells, are atomic, even across a timed-out or misbehaving peer.
pub struct AudioMemory {
    map: MmapMut,
    layout: Layout,
    _file: File,
}
impl AudioMemory {
    /// # Safety
    /// The caller must prevent truncation of this file and non-atomic writes
    /// for the lifetime of every mapping, including mappings in peer processes.
    pub unsafe fn create(path: &Path, layout: Layout) -> io::Result<Self> {
        let layout = Layout::new(layout.inputs, layout.outputs, layout.max_frames)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        file.set_len(layout.bytes() as u64)?;
        // SAFETY: a newly created file of the exact checked size is exclusively
        // owned here. The protocol never truncates it; every access is atomic.
        let mut map = unsafe { MmapOptions::new().len(layout.bytes()).map_mut(&file)? };
        for offset in 0..layout.cells() {
            // SAFETY: mmap is page-aligned, each offset is AtomicU32-aligned and
            // within the checked mapping. The file is not yet shared/published.
            unsafe {
                map.as_mut_ptr()
                    .cast::<AtomicU32>()
                    .add(offset)
                    .write(AtomicU32::new(0))
            };
        }
        let memory = Self {
            map,
            layout,
            _file: file,
        };
        for (index, value) in [
            MAGIC,
            VERSION,
            layout.inputs as u32,
            layout.outputs as u32,
            layout.max_frames as u32,
            layout.bytes() as u32,
        ]
        .into_iter()
        .enumerate()
        {
            memory.cell(index).store(value, Ordering::Relaxed);
        }
        Ok(memory)
    }
    /// # Safety
    /// The caller must supply a file initialized by `create`, prevent truncation
    /// and non-atomic writes, and retain these guarantees for all peer mappings.
    pub unsafe fn open(path: &Path, layout: Layout) -> io::Result<Self> {
        let layout = Layout::new(layout.inputs, layout.outputs, layout.max_frames)?;
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        if file.metadata()?.len() != layout.bytes() as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "IPC mapping length mismatch",
            ));
        }
        // SAFETY: the creator initialized the exact checked layout and never
        // truncates it. Only this module accesses it, using atomic operations.
        let map = unsafe { MmapOptions::new().len(layout.bytes()).map_mut(&file)? };
        let memory = Self {
            map,
            layout,
            _file: file,
        };
        let expected = [
            MAGIC,
            VERSION,
            layout.inputs as u32,
            layout.outputs as u32,
            layout.max_frames as u32,
            layout.bytes() as u32,
        ];
        if expected
            .into_iter()
            .enumerate()
            .any(|(i, n)| memory.cell(i).load(Ordering::Relaxed) != n)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "IPC mapping protocol mismatch",
            ));
        }
        Ok(memory)
    }
    fn cell(&self, index: usize) -> &AtomicU32 {
        assert!(index < self.layout.cells());
        // SAFETY: checked bound and 4-byte cell stride, page-aligned mapping,
        // initialized atomic objects, map retained for the borrow's lifetime.
        unsafe { &*self.map.as_ptr().cast::<AtomicU32>().add(index) }
    }
    pub fn layout(&self) -> Layout {
        self.layout
    }
    pub fn requested(&self) -> u32 {
        self.cell(REQUEST).load(Ordering::Acquire)
    }
    pub fn completed(&self) -> u32 {
        self.cell(COMPLETE).load(Ordering::Acquire)
    }
    pub fn publish(&self, sequence: u32, frames: usize) {
        assert!(sequence != 0 && (1..=self.layout.max_frames).contains(&frames));
        self.cell(FRAMES).store(frames as u32, Ordering::Relaxed);
        self.cell(REQUEST).store(sequence, Ordering::Release);
    }
    pub fn frames(&self) -> usize {
        (self.cell(FRAMES).load(Ordering::Relaxed) as usize).min(self.layout.max_frames)
    }
    pub fn finish(&self, sequence: u32) {
        self.cell(COMPLETE).store(sequence, Ordering::Release);
    }
    pub fn fail(&self) {
        self.cell(FAILED).store(1, Ordering::Release);
    }
    pub fn failed(&self) -> bool {
        self.cell(FAILED).load(Ordering::Acquire) != 0
    }
    pub fn request_flush(&self) -> u32 {
        self.cell(FLUSH)
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }
    pub fn flush_requested(&self) -> u32 {
        self.cell(FLUSH).load(Ordering::Acquire)
    }
    pub fn flush_completed(&self) -> u32 {
        self.cell(FLUSHED).load(Ordering::Acquire)
    }
    pub fn flush_reached(&self, request: u32) -> bool {
        self.flush_completed().wrapping_sub(request) < (1u32 << 31)
    }
    pub fn finish_flush(&self, request: u32) {
        self.cell(FLUSHED).store(request, Ordering::Release);
    }
    pub fn request_reset(&self) {
        self.cell(RESET).store(1, Ordering::Release);
    }
    pub fn take_reset(&self) -> bool {
        self.cell(RESET).swap(0, Ordering::AcqRel) != 0
    }
    pub fn stop(&self) {
        self.cell(STOP).store(1, Ordering::Release);
    }
    pub fn stopping(&self) -> bool {
        self.cell(STOP).load(Ordering::Acquire) != 0
    }
    pub fn write_input(&self, channel: usize, samples: &[f32]) {
        self.write(false, channel, samples);
    }
    pub fn read_input(&self, channel: usize, samples: &mut [f32]) {
        self.read(false, channel, samples);
    }
    pub fn write_output(&self, channel: usize, samples: &[f32]) {
        self.write(true, channel, samples);
    }
    pub fn read_output(&self, channel: usize, samples: &mut [f32]) {
        self.read(true, channel, samples);
    }
    fn offset(&self, output: bool, channel: usize, frames: usize) -> usize {
        assert!(frames <= self.layout.max_frames);
        assert!(
            channel
                < if output {
                    self.layout.outputs
                } else {
                    self.layout.inputs
                }
        );
        HEADER + (if output { self.layout.inputs } else { 0 } + channel) * self.layout.max_frames
    }
    fn write(&self, output: bool, channel: usize, samples: &[f32]) {
        let offset = self.offset(output, channel, samples.len());
        for (i, sample) in samples.iter().enumerate() {
            self.cell(offset + i)
                .store(sample.to_bits(), Ordering::Relaxed);
        }
    }
    fn read(&self, output: bool, channel: usize, samples: &mut [f32]) {
        let offset = self.offset(output, channel, samples.len());
        for (i, sample) in samples.iter_mut().enumerate() {
            *sample = f32::from_bits(self.cell(offset + i).load(Ordering::Relaxed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            Self(std::env::temp_dir().join(format!(
                "rack-map-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    #[test]
    fn distinct_mappings_exchange_samples_and_validate_shape() {
        let fixture = Fixture::new();
        let layout = Layout::new(1, 2, 64).unwrap();
        // SAFETY: the test exclusively owns the file, keeps both mappings alive,
        // and never truncates it or writes through anything except these APIs.
        let a = unsafe { AudioMemory::create(&fixture.0, layout) }.unwrap();
        // SAFETY: same ownership and atomic-only protocol as the creator above.
        let b = unsafe { AudioMemory::open(&fixture.0, layout) }.unwrap();
        a.write_input(0, &[0.25, -0.5]);
        a.publish(1, 2);
        assert_eq!(b.requested(), 1);
        assert_eq!(b.frames(), 2);
        let mut samples = [0.; 2];
        b.read_input(0, &mut samples);
        assert_eq!(samples, [0.25, -0.5]);
        b.write_output(1, &samples);
        b.finish(1);
        assert_eq!(a.completed(), 1);
        a.read_output(1, &mut samples);
        assert_eq!(samples, [0.25, -0.5]);
        a.request_reset();
        assert!(b.take_reset());
        assert!(!b.take_reset());
        a.fail();
        b.finish(2);
        assert!(b.failed());
        assert!(a.failed(), "late completion cannot clear poison");
        b.stop();
        assert!(a.stopping());
        // SAFETY: the file remains initialized and unmodified; only shape differs.
        assert!(unsafe { AudioMemory::open(&fixture.0, Layout::new(1, 1, 64).unwrap()) }.is_err());
    }
    #[test]
    fn layout_checks_prevent_overflow_and_empty_output() {
        for shape in [
            (1, 0, 64),
            (usize::MAX, 1, 64),
            (1, 1, usize::MAX),
            (1, 1, 0),
        ] {
            assert!(Layout::new(shape.0, shape.1, shape.2).is_err());
        }
    }
}
