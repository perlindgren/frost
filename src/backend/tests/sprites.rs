//! Sprite loading and construction: PNG round-trips, sampling filters, buffer generations.

use crate::objects::*;
use std::sync::Arc;

/// The path a test PNG is written to: under the crate's `target/`
/// directory, not the global temp dir. `cargo test` proves `target/`
/// is writable (the build just wrote it), while some machines block or
/// fill `%TEMP%`, which would fail these tests for unrelated reasons.
fn sprite_test_path(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    std::fs::create_dir_all(&dir).expect("the build just wrote to target/");
    dir.join(format!("frost-sprite-{name}-{}.png", std::process::id()))
}

#[test]
fn sprite_loader_round_trips_a_png_file() {
    let path = sprite_test_path("test");
    let buf: Vec<u8> = [
        [255u8, 0, 0, 255],
        [0, 255, 0, 128],
        [0, 0, 255, 0],
        [10, 20, 30, 40],
    ]
    .iter()
    .flat_map(|pixel| pixel.iter().copied())
    .collect();
    image::save_buffer(&path, &buf, 2, 2, image::ColorType::Rgba8).expect("writing the test png");
    let shape = match Shape::sprite(&path) {
        Ok(shape) => shape,
        Err(err) => panic!("failed to load the test png: {err}"),
    };
    let Shape::Sprite {
        data,
        width,
        height,
        color,
        alpha,
        filter,
        generation,
    } = shape
    else {
        panic!("expected a sprite shape");
    };
    assert_eq!((width, height), (2, 2));
    // Every construction stamps a buffer generation (never 0): the
    // backend's texture cache keys on `(pointer, generation)`, so a
    // freed-and-recycled buffer address can never hit a stale texture.
    assert_ne!(generation, 0);
    // The decoded RGBA8 buffer matches the file's pixels byte for byte.
    assert_eq!(&data[..], &buf[..]);
    // The default tint is white and the default opacity is 1.0.
    assert_eq!(
        color,
        Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0
        }
    );
    assert_eq!(alpha, 1.0);
    // The default constructor bilinear-samples the texture.
    assert_eq!(filter, SpriteFilter::Linear);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sprite_constructors_set_their_sampling_filter() {
    // The default constructors bilinear-sample; the `*_nearest` ones
    // nearest-sample.
    let buf = [255u8, 0, 0, 255];
    let path = sprite_test_path("filter");
    image::save_buffer(&path, &buf, 1, 1, image::ColorType::Rgba8).expect("writing the test png");
    let shape = Shape::sprite(&path).expect("a valid png loads");
    let Shape::Sprite { filter, .. } = &shape else {
        panic!("expected a sprite shape");
    };
    assert_eq!(*filter, SpriteFilter::Linear);
    let shape = Shape::sprite_nearest(&path).expect("a valid png loads");
    let Shape::Sprite { filter, .. } = &shape else {
        panic!("expected a sprite shape");
    };
    assert_eq!(*filter, SpriteFilter::Nearest);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sprite_buffers_get_unique_generations() {
    // The backend's texture cache keys on `(pointer, generation)`, and a
    // rebuilt, same-sized buffer is likely to be allocated at a freed
    // buffer's old address: without a fresh generation per construction
    // the cache would hand out the stale texture. So every construction
    // must stamp its own, and a clone (which shares the buffer) must keep
    // it.
    let buf = [10u8, 20, 30, 40];
    let path = sprite_test_path("gen");
    image::save_buffer(&path, &buf, 1, 1, image::ColorType::Rgba8).expect("writing the test png");
    let a = Shape::sprite(&path).expect("a valid png loads");
    let b = Shape::sprite(&path).expect("a valid png loads");
    let Shape::Sprite { generation: ga, .. } = &a else {
        panic!("expected a sprite shape");
    };
    let Shape::Sprite { generation: gb, .. } = &b else {
        panic!("expected a sprite shape");
    };
    assert_ne!(*ga, *gb, "two constructions need distinct generations");
    assert!(*ga >= 1, "a stamped generation is never 0");
    let Shape::Sprite { generation: gc, .. } = &a.clone() else {
        panic!("expected a sprite shape");
    };
    assert_eq!(*ga, *gc, "a clone shares its buffer and its generation");
    // Particle sprite buffers are stamped the same way.
    let p = ParticleShape::sprite(&path).expect("a valid png loads");
    let ParticleShape::Sprite { generation: pg, .. } = &p else {
        panic!("expected a sprite particle shape");
    };
    assert!(*pg >= 1, "a stamped generation is never 0");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cloned_sprites_share_their_pixel_buffer() {
    // The pixels live behind an `Arc`: cloning a sprite shape must not
    // copy the buffer.
    let path = sprite_test_path("share");
    let buf = [255u8, 0, 0, 255, 0, 255, 0, 255];
    image::save_buffer(&path, &buf, 2, 1, image::ColorType::Rgba8).expect("writing the test png");
    let shape = Shape::sprite(&path).unwrap();
    let Shape::Sprite { data: a, .. } = &shape else {
        panic!("expected a sprite shape");
    };
    let Shape::Sprite { data: b, .. } = &shape.clone() else {
        panic!("expected a sprite shape");
    };
    assert!(Arc::ptr_eq(a, b));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sprite_loader_reports_a_missing_file_as_io() {
    let err = Shape::sprite("frost-missing-sprite-file.png").unwrap_err();
    assert!(matches!(
        err,
        SpriteError::Io(err) if err.kind() == std::io::ErrorKind::NotFound
    ));
}
