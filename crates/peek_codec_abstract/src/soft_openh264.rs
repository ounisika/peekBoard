use std::collections::VecDeque;

use openh264::encoder::{self, Encoder, EncoderConfig, FrameType};
use openh264::formats::YUVSlices;
use openh264::OpenH264API;

use crate::protocol::detect_h264_keyframe;
use crate::{CodecError, CodecFormat, EncodeConfig, EncoderFactory, Frame, HwEncoder, Nalu};

#[derive(Debug, Clone, Copy, Default)]
pub struct OpenH264Factory;

// Factory implementation - crates concrete encoder instances 
impl EncoderFactory for OpenH264Factory {
    fn name(&self) -> &str { "openh264-soft" }
    fn available(&self) -> bool{ true }

    //new then configure; spilts failure stages: new shouildn`t fail, configure vaildates config and loads the library 
    fn create(&self, config: &EncodeConfig) -> Result<Box<dyn HwEncoder>, CodecError> {
        let mut encoder = OpenH264Encoder::new();
        encoder.configure(config)?;
        Ok(Box::new(encoder))
    }
}

//Encoder state - Option fields model a "configured/ unconfigured" state machine
pub struct OpenH264Encoder {
    // Config: None means "configure" has not been called
    config: Option<EncodeConfig>,

    // Underlying OpenH264 encoder: None means not initialized yet
    inner: Option<Encoder>,
    
    // Output queue: NALUS producted by `encode` are buffered here until drained by `poll_nalu`
    pending_nalus: VecDeque<Nalu>,

    // Frame counter: drives periodic keyframe insertion via `frame_counter % gap`
    frame_counter: u64,

    // Once forced IDR flag: the next frame is forced to be an I frame
    force_next_idr: bool,
}

// Manual Debug: `Option<Encoder>` doesn't implement Debug, just unpack the interesting fields manually
impl std::fmt::Debug for OpenH264Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenH264Encoder")
        .field("config", &self.config)
        .field("initialized", &self.inner.is_some())
        .field("pending_nalus", &self.pending_nalus.len())
        .field("frame_counter", &self.frame_counter)
        .finish()
    }
}

impl Default for OpenH264Encoder {
    fn default() -> Self {
        Self::new()
    }
}