//! Unified hardware codec abstraction for Peekboard
//! 
//! This crate is platform-agnostic. It defines the traits and data structures that sit between the screen-capture / streaming logic and per vendor
//! hardware encoder backends(Tegra NVENC, Rockchip MPP, Intel oneVPL, ...)
//! 
//! Backends depend on this crate and implement [`HwEncoder`];
//! the upper layers (capture, streamer, viewer) only ever talk to the trait, never to a specific vendor API

#![deny(missing_debug_implementations)]

use thiserror::Error;

pub mod color;
pub mod protocol;

#[cfg(feature = "openh264-backend")]
pub mod soft_openh264;

///Compressed codec format produced by the encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodecFormat {
    ///H264 / AVC
    #[default]
    H264,
    ///H265 / HEVC
    H265,
}

/// Raw pixel format accepted by encoder backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 8-bit RGBA, one plane
    Rgba8,
    /// 8-bit BGRA, one plane (common X11 / framebuffer layout)
    Bgra8,
    /// YUV 4:2:0 semi-planar; Y plane + interleaved UV plane
    Nv12,
    /// YUV 4:2:0 planar Y U V three separate planes.
    I420,
}

/// Latency / quality trade-off hint passed to the encoder
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncoderPreset {
    /// Minimize latency first 
    #[default]
    Realtime,
    /// Balanced quality and latency
    Balanced,
    /// Maximize compression quality first
    Quality,
}

/// Static encoder configuration negotiated before streaming starts
#[derive(Debug, Clone ,PartialEq, Eq)]
pub struct EncodeConfig {
    /// Frame width in pixels
    pub width: u32,
    /// Frame height in pixels
    pub height: u32,
    /// Target frame rate (per second)
    pub framerate: u32,
    /// Target bitrate (per second)
    pub bitrate_bps: u32,
    /// Compressed codec format
    pub codec: CodecFormat,
    /// Group of picture size in frames (keyframe interval)
    pub gop: u32,
    /// Quality / latency preset
    pub preset: EncoderPreset,
}

impl EncodeConfig {
    ///Create a config with sensible defaults: H.264, realtime preset,
    /// one keyframe per second 
    pub fn new(width: u32, height: u32, framerate: u32, bitrate_bps: u32) -> Self{
        Self { width, height, framerate, bitrate_bps, codec: CodecFormat::H264, gop: framerate.max(1), preset: EncoderPreset::Realtime }
    }

    pub fn validates(&self) -> Result<(), CodecError> {
        if self.width == 0 || self.height == 0 {
            return Err(CodecError::InvalidConfig("frame width and height must be greater than zero".into(),));
        }

        //YUV 4:2:0 requires even dimensions (U / V planes are W / 2 * H / 2)
        if self.width % 2 != 0 || self.height % 2 != 0{
            return Err(CodecError::InvalidConfig(format!("4:2:0 requires even dimensions, got {}x{}",self.width, self.height)));
        }

        if self.framerate == 0{
            return Err(CodecError::InvalidConfig("framerate must be greater than zero".into(),));
        }

        if self.bitrate_bps == 0{
            return Err(CodecError::InvalidConfig("bitrate_bps must greater than zero".into(),));
        }

        Ok(())
    }
}

/// A single image plane
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane{
    /// Plane pixel bytes (owned)
    pub data: Vec<u8>,
    /// Bytes per pixel row (may be padded / aligned by the producer)
    pub stride: u32,
}

///A new input frame handed to the encoder
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel format of the planes.
    pub format: PixelFormat,
    /// Image planes, ordered as required by [`PixelFormat`].
    pub planes: Vec<Plane>,
    /// Monotonic timestamp in microseconds.
    pub timestamp_us: u64,
}

impl Frame{
    // Build an RGBA frame
    pub fn new_rgba(width: u32, height: u32, data: Vec<u8>, timestamp_us: u64) -> Result<Self, CodecError> {
        // checked_mul chain guards against 32-bit overflow / huge input.
        let expected = (width as usize).checked_mul(height as usize).and_then(|px| px.checked_mul(4)).ok_or(CodecError::OutOfMemory)?;
        if data.len() < expected {
            return Err(CodecError::InvalidConfig(format!("RGBA8 buffer too small: expected at least {} bytes, got {}", expected, data.len())));
        }
        Ok(Self{width, height, format: PixelFormat::Rgba8, planes: vec![Plane {data, stride: width * 4}], timestamp_us})
    }

