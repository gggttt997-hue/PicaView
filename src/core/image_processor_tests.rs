use super::*;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_image_stitching_logic() {
    let temp = tempdir().unwrap();

    // 1. Create 3 small images (10x10, 10x20, 20x10)
    let img1 = DynamicImage::ImageRgb8(RgbImage::from_pixel(10, 10, Rgb([255, 0, 0])));
    let img2 = DynamicImage::ImageRgb8(RgbImage::from_pixel(10, 20, Rgb([0, 255, 0])));
    let img3 = DynamicImage::ImageRgb8(RgbImage::from_pixel(20, 10, Rgb([0, 0, 255])));

    let p1 = temp.path().join("1.png");
    let p2 = temp.path().join("2.png");
    let p3 = temp.path().join("3.png");
    img1.save(&p1).unwrap();
    img2.save(&p2).unwrap();
    img3.save(&p3).unwrap();

    let input_paths = vec![
        p1.to_string_lossy().to_string(),
        p2.to_string_lossy().to_string(),
        p3.to_string_lossy().to_string(),
    ];

    let out_path = temp.path().join("stitched.png");
    let out_path_str = out_path.to_string_lossy().to_string();

    let stitcher = ImageStitcher::new();

    // 2. Test Horizontal Stitching with Align and Gap
    let config = StitchConfig {
        direction: StitchDirection::Horizontal,
        align: true,
        gap: 5,
        background_color: [0, 0, 0],
    };

    let result = stitcher.stitch(input_paths, config, &out_path_str).unwrap();
    let final_img = image::open(result).unwrap();

    // Alignment logic: all scaled to match img1 height (10)
    // img1: 10x10
    // img2: 10x20 -> scaled to 5x10 (maintain aspect ratio)
    // img3: 20x10 -> scaled to 20x10
    // Total width = 10 + 5 + 5 (gap) + 20 + 5 (gap) = 45
    // Total height = 10

    assert_eq!(final_img.height(), 10);
    assert_eq!(final_img.width(), 45);
}

#[test]
fn test_batch_convert_fail_fast() {
    let temp = tempdir().unwrap();
    let out_dir = temp.path().to_str().unwrap();

    // 1. Prepare input files
    let img = DynamicImage::ImageRgb8(RgbImage::new(10, 10));
    let path1 = temp.path().join("input1.png");
    img.save_with_format(&path1, ImageFormat::Png).unwrap();

    // Create a fake "corrupt" file that will fail to decode
    let path2 = temp.path().join("input2.jpg");
    fs::write(&path2, b"not an image").unwrap();

    let input_paths = vec![
        path1.to_string_lossy().to_string(),
        path2.to_string_lossy().to_string(),
    ];

    // 2. Run conversion
    let processor = BatchProcessor::new();
    let result = processor.convert(input_paths, OutputFormat::WebP, out_dir, |_, _| {});

    // 3. Verify Fail-Fast
    assert!(
        result.is_err(),
        "Conversion should fail due to corrupt file"
    );

    // 4. Verify Zero-Residue
    let files: Vec<_> = fs::read_dir(out_dir)
        .unwrap()
        .map(|res| res.unwrap().path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "webp"))
        .collect();

    assert_eq!(
        files.len(),
        0,
        "No partial webp files should remain in the output directory"
    );
}
