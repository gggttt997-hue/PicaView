use image::{DynamicImage, ImageFormat};
use picaview_lib::ep::decoder::TiledDecoder;
use std::fs::File;
use std::io::BufWriter;
use std::time::Instant;

fn main() {
    println!("--- 100MP JPEG Memory Redline Audit ---");

    let path = "test_100mp.jpg";
    if !std::path::Path::new(path).exists() {
        println!("Generating 100MP JPEG (10000x10000)...");
        let img = DynamicImage::new_rgb8(10000, 10000);
        let file = File::create(path).unwrap();
        let w = &mut BufWriter::new(file);
        img.write_to(w, ImageFormat::Jpeg).unwrap();
        println!("Generated.");
    }

    let decoder = TiledDecoder::new(std::path::Path::new(path)).unwrap();

    // Decode a small tile at the top
    println!("Decoding 256x256 tile at (0, 0)...");
    let start_top = Instant::now();
    let _tile_top = decoder.decode_tile(0, 0, 256, 256).unwrap();
    println!("Top tile decoded in: {:?}", start_top.elapsed());

    // Decode a small tile at the middle
    println!("Decoding 256x256 tile at (5000, 5000)...");
    let start_mid = Instant::now();
    let _tile_mid = decoder.decode_tile(5000, 5000, 256, 256).unwrap();
    println!("Middle tile decoded in: {:?}", start_mid.elapsed());

    // Check memory usage (rough estimate via /proc/self/status on Linux)
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("VmRSS:") || line.starts_with("VmPeak:") {
                println!("{}", line);
            }
        }
    }
}
