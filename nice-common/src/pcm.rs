//! The sample formats a driver's buffers can actually hold.
//!
//! A device API that names its own format gets to decide what a buffer contains;
//! one that asks the device does not. ASIO is the second kind: `ASIOGetChannelInfo`
//! answers a sample type per channel and the host writes those bytes or hears
//! noise. cpal hid that behind `SizedSample` and its `cpal::SampleFormat` list,
//! which is why a Windows host that wanted to ask had to carry cpal to do it.
//! azo asks per channel and reports the answer — `asio_io` makes one
//! `channelInfo` call per opened channel and maps its type code here — so what
//! is left of that job is the arithmetic, with no dependency attached to it and
//! nothing platform about it. That is the part worth a witness, and a gate that
//! runs on Linux should not be blind to it.
//!
//! Everything here converts between interleaved `f32` — the format frost's rings,
//! monitors and plugins speak — and one channel's run of native bytes. The byte
//! order is part of the format: ASIO's `…MSB` types are big-endian and its `…LSB`
//! types are little-endian whatever the machine under them is, so both spellings
//! are named rather than inferred.

/// One frame's worth of one channel, as the driver writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pcm {
    /// Two's-complement, 16 bits, full width.
    Int16 { big_endian: bool },
    /// Two's-complement in three bytes, which is why it needs its own case:
    /// ASIO really does answer this one.
    Int24 { big_endian: bool },
    /// Two's-complement, 32 bits, full width.
    Int32 { big_endian: bool },
    /// `bits` of two's-complement, most-significant-aligned inside a 32-bit
    /// container — ASIO's `Int32LSB16`, `Int32LSB18`, `Int32LSB20`, `Int32LSB24`
    /// family and their big-endian twins. `bits` is 16, 18, 20 or 24; the
    /// container is always four bytes wide, which is the part a host gets wrong
    /// by reading it as a plain `Int16` or `Int32`.
    Padded32 { bits: u32, big_endian: bool },
    /// IEEE 754 binary32.
    Float32 { big_endian: bool },
    /// IEEE 754 binary64.
    Float64 { big_endian: bool },
}

/// Integer full scale used in both directions: the largest value the format can
/// hold, so `1.0` is exactly representable and nothing overflows. Symmetric
/// conversion costs the true negative full scale one least-significant bit — a
/// `-1.0` sample lands on `-32767` rather than `-32768` in a 16-bit format,
/// which is `-0.0000305` off — and buys the guarantee that no input, however
/// far over it is, can wrap around into a opposite-signed full-scale pop.
fn full_scale(bits: u32) -> f32 {
    ((1i64 << (bits - 1)) - 1) as f64 as f32
}

/// Width in bytes of one frame, which is the container's width and not the
/// data's: the padded types carry 16 to 24 bits of sound in four bytes.
impl Pcm {
    /// Bytes per frame for this format.
    pub fn bytes_per_frame(self) -> usize {
        match self {
            Pcm::Int16 { .. } => 2,
            Pcm::Int24 { .. } => 3,
            Pcm::Int32 { .. } | Pcm::Padded32 { .. } | Pcm::Float32 { .. } => 4,
            Pcm::Float64 { .. } => 8,
        }
    }

    /// Bytes one frame of `frames` frames needs.
    pub fn block_bytes(self, frames: usize) -> usize {
        self.bytes_per_frame() * frames
    }

