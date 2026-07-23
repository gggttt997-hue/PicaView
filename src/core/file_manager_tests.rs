use super::*;
use crate::models::{SortCriteria, SortOrder};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_re_sort_persistence_and_index_preservation() {
    let mut manager = FileManager::new();
    let mut info1 = ImageInfo::from_path("a.jpg");
    info1.size = Some(100);
    let mut info2 = ImageInfo::from_path("b.jpg");
    info2.size = Some(50);

    manager.update_state(vec![info1.clone(), info2.clone()], 1, ".".into()); // Currently pointing to b.jpg (index 1)

    // Verify default status
    assert_eq!(manager.sort_criteria, SortCriteria::Name);
    assert_eq!(manager.sort_order, SortOrder::Ascending);

    // Execute re-sort (size ascending)
    manager.re_sort(SortCriteria::Size, SortOrder::Ascending);

    // Assert: internal state updated
    assert_eq!(manager.sort_criteria, SortCriteria::Size);
    assert_eq!(manager.sort_order, SortOrder::Ascending);

    // Assert: index corrected
    assert_eq!(manager.current_index(), 0);
    assert_eq!(manager.current().unwrap().file_name().unwrap(), "b.jpg");
}

#[test]
fn test_remove_filename_sync_all_image_infos() {
    let mut manager = FileManager::new();
    let info1 = ImageInfo::from_path("a.jpg");
    let info2 = ImageInfo::from_path("b.jpg");

    manager.update_state(vec![info1.clone(), info2.clone()], 0, ".".into());

    // Remove b.jpg
    manager.remove_filename("b.jpg");

    // Assert: removed from active list
    assert_eq!(manager.total_count(), 1);

    // Filter reset (which copies all_image_infos back to image_infos)
    manager.filter_assets(None, None, None, None, None, None, None);

    // Assert: b.jpg must NOT reappear, so active list size must remain 1
    assert_eq!(
        manager.total_count(),
        1,
        "Deleted file b.jpg must be cleared from all_image_infos too"
    );
}

#[test]
fn test_rebuild_exif_indices_skip_non_exif() {
    let mut manager = FileManager::new();
    let mut info1 = ImageInfo::from_path("a.jpg");
    info1.iso = Some(100);
    let info2 = ImageInfo::from_path("b.jpg"); // No EXIF

    manager.update_state(vec![info1, info2], 0, ".".into());

    // Assert: index is built for info1
    assert!(manager.iso_index.contains_key(&100));
    assert_eq!(manager.iso_index.get(&100).unwrap().len(), 1);
}

// Helper: create a temporary directory containing test files
fn create_test_directory() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().unwrap();
    let dir_path = temp_dir.path().to_path_buf();
    let file_names = ["a.jpg", "b.png", "c.gif", "d.bmp", "not_image.txt"];
    for name in &file_names {
        let file_path = dir_path.join(name);
        // Tests that perform header detection need real image content
        // Simplified here to write basic bytes
        let content = match name.split('.').next_back() {
            Some("jpg") => vec![0xFF, 0xD8, 0xFF, 0xE0],
            Some("png") => vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            _ => b"test content".to_vec(),
        };
        fs::write(&file_path, content).unwrap();
    }
    (temp_dir, dir_path)
}

#[test]
fn test_directory_scan_and_update() {
    let (_temp_dir, dir_path) = create_test_directory();
    let mut manager = FileManager::new();
    let first_image_path = dir_path.join("a.jpg");

    // Scan directory
    let (file_count, new_infos, target_index, directory) =
        perform_directory_scan_blocking(&first_image_path).unwrap();
    manager.update_state(new_infos, target_index, directory);

    assert_eq!(file_count, 4); // 4 image files
    assert_eq!(manager.total_count(), 4);
    assert!(manager.has_images());
    assert_eq!(manager.current_index(), 0); // Should point to 'a.jpg'
    assert_eq!(manager.current().unwrap(), first_image_path);
}

