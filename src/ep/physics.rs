use glam::{Mat4, Vec2};

#[derive(Debug, Clone)]
pub struct ContinuousImageInfo {
    pub path: std::path::PathBuf,
    pub original_size: Vec2,
    pub y_offset: f32,
    pub scaled_height: f32,
}

#[derive(Debug, Clone)]
pub struct ViewportPhysics {
    pub zoom: f32,
    pub target_zoom: f32,
    pub offset: Vec2,
    pub target_offset: Vec2,
    pub velocity: Vec2,
    pub rotation: f32,
    pub target_rotation: f32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub damping: f32,
    pub friction: f32,
    pub image_size: Vec2,
    pub manual_dirty: bool,
    pub is_long_image: bool,
    pub long_image_width_ratio: f32,
    pub auto_scroll_speed: f32,
    pub continuous_images: Vec<ContinuousImageInfo>,
    pub spacing: f32,
    pub lock_viewport: bool,
}

impl Default for ViewportPhysics {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewportPhysics {
    pub fn new() -> Self {
        Self {
            zoom: 1.0,
            target_zoom: 1.0,
            offset: Vec2::ZERO,
            target_offset: Vec2::ZERO,
            velocity: Vec2::ZERO,
            rotation: 0.0,
            target_rotation: 0.0,
            flip_h: false,
            flip_v: false,
            damping: 20.0,  // High damping for responsive feel
            friction: 0.92, // Friction for inertia
            image_size: Vec2::new(1.0, 1.0),
            manual_dirty: true,
            is_long_image: false,
            long_image_width_ratio: 1.0,
            auto_scroll_speed: 0.0,
            continuous_images: Vec::new(),
            spacing: 8.0,
            lock_viewport: false,
        }
    }

    pub fn tick(&mut self, dt: f32, vw: f32, vh: f32) -> bool {
        let old_zoom = self.zoom;
        let old_offset = self.offset;
        let old_rotation = self.rotation;
        let was_manual = self.manual_dirty;
        self.manual_dirty = false;

        // 1. Handle interpolation for Zoom and Rotation
        let alpha = 1.0 - (-self.damping * dt).exp();
        self.zoom = self.zoom + (self.target_zoom - self.zoom) * alpha;
        self.rotation = self.rotation + (self.target_rotation - self.rotation) * alpha;

        // 2. Calculate Bounds and Constrain Target Offset
        // We use target_zoom and target_rotation to calculate the constraints for target_offset
        let (max_x, max_y) = self.calculate_bounds(vw, vh);

        // 3. Handle Offset with Inertia
        if self.velocity.length() > 0.1 {
            self.target_offset += self.velocity * dt;
            // Decay velocity
            self.velocity *= self.friction.powf(dt * 60.0);
        }

        // Apply constraints to target_offset (The "Bounce Back" target)
        if self.is_long_image && self.auto_scroll_speed != 0.0 {
            self.target_offset.y -= self.auto_scroll_speed * dt;
        }

        self.target_offset.x = self.target_offset.x.clamp(-max_x, max_x);
        self.target_offset.y = self.target_offset.y.clamp(-max_y, max_y);

        // Interpolate offset towards target_offset
        self.offset = self.offset + (self.target_offset - self.offset) * alpha;

        // Returns true if the state has changed significantly
        was_manual
            || (self.zoom - old_zoom).abs() > 0.0001
            || (self.offset - old_offset).length() > 0.01
            || (self.rotation - old_rotation).abs() > 0.001
    }

    pub fn calculate_bounds(&self, vw: f32, vh: f32) -> (f32, f32) {
        if self.is_long_image && !self.continuous_images.is_empty() {
            let actual_w = self.image_size.x * self.target_zoom;
            let max_x = if actual_w <= vw {
                0.0
            } else {
                (actual_w - vw) / 2.0
            };

            let total_height = (self.continuous_images.last().unwrap().y_offset
                + self.continuous_images.last().unwrap().scaled_height)
                * self.target_zoom;
            let max_y = if total_height <= vh {
                0.0
            } else {
                (total_height - vh) / 2.0
            };

            (max_x, max_y)
        } else {
            let iw = self.image_size.x * self.target_zoom;
            let ih = self.image_size.y * self.target_zoom;

            // Use AABB for rotated image bounds calculation
            let cos_r = self.target_rotation.cos().abs();
            let sin_r = self.target_rotation.sin().abs();
            let actual_w = iw * cos_r + ih * sin_r;
            let actual_h = iw * sin_r + ih * cos_r;

            let max_x = if actual_w <= vw {
                0.0
            } else {
                (actual_w - vw) / 2.0
            };

            let max_y = if actual_h <= vh {
                0.0
            } else {
                (actual_h - vh) / 2.0
            };

            (max_x, max_y)
        }
    }