    // Build a BGRA frame 
    pub fn new_bgra(width: u32, height: u32, data: Vec<u8>, timestamp_us: u64) -> Result<Self, CodecError> {
        // checked_mul chain guards against 32-bit overflow / huge input.
        let expected = (width as usize).checked_mul(height as usize).and_then(|px| px.checked_mul(4)).ok_or(CodecError::OutOfMemory)?;
        if data.len() < expected {
            return Err(CodecError::InvalidConfig(format!("RGBA8 buffer too small: expected at least {} bytes, got {}", expected, data.len())));
        }
        Ok(Self{width, height, format: PixelFormat::Bgra8, planes: vec![Plane {data, stride: width * 4}], timestamp_us})
    }

    // Build an I420 frame 
    pub fn new_i420(width: u32, height: u32, y: Vec<u8>, u: Vec<u8>, v: Vec<u8>, timestamp_us: u64) -> Result<Self, CodecError> {
        if width % 2 != 0 || height % 2 != 0 {
            return Err(CodecError::InvalidConfig("I420 requires even width and height".into(),));
        }
        
        let y_len = (width as usize) * (height as usize);
        let uv_len = y_len / 4;
        if y.len() < y_len || u.len() < uv_len || v.len() < uv_len {
            return Err(CodecError::InvalidConfig("I420 plane buffer size mismatch".into(),));
        }

        Ok(Self{width, height, format: PixelFormat::I420, planes: vec![
            Plane {data: y, stride: width},
            Plane {data: u, stride: width / 2},
            Plane {data: v, stride: width / 2},
        ], timestamp_us})
    }

    // Normalize to I420 so downstream consumers only handle one format
    pub fn to_i420(&self) -> Result<Frame, CodecError> {
        match self.format {
            PixelFormat::I420 => Ok(self.clone()),
            PixelFormat::Rgba8 => {
                let plane = self.planes.first().ok_or_else(|| CodecError::InvalidConfig("missing RGBA plane".into()))?;
                let (y, u ,v) = color::rgba_to_i420(self.width, self.height, &plane.data, plane.stride)?;
                Frame::new_i420(self.width, self.height, y, u, v, self.timestamp_us)
            }
            PixelFormat::Bgra8 => {
                let plane = self.planes.first().ok_or_else(|| CodecError::InvalidConfig("missing RGBA plane".into()))?;
                let (y, u ,v) = color::bgra_to_i420(self.width, self.height, &plane.data, plane.stride)?;
                Frame::new_i420(self.width, self.height, y, u, v, self.timestamp_us)
            }
            PixelFormat::Nv12 => {
                if self.planes.len() < 2 {
                    return Err(CodecError::InvalidConfig("NV12 requires 2 planes".into()));
                }
                let y_plane = &self.planes[0];
                let uv_plane = &self.planes[1];
                let (u, v) = color::nv12_to_i420(self.width, self. height, &uv_plane.data, uv_plane.stride)?;
                Frame::new_i420(self.width, self.height, y_plane.data.clone(), u, v, self.timestamp_us)
            }
        }
    }

    //Normalize to NV12 for hardware that only accepts NV12
    pub fn to_nv12(&self) -> Result<Frame, CodecError> {
        match self.format {
            PixelFormat::Nv12 => Ok(self.clone()),
            _=> {
                let i420 = self.to_i420()?;
                let y = i420.planes[0].data.clone();
                let uv = color::i420_uv_to_nv12(&i420.planes[1].data, &i420.planes[2].data);

                Ok(Frame{
                    width: self.width, height: self.height, format: PixelFormat::Nv12, planes: vec![
                        Plane {data: y, stride: self.width},
                        Plane {data: uv, stride: self.width},
                    ], 
                    timestamp_us: self.timestamp_us,
                })
            }
        }
    }
}

/// one encoded network abstraction unit. For H264 / H265 this is an Annex-B
/// NALU (start code + payload)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nalu {
    /// NALU bytes including Annex-B start code
    pub data: Vec<u8>,
    /// Timestamp in microseconds, matching the source frame
    pub timestamp_us: u64,
    /// Whether this NALU belongs to a keyframe
    pub is_keyframe: bool,
}