#[test]
fn test_navigation() {
    let (_temp_dir, dir_path) = create_test_directory();
    let mut manager = FileManager::new();
    let (file_count, new_infos, target_index, directory) =
        perform_directory_scan_blocking(&dir_path).unwrap();
    manager.update_state(new_infos, target_index, directory);

    assert_eq!(file_count, 4);
    assert_eq!(manager.current_index(), 0);
    assert_eq!(manager.current().unwrap(), dir_path.join("a.jpg"));

    // Test forward navigation
    let next_path = manager.next_image().unwrap();
    assert_eq!(manager.current_index(), 1);
    assert_eq!(next_path, dir_path.join("b.png"));

    // Test backward navigation
    let prev_path = manager.prev().unwrap();
    assert_eq!(manager.current_index(), 0);
    assert_eq!(prev_path, dir_path.join("a.jpg"));

    // Test cyclic navigation - from first to last
    let prev_path = manager.prev().unwrap();
    assert_eq!(manager.current_index(), 3); // Last image 'd.bmp'
    assert_eq!(prev_path, dir_path.join("d.bmp"));

    // Test cyclic navigation - from last to first
    let next_path = manager.next_image().unwrap();
    assert_eq!(manager.current_index(), 0); // First image 'a.jpg'
    assert_eq!(next_path, dir_path.join("a.jpg"));
}

#[test]
fn test_empty_directory() {
    let temp_dir = TempDir::new().unwrap();
    let mut manager = FileManager::new();

    let (file_count, new_infos, target_index, directory) =
        perform_directory_scan_blocking(temp_dir.path()).unwrap();
    manager.update_state(new_infos, target_index, directory);

    assert_eq!(file_count, 0);
    assert!(!manager.has_images());
    assert!(manager.next_image().is_none());
    assert!(manager.prev().is_none());
    assert!(manager.current().is_none());
}

#[test]
fn test_nonexistent_directory() {
    let result = perform_directory_scan_blocking(Path::new("/nonexistent/path"));
    assert!(result.is_err());
}

#[test]
fn test_remove_filename() {
    let (_temp_dir, dir_path) = create_test_directory();
    let mut manager = FileManager::new();
    let (_file_count, new_infos, target_index, directory) =
        perform_directory_scan_blocking(&dir_path).unwrap();
    manager.update_state(new_infos, target_index, directory);

    // Initial: a.jpg (0), b.png (1), c.gif (2), d.bmp (3)
    assert_eq!(manager.total_count(), 4);
    assert_eq!(manager.current_index(), 0);

    // Remove current image a.jpg
    manager.remove_filename("a.jpg");
    assert_eq!(manager.total_count(), 3);
    // index should still be 0, but now points to original b.png
    assert_eq!(manager.current_index(), 0);
    assert_eq!(manager.current().unwrap(), dir_path.join("b.png"));

    // Move to last d.bmp (now index 2)
    manager.next_image();
    manager.next_image();
    assert_eq!(manager.current_index(), 2);
    assert_eq!(manager.current().unwrap(), dir_path.join("d.bmp"));

    // Remove last image d.bmp
    manager.remove_filename("d.bmp");
    assert_eq!(manager.total_count(), 2);
    // index should decrement, pointing to current last c.gif (index 1)
    assert_eq!(manager.current_index(), 1);
    assert_eq!(manager.current().unwrap(), dir_path.join("c.gif"));
}

#[test]
fn test_reality_check_delete_consistency() {
    // Simulate deletion after high-frequency state switching
    let (_temp_dir, dir_path) = create_test_directory();
    let mut manager = FileManager::new();
    let (_, new_infos, index, dir) = perform_directory_scan_blocking(&dir_path).unwrap();
    manager.update_state(new_infos, index, dir);

    // 1. Navigate to middle
    manager.next_image(); // b.png (1)
    manager.next_image(); // c.gif (2)

    // 2. Delete current c.gif
    manager.remove_filename("c.gif");

    // 3. Assert consistency
    // [Evidence] Location: RealityCheck Suite, Capture: inSync: true, Rationale: State sync verification
    let in_sync = manager.total_count() == 3 && manager.current_index() == 2;
    assert!(
        in_sync,
        "Index should point to the next item (original d.bmp) after deleting middle item"
    );
    assert_eq!(manager.current().unwrap().file_name().unwrap(), "d.bmp");
}

