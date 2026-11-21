//! Unfiled hardware codec abstraction for Peekboard
//! 
//! This crate is platform-agnostic. It defines the traits and data structures that sit between the screen-capture /streaming logic and per vendor
//! hardware encoder backends(Tegra NVENC, Rockchip MPP, Intel oneVPL, ...)
//! 
//! Backends depend on this crate and implement [`HwEncoder`];
//! the upper layers (capture, streamer, viewer) only ever talk to the trait, never to a specific vendor API

#![deny(missing_debug_implementations)]

use thiserror::Error;

pub mod color;

pub mod protocol;

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
    /// Quality / lateucy perset
    pub perset: EncoderPreset,
}

impl EncodeConfig {
    ///Create a config with senible defaults: H.264, realtime perset,
    /// one keyframe per second 
    pub fn new(width: u32, height: u32, framerate: u32, bitrate_bps: u32) -> Self{
        Self { width, height, framerate, bitrate_bps, codec: CodecFormat::H264, gop: framerate.max(1), perset: EncoderPreset::Realtime }
    }
}

/// A single iamge plane
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane{
    /// Plane pixel bytes (owned)
    pub data: Vec<u8>,
    /// Bytes per pixels row (maybe be padded / aligned by the producer)
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

/// one encoded network abstraction unit. For H264 / H265 this is an Annex-B
/// NALU (start code + payload)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nalu {
    /// NALU bytes including Annex-B start code
    pub data: Vec<u8>,
    /// Timestamp in mricoseconds, matching the source frame
    pub timestamp_us: u64,
    /// Wether this NALU belongs to keyframe
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
    UnsupportedFromat(PixelFormat),

    /// The hardware backend is not preset or not usable on this machine
    #[error("hardware backend unavailable: {0}")]
    BackendUnavailable(String),

    ///Geneic encoder-side failure (ioctl, driver, queue, ...)
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
    ///Apply (or negoiate) the encoder congfiguaration
    fn configure(&mut self, config: &EncodeConfig) -> Result<(), CodecError>;

    /// Feed one raw frame for encoder
    fn feed_frame(&mut self, frame:Frame) -> Result<(), CodecError>;

    ///Drain encoded NALUs produced since the last call
    ///Produced NALUs are appended to `out`; returns how many were appended
    fn poll_nalu(&mut self, out: &mut Vec<Nalu>) -> Result<usize, CodecError>;

    ///Signal end of input and flush any remaining buffered frames
    fn flush(&mut self) -> Result<(), CodecError>;

    ///Name For user like "rk-mpp, tegra-nevnc"
    fn backend_name(&self) -> &str;
}

/// A factory taht probes for a backend and constructs an encoder on demand
/// 
/// Used by the steamer to detect availbale hardware at runtime and fail back to anthoer backend (including software encoder)
/// when the preferrd is unavailable
pub trait EncoderFactory: std::fmt::Debug + Send + Sync {
    /// Backend name, matching [`HwEncoder::backend_name`].
    fn name(&self) -> &str;
    
    /// Whether this backend can run on the current machine.
    fn available(&self) -> bool;

    ///Construct a configured encoder for the given config
    fn create(&self, config: &EncodeConfig) -> Result<Box<dyn HwEncoder>, CodecError>;
}