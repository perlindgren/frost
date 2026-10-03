//! Headless read-check of the `dogs_name` flipbook art: loads the four
//! `Monster*.png` frames through the same public entry point the example
//! uses (`frost::Shape::sprite`) and reports what came back. No window, no
//! device — `Shape::sprite` is `fs::read` plus a PNG decode.

use frost::{Shape, SpriteFilter};
use std::sync::Arc;

/// The folder the example loads from.
fn frame_path(index: u32) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/dogs_name/assets/sprites")
        .join(format!("Monster{index}.png"))
}

/// Everything one decoded frame tells us.
struct Report {
    index: u32,
    width: u32,
    height: u32,
    bytes: usize,
    opaque: usize,
    transparent: usize,
    partial: usize,
    mean_rgb: [f64; 3],
    checksum: u64,
    decode_ms: u128,
}

fn checksum(data: &[u8]) -> u64 {
    // FNV-1a: enough to tell two frames apart, no dependency needed.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn load(index: u32) -> Report {
    let path = frame_path(index);
    let started = std::time::Instant::now();
    let shape = Shape::sprite(&path)
        .unwrap_or_else(|err| panic!("failed to load {}: {err}", path.display()));
    let decode_ms = started.elapsed().as_millis();

    let Shape::Sprite {
        data,
        width,
        height,
        color,
        alpha,
        filter,
        generation,
    } = &shape
    else {
        panic!("frame {index}: expected a sprite shape");
    };

    // The invariants the renderer and the texture cache rely on.
    assert_eq!(
        data.len(),
        *width as usize * *height as usize * 4,
        "frame {index}: buffer is not width*height*4"
    );
    assert_ne!(*generation, 0, "frame {index}: generation must never be 0");
    assert_eq!(*filter, SpriteFilter::Linear, "frame {index}: filter");
    assert_eq!(*alpha, 1.0, "frame {index}: default opacity");
    assert_eq!(
        (color.r, color.g, color.b, color.a),
        (1.0, 1.0, 1.0, 1.0),
        "frame {index}: default tint"
    );
    assert_eq!(
        shape.sprite_size(),
        Some([*width as f32, *height as f32]),
        "frame {index}: sprite_size disagrees with the texture"
    );

    let mut opaque = 0;
    let mut transparent = 0;
    let mut partial = 0;
    let mut sums = [0u64; 3];
    for pixel in data.as_chunks::<4>().0 {
        match pixel[3] {
            0 => transparent += 1,
            255 => opaque += 1,
            _ => partial += 1,
        }
        for (channel, sum) in pixel[..3].iter().zip(sums.iter_mut()) {
            *sum += *channel as u64;
        }
    }
    let pixels = (data.len() / 4) as f64;
    let mean_rgb = sums.map(|sum| sum as f64 / pixels);

    Report {
        index,
        width: *width,
        height: *height,
        bytes: data.len(),
        opaque,
        transparent,
        partial,
        mean_rgb,
        checksum: checksum(data),
        decode_ms,
    }
}

#[test]
fn monster_frames_decode_and_match_their_png_headers() {
    let mut reports = Vec::new();
    for index in 1..=4 {
        let report = load(index);
        // What `file` says about the file must be what the decode produced.
        assert_eq!((report.width, report.height), (1920, 1080), "frame {index}");
        // Art, not a blank: at least one texel carries some coverage.
        assert!(
            report.opaque + report.partial > 0,
            "frame {} decoded fully transparent",
            report.index
        );
        println!(
            "Monster{index}.png: {}x{}, {} MiB RGBA8, decoded in {} ms | opaque {:.1}% \
             partial {:.1}% clear {:.1}% | mean rgb {:>3.0},{:>3.0},{:>3.0} | fnv {:016x}",
            report.width,
            report.height,
            report.bytes / (1024 * 1024),
            report.decode_ms,
            100.0 * report.opaque as f64 / (report.bytes / 4) as f64,
            100.0 * report.partial as f64 / (report.bytes / 4) as f64,
            100.0 * report.transparent as f64 / (report.bytes / 4) as f64,
            report.mean_rgb[0],
            report.mean_rgb[1],
            report.mean_rgb[2],
            report.checksum,
        );
        reports.push(report);
    }

    // Every frame shares its geometry, or the flipbook jumps between sizes.
    let first = (reports[0].width, reports[0].height);
    for report in &reports {
        assert_eq!(
            (report.width, report.height),
            first,
            "frame {}",
            report.index
        );
    }

    // Report duplicate frames: the animation only reads as four frames if
    // the four buffers differ.
    let mut duplicates = Vec::new();
    for (i, a) in reports.iter().enumerate() {
        for b in reports.iter().skip(i + 1) {
            if a.checksum == b.checksum {
                duplicates.push((a.index, b.index));
            }
        }
    }
    for (a, b) in &duplicates {
        println!("frames {a} and {b} are pixel-identical");
    }
    println!(
        "{} distinct frame(s) of {}",
        reports.len() - duplicates.len(),
        reports.len()
    );

    // The fit scale the example computes from its window, which opens at
    // a physical 1920x1080 on every display (`Config::window_size_px`).
    let fit = (1920.0f32 / first.0 as f32).min(1080.0 / first.1 as f32);
    println!("fit scale into a physical 1920x1080 window: {fit}");
}

#[test]
fn monster_frames_survive_the_arc_clone_the_flipbook_does() {
    // The example clones a `Shape` per frame tick; the pixels must stay
    // shared and intact across the clone.
    let original = Shape::sprite(frame_path(2)).expect("Monster2.png");
    let clone = original.clone();
    let Shape::Sprite { data: a, .. } = &original else {
        unreachable!()
    };
    let Shape::Sprite { data: b, .. } = &clone else {
        unreachable!()
    };
    assert!(
        Arc::ptr_eq(a, b),
        "cloning a sprite shape copied its pixel buffer"
    );
    assert_eq!(checksum(a), checksum(b));
}