#[test]
fn test_directory_scan_metadata_extraction() {
    let (_temp_dir, dir_path) = create_test_directory();
    let (_file_count, new_infos, _target_index, _directory) =
        perform_directory_scan_blocking(&dir_path).unwrap();

    assert!(!new_infos.is_empty());
    for info in new_infos {
        assert!(info.size.is_some(), "File size should be fetched");
        assert!(
            info.modified_ms.is_some(),
            "Modified time should be fetched"
        );
        assert!(info.size.unwrap() > 0, "File size should be > 0");
    }
}

#[test]
fn test_performance_10k_sort() {
    let mut manager = FileManager::new();
    let mut infos = Vec::with_capacity(10000);
    for i in 0..10000 {
        let mut info = ImageInfo::from_path(format!("img_{:05}.jpg", i));
        info.size = Some(i as u64 * 1024); // Different sizes to verify sorting logic
        infos.push(info);
    }

    manager.update_state(infos, 0, ".".into());

    let start = std::time::Instant::now();
    // Stress test: execute 10 sorts to simulate high-frequency switching
    for _ in 0..10 {
        manager.re_sort(SortCriteria::Size, SortOrder::Descending);
    }
    let duration = start.elapsed().as_millis() / 10;

    // [Evidence] Location: Reality Check (Rust), Capture: file_count=10000, avg_latency_ms={}, Rationale: Verify sorting SLA (< 50ms).
    println!(
        "[Reality Check] 10,000 files sort avg latency: {}ms",
        duration
    );
    assert!(
        duration < 50,
        "10k sort avg time {}ms exceeds SLA threshold (50ms)",
        duration
    );
}

#[test]
fn test_find_neighboring_dir() {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();

    // Create directory structure:
    // /parent/vol1
    // /parent/vol2
    // /parent/vol3
    let parent = base_path.join("parent");
    let vol1 = parent.join("vol1");
    let vol2 = parent.join("vol2");
    let vol3 = parent.join("vol3");

    fs::create_dir_all(&vol1).unwrap();
    fs::create_dir_all(&vol2).unwrap();
    fs::create_dir_all(&vol3).unwrap();

    let mut manager = FileManager::new();
    // Simulate being in vol2
    manager.update_state(vec![], 0, vol2.clone());

    // Test forward (vol3)
    let next = manager.find_neighboring_dir(true).unwrap();
    assert_eq!(next, Some(vol3.clone()));

    // Test backward (vol1)
    let prev = manager.find_neighboring_dir(false).unwrap();
    assert_eq!(prev, Some(vol1.clone()));

    // Test boundaries: vol3 next should be None
    manager.update_state(vec![], 0, vol3.clone());
    let next_none = manager.find_neighboring_dir(true).unwrap();
    assert_eq!(next_none, None);

    // Test boundaries: vol1 prev should be None
    manager.update_state(vec![], 0, vol1.clone());
    let prev_none = manager.find_neighboring_dir(false).unwrap();
    assert_eq!(prev_none, None);
}

#[test]
fn test_cascading_full_cycle() {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();

    let vol1 = base_path.join("Vol_01");
    let vol2 = base_path.join("Vol_02");
    fs::create_dir_all(&vol1).unwrap();
    fs::create_dir_all(&vol2).unwrap();

    // Create dummy images
    fs::write(vol1.join("01.jpg"), "").unwrap();
    fs::write(vol1.join("02.jpg"), "").unwrap();
    fs::write(vol2.join("01.jpg"), "").unwrap();

    let mut manager = FileManager::new();

    // 1. Initialize in Vol_01
    manager.scan_directory(&vol1).unwrap();
    assert_eq!(manager.total_count(), 2);

    // 2. Navigate to last (Index 1)
    manager.next_image().unwrap();
    assert_eq!(manager.current_index(), 1);

    // 3. Simulate manga mode interception logic: boundary reached, find next chapter
    let next_vol = manager.find_neighboring_dir(true).unwrap();
    assert_eq!(next_vol, Some(vol2.clone()));

    // 4. Simulate frontend directory switch operation
    manager.scan_directory(&vol2).unwrap();
    assert_eq!(manager.total_count(), 1);
    assert_eq!(manager.current_index(), 0);
    assert_eq!(manager.base_directory.as_ref().unwrap(), &vol2);
}