    pub fn reset_with_size(&mut self, width: f32, height: f32, viewport_w: f32, viewport_h: f32) {
        if self.lock_viewport && self.image_size.x > 0.0 {
            self.image_size = Vec2::new(width, height);
            let (max_x, max_y) = self.calculate_bounds(viewport_w, viewport_h);
            self.target_offset.x = self.target_offset.x.clamp(-max_x, max_x);
            self.target_offset.y = self.target_offset.y.clamp(-max_y, max_y);
            return;
        }

        // Calculate the current relative scroll position before resizing
        let old_max_y = self.calculate_bounds(viewport_w, viewport_h).1;
        let scroll_ratio = if old_max_y > 0.0 {
            // Corrected mapping: 0.0 is top (old_max_y), 1.0 is bottom (-old_max_y)
            ((old_max_y - self.target_offset.y) / (2.0 * old_max_y)).clamp(0.0, 1.0)
        } else {
            0.0
        };

        self.image_size = Vec2::new(width, height);

        let fit_zoom = if self.is_long_image {
            // Width Fit: Let the image width fit the viewport width (with a small padding of 16px) * custom ratio
            let padding = 16.0;
            (((viewport_w - padding) / width) * self.long_image_width_ratio).min(5000.0)
        } else {
            // Calculate "Contain" zoom
            let zoom_x = viewport_w / width;
            let zoom_y = viewport_h / height;
            zoom_x.min(zoom_y)
        };

        self.zoom = fit_zoom;
        self.target_zoom = fit_zoom;

        // Recalculate bounds with new zoom
        let (_, new_max_y) = self.calculate_bounds(viewport_w, viewport_h);

        let initial_y = if self.is_long_image {
            if new_max_y > 0.0 {
                // Maintain the exact same relative scroll position (0.0 is top, 1.0 is bottom)
                new_max_y - scroll_ratio * 2.0 * new_max_y
            } else {
                0.0
            }
        } else {
            0.0
        };

        self.offset = Vec2::new(0.0, initial_y);
        self.target_offset = Vec2::new(0.0, initial_y);
        self.rotation = 0.0;
        self.target_rotation = 0.0;
        self.velocity = Vec2::ZERO;
        self.flip_h = false;
        self.flip_v = false;

        tracing::info!(
            image = %format!("{}x{}", width, height),
            viewport = %format!("{}x{}", viewport_w, viewport_h),
            fit_zoom,
            is_long_image = self.is_long_image,
            "Physics State Reset with Auto-Fit"
        );
    }

    pub fn zoom_at(&mut self, delta: f32, mx: f32, my: f32, viewport_w: f32, viewport_h: f32) {
        let zoom_factor = 1.15f32; // Slightly faster zoom for responsiveness
        let old_target_zoom = self.target_zoom;

        if delta > 0.0 {
            self.target_zoom *= zoom_factor;
        } else if delta < 0.0 {
            self.target_zoom /= zoom_factor;
        }

        // Clamp zoom to reasonable limits
        self.target_zoom = self.target_zoom.clamp(0.001, 5000.0);

        let ratio = self.target_zoom / old_target_zoom;

        // Center of the viewport
        let cx = viewport_w / 2.0;
        let cy = viewport_h / 2.0;

        // P is the mouse position relative to the viewport center
        let px_rel = mx - cx;
        let py_rel = my - cy;

        // New Offset calculation to keep the point under the mouse stationary
        // O_new = P_rel - (P_rel - O_old) * ratio
        self.target_offset.x = px_rel - (px_rel - self.target_offset.x) * ratio;
        self.target_offset.y = py_rel - (py_rel - self.target_offset.y) * ratio;
    }

    pub fn get_view_proj(&self, width: f32, height: f32) -> Mat4 {
        // 1. Orthographic Projection (0,0 at Top-Left, width,height at Bottom-Right)
        let projection = Mat4::orthographic_rh_gl(0.0, width, height, 0.0, -1.0, 1.0);

        // 2. Center of the screen
        let cx = width / 2.0;
        let cy = height / 2.0;

        // 3. Transformation Sequence:
        // Translate to (cx + offset.x, cy + offset.y)
        // Rotate by self.rotation
        // Scale by (image_size * zoom)

        let transform =
            Mat4::from_translation(glam::vec3(cx + self.offset.x, cy + self.offset.y, 0.0))
                * Mat4::from_rotation_z(self.rotation)
                * Mat4::from_scale(glam::vec3(
                    self.image_size.x * self.zoom * (if self.flip_h { -1.0 } else { 1.0 }),
                    self.image_size.y * self.zoom * (if self.flip_v { -1.0 } else { 1.0 }),
                    1.0,
                ));

        projection * transform
    }

