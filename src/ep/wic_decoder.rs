use anyhow::{anyhow, Result};
use std::path::Path;
use windows::core::HSTRING;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

const CLSID_WICIMAGING_FACTORY: windows::core::GUID =
    windows::core::GUID::from_u128(0xcacaf262_9370_4615_a13b_9f5539da4c0a);

/// 确保当前线程加入了 COM 多线程单元 (MTA)
fn ensure_mta() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

pub struct WicDecoder {
    factory: IWICImagingFactory,
    _decoder: IWICBitmapDecoder, // 保留引用防止 COM 对象被回收
    frame: IWICBitmapFrameDecode,
    width: u32,
    height: u32,
}

// 既然我们使用了 MTA (COINIT_MULTITHREADED)，WIC 对象是跨线程安全的。
// 我们在此处声明 Send 和 Sync，以便能够在后台 Tokio 线程池中共享此解码器实例来提取 Tile。
unsafe impl Send for WicDecoder {}
unsafe impl Sync for WicDecoder {}

impl WicDecoder {
    /// 打开文件并初始化解码器
    pub fn new(path: &Path) -> Result<Self> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("Invalid path for WIC decoder"))?;
        let path_hstr = HSTRING::from(path_str);

        ensure_mta();

        unsafe {
            // 1. 创建 WIC Imaging Factory
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICIMAGING_FACTORY, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| anyhow!("Failed to create IWICImagingFactory: {}", e))?;

            // 2. 创建 Decoder (开启懒加载模式 WICDecodeMetadataCacheOnDemand)
            let decoder: IWICBitmapDecoder = factory
                .CreateDecoderFromFilename(
                    &path_hstr,
                    None,
                    GENERIC_READ,
                    WICDecodeMetadataCacheOnDemand,
                )
                .map_err(|e| anyhow!("Failed to create decoder for file: {}", e))?;

            // 3. 提取第 0 帧
            let frame: IWICBitmapFrameDecode = decoder
                .GetFrame(0)
                .map_err(|e| anyhow!("Failed to get frame 0: {}", e))?;

            // 4. 获取原始尺寸
            let mut width = 0;
            let mut height = 0;
            frame
                .GetSize(&mut width, &mut height)
                .map_err(|e| anyhow!("Failed to get frame size: {}", e))?;

            Ok(Self {
                factory,
                _decoder: decoder,
                frame,
                width,
                height,
            })
        }
    }

    /// 获取图片原始的像素维度
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 提取适应屏幕的缩小预览图（Base Texture）
    /// 借助 `IWICBitmapScaler` 实现极速底层硬件缩放，避免在内存中展开全图
    pub fn get_scaled_preview(&self, max_dim: u32) -> Result<(u32, u32, Vec<u8>)> {
        ensure_mta();
        unsafe {
            let (scaled_w, scaled_h) = Self::calc_scale(self.width, self.height, max_dim);

            let scaler: IWICBitmapScaler = self
                .factory
                .CreateBitmapScaler()
                .map_err(|e| anyhow!("Failed to create WIC scaler: {}", e))?;

            scaler
                .Initialize(
                    &self.frame,
                    scaled_w,
                    scaled_h,
                    WICBitmapInterpolationModeFant,
                )
                .map_err(|e| anyhow!("Failed to initialize WIC scaler: {}", e))?;

            let converter: IWICFormatConverter = self
                .factory
                .CreateFormatConverter()
                .map_err(|e| anyhow!("Failed to create format converter: {}", e))?;

            converter
                .Initialize(
                    &scaler,
                    &GUID_WICPixelFormat32bppRGBA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .map_err(|e| anyhow!("Failed to initialize format converter: {}", e))?;

            let stride = scaled_w * 4;
            let buffer_size = stride as usize * scaled_h as usize;
            let mut buffer = vec![0u8; buffer_size];

            converter
                .CopyPixels(
                    std::ptr::null(), // 读取全部已缩放的像素
                    stride,
                    &mut buffer,
                )
                .map_err(|e| anyhow!("Failed to copy scaled pixels: {}", e))?;

            Ok((scaled_w, scaled_h, buffer))
        }
    }

    /// 提取全分辨率的指定矩形块（Tile-on-Demand 核心）
    /// 从原始图片中按需抠出一块完整的 Tile，直接上传 GPU，绝对不在 CPU 中合并全图
    pub fn extract_tile(&self, x: u32, y: u32, w: u32, h: u32) -> Result<Vec<u8>> {
        ensure_mta();
        unsafe {
            // 防溢出保护
            let safe_w = if x + w > self.width {
                self.width.saturating_sub(x)
            } else {
                w
            };
            let safe_h = if y + h > self.height {
                self.height.saturating_sub(y)
            } else {
                h
            };

            if safe_w == 0 || safe_h == 0 {
                return Ok(Vec::new());
            }

            let converter: IWICFormatConverter = self
                .factory
                .CreateFormatConverter()
                .map_err(|e| anyhow!("Failed to create format converter: {}", e))?;

            converter
                .Initialize(
                    &self.frame,
                    &GUID_WICPixelFormat32bppRGBA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .map_err(|e| anyhow!("Failed to initialize format converter: {}", e))?;

            let rect = WICRect {
                X: x as i32,
                Y: y as i32,
                Width: safe_w as i32,
                Height: safe_h as i32,
            };

            let stride = safe_w * 4;
            let buffer_size = stride as usize * safe_h as usize;
            let mut buffer = vec![0u8; buffer_size];

            converter
                .CopyPixels(&rect, stride, &mut buffer)
                .map_err(|e| anyhow!("Failed to copy tile pixels: {}", e))?;

            Ok(buffer)
        }
    }

    /// 工具函数：计算保持宽高比的缩放尺寸
    fn calc_scale(w: u32, h: u32, max_dim: u32) -> (u32, u32) {
        if w <= max_dim && h <= max_dim {
            return (w, h);
        }
        let ratio = max_dim as f32 / w.max(h) as f32;
        let nw = (w as f32 * ratio).round() as u32;
        let nh = (h as f32 * ratio).round() as u32;
        (nw.max(1), nh.max(1))
    }
}