    /// Read frames out of one channel's buffer into `to`, stopping at whichever
    /// of the two runs out first, and report how many frames that was. No format
    /// here can fail: a byte pattern is a sample, however much the driver meant
    /// it.
    pub fn read(self, from: &[u8], to: &mut [f32]) -> usize {
        let width = self.bytes_per_frame();
        let frames = to.len().min(from.len() / width);
        for index in 0..frames {
            let source = &from[index * width..][..width];
            to[index] = match self {
                Pcm::Int16 { big_endian } => {
                    word(source, big_endian) as i16 as f32 / full_scale(16)
                }
                Pcm::Int24 { big_endian } => {
                    // Three bytes of a 24-bit two's-complement value, which
                    // `word` lays at the bottom of its `u32`: shift the sign bit
                    // up to bit 31 so the `i32` cast sees it, then shift back
                    // arithmetically so the 23 bits under it stay. Skipping the
                    // second shift is the classic mistake, and it reads `-1` as
                    // `-256`.
                    let unsigned = word(source, big_endian);
                    (((unsigned << 8) as i32) >> 8) as f32 / full_scale(24)
                }
                Pcm::Int32 { big_endian } => {
                    word(source, big_endian) as i32 as f32 / full_scale(32)
                }
                Pcm::Padded32 { bits, big_endian } => {
                    let shifted = (word(source, big_endian) as i32) >> (32 - bits);
                    shifted as f32 / full_scale(bits)
                }
                Pcm::Float32 { big_endian } => {
                    let bytes: [u8; 4] = source.try_into().expect("width is 4");
                    if big_endian {
                        f32::from_be_bytes(bytes)
                    } else {
                        f32::from_le_bytes(bytes)
                    }
                }
                Pcm::Float64 { big_endian } => {
                    let bytes: [u8; 8] = source.try_into().expect("width is 8");
                    let wide = if big_endian {
                        f64::from_be_bytes(bytes)
                    } else {
                        f64::from_le_bytes(bytes)
                    };
                    wide as f32
                }
            };
        }
        frames
    }

    /// Write frames into one channel's buffer, clamping to the format, and report
    /// how many frames that was. Clamping is deliberate: a buffer of samples over
    /// full scale is a thing that happens — a plugin with the gain up, a monitor
    /// fed a hot input — and the alternative is that a 1.4 turns into a big
    /// negative number and the listener hears a crack instead of a loud sound.
    pub fn write(self, from: &[f32], to: &mut [u8]) -> usize {
        let width = self.bytes_per_frame();
        let frames = from.len().min(to.len() / width);
        for index in 0..frames {
            let target = &mut to[index * width..][..width];
            let sample = from[index].clamp(-1.0, 1.0);
            match self {
                Pcm::Int16 { big_endian } => put(
                    target,
                    (sample * full_scale(16)).round() as i16 as u16 as u32,
                    2,
                    big_endian,
                ),
                Pcm::Int24 { big_endian } => put(
                    target,
                    (sample * full_scale(24)).round() as i32 as u32,
                    3,
                    big_endian,
                ),
                Pcm::Int32 { big_endian } => put(
                    target,
                    (sample * full_scale(32)).round() as i32 as u32,
                    4,
                    big_endian,
                ),
                Pcm::Padded32 { bits, big_endian } => {
                    let value = (sample * full_scale(bits)).round() as i32;
                    put(target, (value as u32) << (32 - bits), 4, big_endian);
                }
                Pcm::Float32 { big_endian } => {
                    let bytes = if big_endian {
                        sample.to_be_bytes()
                    } else {
                        sample.to_le_bytes()
                    };
                    target.copy_from_slice(&bytes);
                }
                Pcm::Float64 { big_endian } => {
                    let bytes = if big_endian {
                        (sample as f64).to_be_bytes()
                    } else {
                        (sample as f64).to_le_bytes()
                    };
                    target.copy_from_slice(&bytes);
                }
            };
        }
        frames
    }

    /// The name a driver's own documentation uses, for the messages a user reads.
    pub fn name(self) -> &'static str {
        match self {
            Pcm::Int16 { big_endian: false } => "Int16LSB",
            Pcm::Int16 { big_endian: true } => "Int16MSB",
            Pcm::Int24 { big_endian: false } => "Int24LSB",
            Pcm::Int24 { big_endian: true } => "Int24MSB",
            Pcm::Int32 { big_endian: false } => "Int32LSB",
            Pcm::Int32 { big_endian: true } => "Int32MSB",
            Pcm::Padded32 {
                bits: 16,
                big_endian: false,
            } => "Int32LSB16",
            Pcm::Padded32 {
                bits: 16,
                big_endian: true,
            } => "Int32MSB16",
            Pcm::Padded32 {
                bits: 18,
                big_endian: false,
            } => "Int32LSB18",
            Pcm::Padded32 {
                bits: 18,
                big_endian: true,
            } => "Int32MSB18",
            Pcm::Padded32 {
                bits: 20,
                big_endian: false,
            } => "Int32LSB20",
            Pcm::Padded32 {
                bits: 20,
                big_endian: true,
            } => "Int32MSB20",
            Pcm::Padded32 {
                bits: 24,
                big_endian: false,
            } => "Int32LSB24",
            Pcm::Padded32 {
                bits: 24,
                big_endian: true,
            } => "Int32MSB24",
            Pcm::Padded32 { .. } => "Int32 padded",
            Pcm::Float32 { big_endian: false } => "Float32LSB",
            Pcm::Float32 { big_endian: true } => "Float32MSB",
            Pcm::Float64 { big_endian: false } => "Float64LSB",
            Pcm::Float64 { big_endian: true } => "Float64MSB",
        }
    }
}