#[test]
fn test_exif_multidimensional_filter() {
    let mut manager = FileManager::new();

    // Create 3 mock images with different EXIF metadata
    let mut img1 = ImageInfo::from_path("fuji_50mm_f2_iso100.jpg");
    img1.camera_model = Some("Fujifilm X-T4".to_string());
    img1.focal_length = Some(50.0);
    img1.aperture = Some(2.0);
    img1.iso = Some(100);

    let mut img2 = ImageInfo::from_path("sony_85mm_f1.4_iso400.jpg");
    img2.camera_model = Some("Sony ILCE-7M4".to_string());
    img2.focal_length = Some(85.0);
    img2.aperture = Some(1.4);
    img2.iso = Some(400);

    let mut img3 = ImageInfo::from_path("sony_24mm_f4_iso1600.jpg");
    img3.camera_model = Some("Sony ILCE-7M4".to_string());
    img3.focal_length = Some(24.0);
    img3.aperture = Some(4.0);
    img3.iso = Some(1600);

    manager.update_state(vec![img1, img2, img3], 0, ".".into());
    assert_eq!(manager.total_count(), 3);

    // Test 1: Filter by Camera Model
    manager.filter_assets(None, None, None, None, None, None, Some("Sony".to_string()));
    assert_eq!(manager.total_count(), 2);

    // Test 2: Filter by ISO range
    manager.filter_assets(None, None, None, None, Some(200), Some(1000), None);
    assert_eq!(manager.total_count(), 1);
    assert_eq!(
        manager.current().unwrap().file_name().unwrap(),
        "sony_85mm_f1.4_iso400.jpg"
    );

    // Test 3: Filter by Focal Length range
    manager.filter_assets(Some(30.0), Some(90.0), None, None, None, None, None);
    assert_eq!(manager.total_count(), 2);

    // Test 4: Reset Filter
    manager.filter_assets(None, None, None, None, None, None, None);
    assert_eq!(manager.total_count(), 3);
}

#[test]
fn test_zip_archive_vfs_scan() {
    use std::io::Write;
    let temp_dir = TempDir::new().unwrap();
    let zip_path = temp_dir.path().join("manga.zip");

    // 创建压缩包并写入图片条目
    let file = fs::File::create(&zip_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default();

    zip.start_file("001.jpg", options).unwrap();
    zip.write_all(b"dummy jpeg content").unwrap();

    zip.start_file("002.png", options).unwrap();
    zip.write_all(b"dummy png content").unwrap();

    zip.start_file("readme.txt", options).unwrap(); // 不应该被扫描
    zip.write_all(b"text file").unwrap();

    zip.finish().unwrap();

    // 1. 测试直接双击/打开 zip 文件本身
    let (count, infos, index, dir) = perform_directory_scan_blocking(&zip_path).unwrap();
    assert_eq!(count, 2);
    assert_eq!(index, 0);
    assert_eq!(dir, zip_path);
    assert_eq!(infos[0].name, "001.jpg");
    assert_eq!(infos[1].name, "002.png");

    let zip_path_str = zip_path.to_string_lossy();
    assert_eq!(infos[0].path, format!("{}|001.jpg", zip_path_str));
    assert_eq!(infos[1].path, format!("{}|002.png", zip_path_str));

    // 2. 测试传入虚拟路径时的扫描
    let virtual_path = PathBuf::from(format!("{}|002.png", zip_path_str));
    let (count2, _infos2, index2, dir2) = perform_directory_scan_blocking(&virtual_path).unwrap();
    assert_eq!(count2, 2);
    assert_eq!(index2, 1);
    assert_eq!(dir2, zip_path);
}
