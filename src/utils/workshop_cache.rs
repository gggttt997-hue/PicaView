use dashmap::DashMap;
use image::DynamicImage;
use std::sync::Arc;
use tracing::{debug, info};

/// WorkshopCacheHub provides an in-memory buffer for high-fidelity images
/// during workshop operations (composition, conversion, ROI preview).
#[derive(Debug)]
pub struct WorkshopCacheHub {
    /// Maps file path to its decoded DynamicImage
    cache: DashMap<String, Arc<DynamicImage>>,
}

impl WorkshopCacheHub {
    pub fn new() -> Self {
        info!("[Evidence] Initializing WorkshopCacheHub");
        Self {
            cache: DashMap::new(),
        }
    }

    /// Loads an image into cache if not already present
    pub fn load(&self, path: &str, image: DynamicImage) {
        if !self.cache.contains_key(path) {
            info!(
                "[Evidence] WorkshopCacheHub: Loading asset to cache: {}",
                path
            );
            self.cache.insert(path.to_string(), Arc::new(image));
        } else {
            debug!(
                "[Evidence] WorkshopCacheHub: Asset already in cache, skipping: {}",
                path
            );
        }
    }

    /// Gets an image from cache
    pub fn get(&self, path: &str) -> Option<Arc<DynamicImage>> {
        let result = self.cache.get(path).map(|r| Arc::clone(r.value()));
        if result.is_some() {
            debug!("[ROI_HIT] Path: {}", path);
        } else {
            debug!("[ROI_MISS] Path: {}", path);
        }
        result
    }

    /// Removes a specific image from cache
    pub fn remove(&self, path: &str) {
        if self.cache.remove(path).is_some() {
            info!(
                "[Evidence] WorkshopCacheHub: Removed asset from cache: {}",
                path
            );
        }
    }

    /// Clears all cached images
    pub fn clear(&self) {
        let count = self.cache.len();
        self.cache.clear();
        info!(
            "[Evidence] WorkshopCacheHub: Cleared {} assets from cache",
            count
        );
    }
}

impl Default for WorkshopCacheHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbaImage};

    #[test]
    fn test_cache_operations() {
        let hub = WorkshopCacheHub::new();
        let path = "test_path.png";
        let img = DynamicImage::ImageRgba8(RgbaImage::new(10, 10));

        // Test loading
        hub.load(path, img.clone());
        assert!(hub.get(path).is_some());

        // Test idempotency
        hub.load(path, img);
        assert_eq!(hub.cache.len(), 1);

        // Test removal
        hub.remove(path);
        assert!(hub.get(path).is_none());

        // Test clear
        hub.load("path1", DynamicImage::ImageRgba8(RgbaImage::new(1, 1)));
        hub.load("path2", DynamicImage::ImageRgba8(RgbaImage::new(1, 1)));
        hub.clear();
        assert_eq!(hub.cache.len(), 0);
    }

    #[test]
    #[ignore = "Performance SLA test: Runs locally on dedicated hardware"]
    fn test_roi_performance_8k() {
        use crate::utils::image_utils::crop_image;
        use image::RgbaImage;
        use std::io::Cursor;
        use std::time::Instant;

        // Simulate 8K image
        let img = DynamicImage::ImageRgba8(RgbaImage::new(8000, 8000));
        let start = Instant::now();

        let cropped = crop_image(&img, 1000, 1000, 800, 600);
        let mut buffer = Cursor::new(Vec::new());
        cropped
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();

        let duration = start.elapsed();
        println!(
            "[SLA_AUDIT] 8K ROI Process + PNG Encode Time: {:?}",
            duration
        );

        // Assert backend processing is within SLA (PNG encoding is the bottleneck)
        // 800x600 PNG encoding should be < 200ms on most modern CPUs
        assert!(
            duration.as_millis() < 200,
            "ROI processing too slow: {:?}",
            duration
        );
    }
}