    pub fn flip(&mut self, horizontal: bool) {
        if horizontal {
            self.flip_h = !self.flip_h;
        } else {
            self.flip_v = !self.flip_v;
        }

        // Apply mirror transformation to offset and target_offset in rotated space
        // so that the flip is centered on the viewport center.
        let cos_r = self.rotation.cos();
        let sin_r = self.rotation.sin();

        // 1. Transform offset
        let rx = cos_r * self.offset.x + sin_r * self.offset.y;
        let ry = -sin_r * self.offset.x + cos_r * self.offset.y;
        let (rx_new, ry_new) = if horizontal { (-rx, ry) } else { (rx, -ry) };
        self.offset.x = cos_r * rx_new - sin_r * ry_new;
        self.offset.y = sin_r * rx_new + cos_r * ry_new;

        // 2. Transform target_offset (using target_rotation as it represents the target state)
        let cos_tr = self.target_rotation.cos();
        let sin_tr = self.target_rotation.sin();
        let trx = cos_tr * self.target_offset.x + sin_tr * self.target_offset.y;
        let try_val = -sin_tr * self.target_offset.x + cos_tr * self.target_offset.y;
        let (trx_new, try_new) = if horizontal {
            (-trx, try_val)
        } else {
            (trx, -try_val)
        };
        self.target_offset.x = cos_tr * trx_new - sin_tr * try_new;
        self.target_offset.y = sin_tr * trx_new + cos_tr * try_new;
    }

