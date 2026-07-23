use super::*;
use std::fs;
use tempfile::TempDir;

fn create_test_directory_with_images() -> TempDir {
    let temp_dir = TempDir::new().unwrap();

    // Create test image files
    let file_names = ["test1.jpg", "test2.png", "test3.gif"];
    for name in &file_names {
        let file_path = temp_dir.path().join(name);
        fs::write(&file_path, "test content").unwrap();
    }

    temp_dir
}

#[test]
fn test_app_state_creation() {
    let state = AppState::new();

    let summary = state.get_status_summary().unwrap();
    assert_eq!(summary.total_images, 0);
    assert_eq!(summary.current_index, 0);
    assert!(!summary.has_images);
    assert!(summary.current_directory.is_none());

    // Test if HistoryManager is integrated
    assert!(state.history_manager.lock().is_ok());
}

#[test]
fn test_manga_mode_initial_state() {
    let state = AppState::new();
    let manga_mode = state.manga_mode_enabled.lock().unwrap();
    assert!(!(*manga_mode), "Manga mode should be disabled by default");
}

#[test]
fn test_history_manager_capacity() {
    let state = AppState::new();
    let history_manager = state.history_manager.lock().unwrap();
    assert_eq!(
        history_manager.capacity(),
        50,
        "HistoryManager capacity should be 50"
    );
}

#[test]
fn test_file_manager_access() {
    use crate::core::file_manager::perform_directory_scan_blocking;
    let state = AppState::new();
    let temp_dir = create_test_directory_with_images();
    let test_file = temp_dir.path().join("test1.jpg");

    // Execute blocking scan and update state
    let (_file_count, new_infos, target_index, directory) =
        perform_directory_scan_blocking(&test_file).unwrap();
    state
        .with_file_manager_mut(|manager| {
            manager.update_state(new_infos, target_index, directory);
        })
        .unwrap();

    // Test read-only access
    let result = state.with_file_manager(|manager| manager.total_count());

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 3);
}

#[test]
fn test_status_summary() {
    use crate::core::file_manager::perform_directory_scan_blocking;
    let state = AppState::new();
    let temp_dir = create_test_directory_with_images();
    let test_file = temp_dir.path().join("test1.jpg");

    // Scan directory and update state
    let (_file_count, new_infos, target_index, directory) =
        perform_directory_scan_blocking(&test_file).unwrap();
    state
        .with_file_manager_mut(|manager| {
            manager.update_state(new_infos, target_index, directory);
        })
        .unwrap();

    // Get status summary
    let summary = state.get_status_summary().unwrap();
    assert_eq!(summary.total_images, 3);
    assert_eq!(summary.current_index, 0);
    assert!(summary.has_images);
    assert!(summary.current_directory.is_some());

    // Test string formatting
    let summary_str = summary.to_string();
    assert!(summary_str.contains("3 images"));
    assert!(summary_str.contains("current index: 0"));
}

#[test]
fn test_concurrent_access() {
    use crate::core::file_manager::perform_directory_scan_blocking;
    use std::thread;
    use std::time::Duration;

    let state = AppState::new();
    let temp_dir = create_test_directory_with_images();
    let test_file = temp_dir.path().join("test1.jpg");

    // Initialize state
    let (_file_count, new_infos, target_index, directory) =
        perform_directory_scan_blocking(&test_file).unwrap();
    state
        .with_file_manager_mut(|manager| {
            manager.update_state(new_infos, target_index, directory);
        })
        .unwrap();

    // Clone Arc<Mutex<FileManager>> instead of entire AppState
    let file_manager_clone = Arc::clone(&state.file_manager);

    // Start a thread for navigation operations
    let handle = thread::spawn(move || {
        for _ in 0..5 {
            // Use cloned Arc directly
            let _ = file_manager_clone.lock().unwrap().next_image();
            thread::sleep(Duration::from_millis(10));
        }
    });

    // Main thread queries state
    for _ in 0..5 {
        let _ = state.with_file_manager(|manager| manager.current_index());
        thread::sleep(Duration::from_millis(10));
    }

    handle.join().unwrap();

    // Verify final state
    let final_index = state
        .with_file_manager(|manager| manager.current_index())
        .unwrap();

    // Due to circular navigation, index should be between 0-2
    assert!(final_index < 3);
}

#[test]
fn test_thumbnail_cache_weight_eviction() {
    use crate::models::ImageInfo;

    // Create a cache with a max weight of 100 bytes
    let mut cache = ThumbnailCache::new(100);

    // Insert first entry, weight 64 (4x4x4)
    let mut info1 = ImageInfo::new("test1.jpg".into());
    info1.pixel_buffer = Some(slint::SharedPixelBuffer::new(4, 4));
    cache.put("test1.jpg".into(), info1);

    assert_eq!(cache.current_weight(), 64);
    assert!(cache.get("test1.jpg").is_some());

    // Insert second entry, weight 64 (total 128 > 100)
    let mut info2 = ImageInfo::new("test2.jpg".into());
    info2.pixel_buffer = Some(slint::SharedPixelBuffer::new(4, 4));
    cache.put("test2.jpg".into(), info2);

    // Expected eviction triggered
    // First entry should be evicted, total weight should be 64
    assert_eq!(
        cache.current_weight(),
        64,
        "Current weight should be 64 after eviction"
    );
    assert!(
        cache.get("test1.jpg").is_none(),
        "test1.jpg should be evicted"
    );
    assert!(cache.get("test2.jpg").is_some(), "test2.jpg should remain");
}