/// Up to four bytes, taken in the order the format says they are significant.
fn word(bytes: &[u8], big_endian: bool) -> u32 {
    let mut value = 0u32;
    for (offset, byte) in bytes.iter().enumerate() {
        let shift = if big_endian {
            8 * (bytes.len() - 1 - offset)
        } else {
            8 * offset
        };
        value |= (*byte as u32) << shift;
    }
    value
}

/// The low `width` bytes of `value`, laid down in the order the format wants.
/// The bits above `width * 8` are dropped, which is what a two's-complement
/// narrowing means and is why the caller clamps before it gets here.
fn put(bytes: &mut [u8], value: u32, width: usize, big_endian: bool) {
    for (offset, byte) in bytes.iter_mut().enumerate().take(width) {
        let shift = if big_endian {
            8 * (width - 1 - offset)
        } else {
            8 * offset
        };
        *byte = (value >> shift) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A format is a promise about bytes, so the test asks about bytes and not
    /// about round trips alone: a converter that is wrong in the same direction
    /// twice round-trips perfectly.
    #[test]
    fn each_integer_format_names_the_bytes_the_driver_will_see() {
        let mut buffer = [0u8; 8];
        Pcm::Int16 { big_endian: false }.write(&[-1.0], &mut buffer[..2]);
        // The negative side of a symmetric scale is one least-significant bit
        // short of the format's own bottom, which is the promise
        // `full_scale` makes: `-1.0` is `-32767` here, not `-32768`.
        assert_eq!(&buffer[..2], &(-(full_scale(16) as i16)).to_le_bytes());

        Pcm::Int32 { big_endian: true }.write(&[1.0], &mut buffer[..4]);
        // 2^31 - 1 does not fit in an `f32`, so the scale rounds to 2^31 and the
        // cast saturates onto the top of the range: `1.0` writes `i32::MAX`.
        assert_eq!(&buffer[..4], &i32::MAX.to_be_bytes());

        // Three bytes, most significant first, for a value whose sign bit is set.
        Pcm::Int24 { big_endian: true }.write(&[-0.5], &mut buffer[..3]);
        let bytes = (full_scale(24) * -0.5).round() as i32;
        assert_eq!(&buffer[..3], &bytes.to_be_bytes()[1..]);

        // Three bytes little-endian at full negative swing, which is the case
        // `read` has to sign-extend rather than just relocate: the loudest
        // negative sample going out as `01 00 80` — that is `0x800001`, `-8388607`
        // in 24-bit two's complement — and coming back as `-1.0` is what pins
        // both halves of that arithmetic.
        let le24 = Pcm::Int24 { big_endian: false };
        le24.write(&[-1.0], &mut buffer[..3]);
        assert_eq!(&buffer[..3], &[0x01, 0x00, 0x80]);
        let mut back = [0f32; 1];
        assert_eq!(le24.read(&buffer[..3], &mut back), 1);
        assert_eq!(back[0], -1.0);

        // Silence is zero in all of them, which is the one thing a wrong
        // scaling still gets right and so is worth pinning on its own.
        for format in [
            Pcm::Int16 { big_endian: false },
            Pcm::Int24 { big_endian: true },
            Pcm::Int32 { big_endian: true },
            Pcm::Padded32 {
                bits: 18,
                big_endian: false,
            },
            Pcm::Float32 { big_endian: true },
            Pcm::Float64 { big_endian: false },
        ] {
            let mut buffer = [7u8; 8];
            format.write(&[0.0], &mut buffer[..format.bytes_per_frame()]);
            assert_eq!(
                buffer[..format.bytes_per_frame()],
                [0; 8][..format.bytes_per_frame()],
                "{:?} does not write silence as zero",
                format
            );
        }
    }

    /// The padded family is where a host reads the wrong width: four bytes of
    /// container, `bits` of sound, aligned to the top.
    #[test]
    fn a_padded_format_fills_the_top_of_its_container() {
        for bits in [16, 18, 20, 24] {
            let format = Pcm::Padded32 {
                bits,
                big_endian: false,
            };
            let mut buffer = [0u8; 4];
            format.write(&[0.5], &mut buffer);
            let word = i32::from_le_bytes(buffer);
            assert_eq!(
                word >> (32 - bits),
                (full_scale(bits) * 0.5).round() as i32,
                "{bits} bits should sit at the top of the word"
            );

            let mut back = [0f32; 1];
            assert_eq!(format.read(&buffer, &mut back), 1);
            assert!((back[0] - 0.5).abs() < 1.0 / full_scale(bits));
        }
    }

    /// Every format, both byte orders, over a ramp: the shape survives, and no
    /// step of the ramp lands on the wrong side of zero.
    #[test]
    fn every_format_carries_a_ramp_without_losing_its_order() {
        let formats = [
            Pcm::Int16 { big_endian: false },
            Pcm::Int16 { big_endian: true },
            Pcm::Int24 { big_endian: false },
            Pcm::Int24 { big_endian: true },
            Pcm::Int32 { big_endian: false },
            Pcm::Int32 { big_endian: true },
            Pcm::Padded32 {
                bits: 16,
                big_endian: false,
            },
            Pcm::Padded32 {
                bits: 24,
                big_endian: true,
            },
            Pcm::Float32 { big_endian: false },
            Pcm::Float32 { big_endian: true },
            Pcm::Float64 { big_endian: false },
            Pcm::Float64 { big_endian: true },
        ];
        for format in formats {
            let ramp: Vec<f32> = (0..65).map(|n| n as f32 / 64.0 * 2.0 - 1.0).collect();
            let mut bytes = vec![0u8; format.block_bytes(ramp.len())];
            assert_eq!(format.write(&ramp, &mut bytes), ramp.len());
            let mut back = vec![0f32; ramp.len()];
            assert_eq!(format.read(&bytes, &mut back), ramp.len());
            let step = 2.0 / full_scale(16);
            for (index, (want, got)) in ramp.iter().zip(&back).enumerate() {
                assert!(
                    (want - got).abs() <= step,
                    "{format:?} frame {index}: wrote {want}, read {got}"
                );
            }
            assert!(back[32].abs() < step, "{format:?} did not put zero at zero");
            assert!(back[33] > 0.0, "{format:?} lost the sign near zero");
        }
    }

    /// The reason the write path clamps: this is what a hot plugin looks like,
    /// and wrapping would be a click at every one of them.
    #[test]
    fn a_sample_over_full_scale_stays_on_its_own_side() {
        let hot = [8.0f32, -3.0, 1.0, f32::MAX];
        for format in [
            Pcm::Int16 { big_endian: false },
            Pcm::Int24 { big_endian: true },
            Pcm::Int32 { big_endian: false },
            Pcm::Padded32 {
                bits: 20,
                big_endian: true,
            },
        ] {
            let mut bytes = vec![0u8; format.block_bytes(hot.len())];
            format.write(&hot, &mut bytes);
            let mut back = vec![0f32; hot.len()];
            format.read(&bytes, &mut back);
            assert!(back[0] > 0.99, "{format:?} turned a loud sample quiet");
            assert!(
                back[1] < -0.99,
                "{format:?} turned a loud sample loud and positive"
            );
            assert!(back[2] > 0.99, "{format:?} lost full scale");
        }
    }

    /// A short buffer is a real thing — a caller that stops early, a driver whose
    /// last block is partial — and must read as far as it can rather than panic.
    #[test]
    fn a_short_run_stops_at_the_end_of_whichever_side_is_shorter() {
        let format = Pcm::Int32 { big_endian: false };
        let mut bytes = [0u8; 12];
        assert_eq!(format.write(&[0.25; 10], &mut bytes), 3);
        let mut back = [0f32; 10];
        assert_eq!(format.read(&bytes, &mut back), 3);
        assert_eq!(back[2], 0.25);
        // Two bytes of a four-byte frame is half a sample, not a sample.
        assert_eq!(format.read(&bytes[..6], &mut back), 1);
    }
}
