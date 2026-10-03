#[test]
fn means() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/dogs_name/assets/sprites/Monster1.png");
    let img = image::ImageReader::open(&path)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap()
        .to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut bands = [(0u64, 0u64, 0u64, 0u64); 6];
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x as u32, y as u32).0;
            if p[3] < 128 {
                continue;
            }
            let i = (x * 6 / w).min(5);
            let b = &mut bands[i];
            b.0 += p[0] as u64;
            b.1 += p[1] as u64;
            b.2 += p[2] as u64;
            b.3 += 1;
        }
    }
    for (i, band) in bands.iter().enumerate() {
        let (r64, g64, b64, n) = *band;
        if n == 0 {
            println!("band {i}: empty");
            continue;
        }
        let (r, g, b) = (r64 / n, g64 / n, b64 / n);
        println!(
            "band {} (x {:>4}-{:>4}): #{:02X}{:02X}{:02X} over {} px",
            i,
            i * w / 6,
            (i + 1) * w / 6,
            r,
            g,
            b,
            n
        );
    }
}
