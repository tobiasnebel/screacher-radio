use std::{env, fs, io::BufReader, path::PathBuf};

const ICON: &str = "media/icons/screacher-radio-icon-512.png";

fn main() {
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-env-changed=SCREACHER_RELEASE_TAG");
    println!("cargo:rerun-if-env-changed=SCREACHER_COMMIT_SHA");

    let version = env::var("SCREACHER_RELEASE_TAG")
        .ok()
        .filter(|tag| !tag.is_empty())
        .unwrap_or_else(|| {
            let mut version = format!("{}-snapshot", env!("CARGO_PKG_VERSION"));
            if let Ok(sha) = env::var("SCREACHER_COMMIT_SHA")
                && let Some(short_sha) = sha.get(..7)
            {
                version.push('+');
                version.push_str(short_sha);
            }
            version
        });
    println!("cargo:rustc-env=SCREACHER_VERSION={version}");

    let input = fs::File::open(ICON).expect("open application icon PNG");
    let mut reader = png::Decoder::new(BufReader::new(input))
        .read_info()
        .expect("read application icon PNG header");
    let mut rgba = vec![0; reader.output_buffer_size().expect("icon buffer size")];
    let info = reader
        .next_frame(&mut rgba)
        .expect("decode application icon");
    assert_eq!(info.color_type, png::ColorType::Rgba, "icon must be RGBA");
    assert_eq!(info.bit_depth, png::BitDepth::Eight, "icon must be 8-bit");
    assert_eq!(info.width, info.height, "icon must be square");

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    for size in [32, 64] {
        let pixels = downscale(&rgba, info.width as usize, size);
        fs::write(output.join(format!("icon-{size}.rgba")), pixels).expect("write RGBA icon");
    }
}

fn downscale(source: &[u8], width: usize, size: usize) -> Vec<u8> {
    assert!(
        width.is_multiple_of(size),
        "icon width must be divisible by {size}"
    );
    let block = width / size;
    let area = (block * block) as u32;
    let mut pixels = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let mut rgb = [0u32; 3];
            let mut alpha = 0;
            for row in y * block..(y + 1) * block {
                let start = (row * width + x * block) * 4;
                for pixel in source[start..start + block * 4].chunks_exact(4) {
                    let opacity = u32::from(pixel[3]);
                    alpha += opacity;
                    for (total, channel) in rgb.iter_mut().zip(pixel) {
                        *total += u32::from(*channel) * opacity;
                    }
                }
            }
            // Weight by alpha to keep the transparent edges free of dark fringes.
            let red_green_blue = rgb.map(|total| ((total + alpha / 2) / alpha.max(1)) as u8);
            pixels.extend(red_green_blue);
            pixels.push(((alpha + area / 2) / area) as u8);
        }
    }
    pixels
}