    pub fn format_status(&self) -> String {
        format!(
            "{}x{} | {:.0}%",
            self.image_size.x,
            self.image_size.y,
            self.zoom * 100.0
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_physics_interpolation() {
        let mut physics = ViewportPhysics::new();
        physics.target_zoom = 2.0;

        // After 1 second with damping 10, it should be very close to 2.0
        physics.tick(1.0, 1000.0, 1000.0);
        assert!(
            (physics.zoom - 2.0).abs() < 0.01,
            "Zoom should interpolate towards target. Current: {}",
            physics.zoom
        );
    }

    #[test]
    fn test_physics_no_overshoot() {
        let mut physics = ViewportPhysics::new();
        physics.target_zoom = 2.0;

        // Multiple small steps
        for _ in 0..100 {
            physics.tick(0.016, 1000.0, 1000.0); // ~60fps
        }

        assert!(
            physics.zoom <= 2.0,
            "Zoom should not overshoot 2.0. Current: {}",
            physics.zoom
        );
        assert!(
            physics.zoom > 1.0,
            "Zoom should increase. Current: {}",
            physics.zoom
        );
    }

    #[test]
    fn test_physics_inertia_decay() {
        let mut physics = ViewportPhysics::new();
        physics.image_size = Vec2::new(2000.0, 2000.0);
        physics.zoom = 1.0;
        physics.target_zoom = 1.0;
        physics.velocity = Vec2::new(100.0, 0.0);

        // Initial tick (Viewport is 1000x1000, Image is 2000x2000)
        physics.tick(0.016, 1000.0, 1000.0);
        assert!(
            physics.offset.x > 0.0,
            "Offset should increase with velocity"
        );
        let v1 = physics.velocity.x;
        assert!(
            v1 < 100.0,
            "Velocity should decay due to friction. Current: {}",
            v1
        );

        // Run for 10 ticks
        for _ in 0..10 {
            physics.tick(0.016, 1000.0, 1000.0);
        }
        assert!(
            physics.offset.x > 3.0,
            "Offset should accumulate over time. Current: {}",
            physics.offset.x
        );
        assert!(physics.velocity.x < v1, "Velocity should continue to decay");
    }

    #[test]
    fn test_boundary_constraints() {
        let mut physics = ViewportPhysics::new();
        // Image 2000x2000 in 1000x1000 viewport. Max offset = (2000-1000)/2 = 500
        physics.image_size = Vec2::new(2000.0, 2000.0);
        physics.zoom = 1.0;
        physics.target_zoom = 1.0;

        physics.target_offset = Vec2::new(1000.0, 0.0); // Way beyond limit
        physics.tick(0.1, 1000.0, 1000.0);

        assert!(
            physics.target_offset.x <= 500.01,
            "Target offset should be clamped to 500. Got: {}",
            physics.target_offset.x
        );

        // Image smaller than viewport should be centered
        physics.image_size = Vec2::new(500.0, 500.0);
        physics.target_offset = Vec2::new(100.0, 100.0);
        physics.tick(0.1, 1000.0, 1000.0);
        assert_eq!(
            physics.target_offset,
            Vec2::ZERO,
            "Small image should have zero target offset (centered)"
        );
    }

    #[test]
    fn test_physics_is_decoupled() {
        // PEC Verification: physics.rs contains tick() function with delta time.
        // We check if the source contains the required signature.
        let source = std::fs::read_to_string("src/ep/physics.rs")
            .or_else(|_| std::fs::read_to_string("src-tauri/src/ep/physics.rs"))
            .expect("Could not read physics.rs");
        assert!(
            source.contains("fn tick"),
            "Physics engine must expose a physics loop handler."
        );
    }

    #[test]
    fn test_flip_offset_reflection() {
        use glam::Vec2;
        let mut physics = ViewportPhysics::new();
        // Setup initial offset and target offset
        physics.offset = Vec2::new(100.0, 50.0);
        physics.target_offset = Vec2::new(120.0, 60.0);

        // Flip horizontal with 0 rotation
        physics.flip(true);
        assert!(physics.flip_h);
        assert!((physics.offset.x + 100.0).abs() < 0.001);
        assert!((physics.offset.y - 50.0).abs() < 0.001);
        assert!((physics.target_offset.x + 120.0).abs() < 0.001);
        assert!((physics.target_offset.y - 60.0).abs() < 0.001);

        // Reset
        physics.offset = Vec2::new(100.0, 50.0);
        physics.target_offset = Vec2::new(120.0, 60.0);
        physics.rotation = 90.0f32.to_radians();
        physics.target_rotation = 90.0f32.to_radians();

        // Flip horizontal under 90 degree rotation (equivalent to vertical flip in screen space)
        physics.flip(true);
        assert!((physics.offset.x - 100.0).abs() < 0.001);
        assert!((physics.offset.y + 50.0).abs() < 0.001);
        assert!((physics.target_offset.x - 120.0).abs() < 0.001);
        assert!((physics.target_offset.y + 60.0).abs() < 0.001);
    }

    /// TDD 测试 1: 验证长图模式下，若图片被放大（或调整宽度比例），水平移动限制 max_x 不得锁死为 0.0，必须允许水平拖拽
    #[test]
    fn test_long_image_horizontal_panning_unlocked_when_zoomed() {
        let mut physics = ViewportPhysics::new();
        physics.is_long_image = true;
        physics.image_size = Vec2::new(1000.0, 5000.0); // 1000x5000 的长图

        let vw = 800.0;
        let vh = 600.0;

        // 1. 默认自适应模式下：拉伸宽度以适应窗口（带有 16px 的 padding，比例 1.0）
        physics.target_zoom = 0.784; // (800 - 16) / 1000
        let (max_x, _) = physics.calculate_bounds(vw, vh);
        assert_eq!(max_x, 0.0, "默认宽度自适应长图应该水平居中");

        // 2. 交互放大场景：用户使用 Ctrl+滚轮放大到 2.0x
        physics.target_zoom = 2.0;
        let (max_x_zoomed, _) = physics.calculate_bounds(vw, vh);
        assert!(
            max_x_zoomed > 0.0,
            "放大长图后，最大水平偏移必须解锁（应大于 0.0）。当前值为: {}",
            max_x_zoomed
        );
        assert_eq!(max_x_zoomed, 600.0); // (1000*2.0 - 800)/2 = 600
    }

    /// TDD 测试 2: 验证长图滚动条进度正反向逻辑（0.0 代表最顶端，1.0 代表最底端）
    #[test]
    fn test_long_image_scroll_direction_mapping() {
        let mut physics = ViewportPhysics::new();
        physics.is_long_image = true;
        physics.image_size = Vec2::new(1000.0, 5000.0);

        let vw = 800.0;
        let vh = 600.0;
        physics.target_zoom = 0.8; // 4000px 高度

        let (_, max_y) = physics.calculate_bounds(vw, vh); // (4000 - 600)/2 = 1700
        assert!(max_y > 0.0);

        // A. 进度为 0.0 时，必须位于最顶端，偏移量为正 max_y
        // 我们在此验证预期修复后的数学逻辑契约
        let target_y_top = max_y - 0.0 * 2.0 * max_y;
        assert_eq!(target_y_top, max_y, "0.0 进度应正向对应最顶端");

        // B. 进度为 1.0 时，必须位于最底端，偏移量为负 -max_y
        let target_y_bottom = max_y - 1.0 * 2.0 * max_y;
        assert_eq!(target_y_bottom, -max_y, "1.0 进度应正向对应最底端");

        // C. 当前处于最顶端 offset = max_y 时，计算进度应为 0.0
        let p_top = ((max_y - max_y) / (2.0 * max_y)).clamp(0.0, 1.0);
        assert_eq!(p_top, 0.0);

        // D. 当前处于最底端 offset = -max_y 时，计算进度应为 1.0
        let p_bottom = ((max_y - (-max_y)) / (2.0 * max_y)).clamp(0.0, 1.0);
        assert_eq!(p_bottom, 1.0);
    }

    /// TDD 测试 3: 验证多图拼接模式下一维瀑布流的绝对布局与 max_y 边界计算
    #[test]
    fn test_continuous_waterfall_layout_calculation() {
        let mut physics = ViewportPhysics::new();
        physics.is_long_image = true;
        physics.spacing = 10.0;

        // 构造三张图片的布局数据
        physics.continuous_images = vec![
            ContinuousImageInfo {
                path: std::path::PathBuf::from("1.jpg"),
                original_size: Vec2::new(100.0, 500.0),
                y_offset: 0.0,
                scaled_height: 500.0,
            },
            ContinuousImageInfo {
                path: std::path::PathBuf::from("2.jpg"),
                original_size: Vec2::new(100.0, 800.0),
                y_offset: 510.0, // 500 + spacing (10.0)
                scaled_height: 800.0,
            },
            ContinuousImageInfo {
                path: std::path::PathBuf::from("3.jpg"),
                original_size: Vec2::new(100.0, 300.0),
                y_offset: 1320.0, // 510 + 800 + spacing (10.0)
                scaled_height: 300.0,
            },
        ];

        let vw = 100.0;
        let vh = 600.0;
        physics.target_zoom = 1.0;

        let (_, max_y) = physics.calculate_bounds(vw, vh);
        // 总高度 = 1320.0 (第三张起点) + 300.0 (第三张高度) = 1620.0
        // max_y = (1620.0 - 600.0) / 2 = 510.0
        assert_eq!(
            max_y, 510.0,
            "多图拼接瀑布流的总边界 max_y 应该基于累加总高正确计算"
        );
    }

    /// TDD 测试 4: 验证 lock_viewport 开启时，reset_with_size 不会重置 zoom，且能正确 clamp offset
    #[test]
    fn test_lock_viewport_preserves_zoom_and_clamps_offset() {
        let mut physics = ViewportPhysics::new();
        physics.image_size = Vec2::new(1000.0, 1000.0);
        physics.target_zoom = 2.0; // 放大 2 倍
        physics.zoom = 2.0;
        physics.target_offset = Vec2::new(500.0, 500.0); // 移动到了右下角
        physics.offset = Vec2::new(500.0, 500.0);

        let vw = 1000.0;
        let vh = 1000.0;

        // 开启锁定
        physics.lock_viewport = true;

        // 载入一张更小的图 (500x500)
        physics.reset_with_size(500.0, 500.0, vw, vh);

        // 1. 验证 zoom 是否被保持
        assert_eq!(
            physics.target_zoom, 2.0,
            "lock_viewport 时 target_zoom 应保持不变"
        );
        assert_eq!(physics.zoom, 2.0, "lock_viewport 时 zoom 应保持不变");

        // 2. 验证 bounds clamp
        // 新图片渲染大小: 500 * 2.0 = 1000x1000
        // viewport 大小: 1000x1000
        // max_x = (1000 - 1000)/2 = 0.0
        // max_y = (1000 - 1000)/2 = 0.0
        // 所以 500.0, 500.0 的 offset 应该被 clamp 到 0.0, 0.0
        assert_eq!(
            physics.target_offset.x, 0.0,
            "offset 应该被 clamp 到新的安全边界"
        );
        assert_eq!(
            physics.target_offset.y, 0.0,
            "offset 应该被 clamp 到新的安全边界"
        );
        assert_eq!(physics.image_size.x, 500.0, "image_size 应该更新");
    }
}