/// Errors shared by every encoder backend.
#[derive(Debug, Error)]
pub enum CodecError {
    /// Encoder has not been configured yet.
    #[error("encoder is not initialized")]
    NotInitialized,

    /// The encoder rejected the supplied configuration.
    #[error("invalid encoder config:{0}")]
    InvalidConfig(String),

    /// The pixel format is not supported by this backend
    #[error("unsupported pixel format: {0:?}")]
    UnsupportedFormat(PixelFormat),

    /// The hardware backend is not present or not usable on this machine
    #[error("hardware backend unavailable: {0}")]
    BackendUnavailable(String),

    ///Generic encoder-side failure (ioctl, driver, queue, ...)
    #[error("encoder failure: {0}")]
    EncoderFailure(String),
    
    ///Buffer / Memory allocation failed
    #[error("out of memory or Buffer allocation failed")]
    OutOfMemory,

    ///Underlying I/O failure
    #[error(transparent)]
    IO(#[from] std::io::Error )
}

/// Abstraction over a hardware (or fallback software) H.264/H.265 encoder.
/// 
/// Lifecycle:
/// [`HwEncoder::configure`] once with an [`EncodeConfig`]
/// Repeatedly [`HwEncoder::send_frame`] followed by [`HwEncoder::poll_nalu`].
/// [`HwEncoder::flush`] at the end to drain buffered frames.
pub trait HwEncoder: std::fmt::Debug + Send {
    ///Apply (or negotiate) the encoder configuration
    fn configure(&mut self, config: &EncodeConfig) -> Result<(), CodecError>;

    /// Feed one raw frame to the encoder
    fn feed_frame(&mut self, frame:Frame) -> Result<(), CodecError>;

    ///Drain encoded NALUs produced since the last call
    ///Produced NALUs are appended to `out`; returns how many were appended
    fn poll_nalu(&mut self, out: &mut Vec<Nalu>) -> Result<usize, CodecError>;

    ///Signal end of input and flush any remaining buffered frames
    fn flush(&mut self) -> Result<(), CodecError>;

    ///Name for the user, like "rk-mpp, tegra-nvenc"
    fn backend_name(&self) -> &str;

    fn force_keyframe(&mut self) -> Result<(), CodecError>;
}

/// A factory that probes for a backend and constructs an encoder on demand
/// 
/// Used by the streamer to detect available hardware at runtime and fall back to another backend (including software encoder)
/// when the preferred one is unavailable
pub trait EncoderFactory: std::fmt::Debug + Send + Sync {
    /// Backend name, matching [`HwEncoder::backend_name`].
    fn name(&self) -> &str;
    
    /// Whether this backend can run on the current machine.
    fn available(&self) -> bool;

    ///Construct a configured encoder for the given config
    fn create(&self, config: &EncodeConfig) -> Result<Box<dyn HwEncoder>, CodecError>;
}

/// Backend selection: try in caller's order, fall back on failure
/// Finally use software encoding (need turn on in feature) or return "BackendUnavailable"
pub fn create_best_encoder(factories: &[&dyn EncoderFactory], config: &EncodeConfig,) -> Result<Box<dyn HwEncoder>, CodecError> {
    config.validates()?;
    for factory in factories {

        // Skip factories whose probe reports unavailable (e.g. no GPU)
        if factory.available() {
            match factory.create(config) {
                Ok(enc) => {
                    tracing::info!(backend = factory.name(), "Initialized hardware encoder");
                    return Ok(enc);
                }
                Err(err) => {
                    // A single factory failure is not fatal; keep trying the next one
                    tracing::warn!(
                        backend = factory.name(),
                        error = %err, 
                        "Hardware encoder failed initialization, trying next backend"
                    );
                }
            }
        }
    }
    // Software fallback: always available when the feature is on (H264 only)
    #[cfg(feature = "openh264-backend")]
    {
        let fallback = soft_openh264::OpenH264Factory;
        tracing::info!(backend = fallback.name(), "Falling back to OpenH264 software encoder");
        return  fallback.create(config);
    }
     // Feature off and no hardware backend: return a clear environment error.
    #[cfg(not(feature = "openh264-backend"))]
    Err(CodecError::BackendUnavailable("no hardware encoder available and openh264-backend feature is disabled".into(),))
}