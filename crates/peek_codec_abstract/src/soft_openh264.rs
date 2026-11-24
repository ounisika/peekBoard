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

impl OpenH264Encoder {
    pub fn new() -> Self{
        Self {
            config: None, inner: None, pending_nalus: VecDeque::new(), frame_counter: 0, 
            // Initially force the next frame to be IDR so decodedrs can start from a clean state
            force_next_idr: true,
        }
    }
}

// HwEncoder trait implmentation
impl HwEncoder for OpenH264Encoder {
    // Configure and initialise the encoder Indempotent; resets state
    fn configure(&mut self, config: &EncodeConfig) -> Result<(), CodecError> {
        config.vaildates()?;
        
        //Software backend only supports H.264
        if config.codec != CodecFormat::H264 {
            return Err(CodecError::InvalidConfig("OpenH264 software fallback only supports CodecFormat::H264".into()));
        }

    //Load lobopenh264 (dlopen on first use)
    let api = OpenH264API::from_source();

    // Builder config: bitrate(bps), max frame-rate, skip-frame disabled
    // enable_skip_frame(false) is crucial: it prevents the encoder from dropping frames to honour bitrate, so every input
    // frame yields an output - required for real-time streaming latency / completness
    let enc_cfg = EncoderConfig::new()
        .set_bitrate_bps(config.bitrate_bps)
        .max_frame_rate(config.framerate as f32)
        .enable_skip_frame(false);
    let encoder = Encoder::with_api_config(api, enc_cfg)
    .map_err(|e| CodecError::EncoderFailure(format!("OpenH264 init failed: {e}")))?;
    
    // Store state and reset counters so `configure` restarts cleanly
    self.config = Some(config.clone());
    self.inner = Some(encoder);
    self.pending_nalus.clear();
    self.frame_counter = 0;
    self.force_next_idr = true;
    Ok(())
    }
    
    // Feed one raw pixel frame; encoding results go into pending_nalus 
    fn feed_frame(&mut self, frame:Frame) -> Result<(), CodecError> {
        // Split the borrow: config used for validation, encoder needs &mut; 
        //both borrowed simultaneously
        let cfg = self.config.as_ref().ok_or(CodecError::NotInitialized)?;
        let encoder = self.inner.as_mut().ok_or(CodecError::NotInitialized)?;

        //Dimensions must match config; otherwise the encoder's behaviour is undefined
        if frame.width != cfg.width || frame.height != cfg.height {
            return Err(CodecError::InvalidConfig(format!(
                "frame dimensions {}x{} do not match configured {}x{}",
                frame.width, frame.height, cfg.width, cfg.height
            )));
        }

        let i420 = frame.to_i420()?;

        // Keyframe scheduling
        // Froce the currnet frame to IDR when either:
        // - one-shot flag set after configure() / force_keyframe()
        // - GOP period reached
        let gop = cfg.gop.max(1) as u64;
        if self.force_next_idr || (self.frame_counter % gop ==0) {
            encoder.force_intra_frame();
            self.force_next_idr = false;
        }

        // YUVSlices 借用三平面 + 宽高 + 每平面 stride，交给编码器
        // YUVSlices borrows the three planes plus dimensions and per-plane strides, feeding them to the encoder 
        let yuv = YUVSlices::new(
        (
                &i420.planes[0].data,
                &i420.planes[1].data,
                &i420.planes[2].data,   
            ),
            (cfg.width as usize, cfg.height as usize),
            (
                i420.planes[0].stride as usize, 
                i420.planes[1].stride as usize, 
                i420.planes[2].stride as usize,
            )
        );

        let bitstream = encoder
            .encode(&yuv)
            .map_err(|e| CodecError::EncoderFailure(format!("OpenH264 encode failed: {e}")))?;
        
        // Copy out the bytes. An IDR-I frame usually carries SPS/PPS/IDR NAL
        //units; we treat the whole Annex-B bytes stram as one `Nalu` packet
        let data = bitstream.to_vec();

        // Keyframe detection: primary via encoder frame_type, fallback via 
        // bitstream scan
        let is_keyframe = matches!(bitstream.frame_type(), FrameType::IDR | FrameType::I) || detect_h264_keyframe(&data);

        //Empty bitstreams (shouldn't happen) are silently skipped, mirroring the protocol writer's "don't emit empty NALUs" rule
        if !data.is_empty() {
            self.pending_nalus.push_back(Nalu {
                data, 
                timestamp_us: frame.timestamp_us,
                is_keyframe,
            });
        }
        self.frame_counter += 1;
        Ok(())
    }

    // Drain all produced NALUs into `out` and return the count
    fn poll_nalu(&mut self, out: &mut Vec<Nalu>) -> Result<usize, CodecError> {
        if self.inner.is_none() {
            return Err(CodecError::NotInitialized);
        }
        let count = self.pending_nalus.len();

        //drain(..). removes everything an O(1) claer (up to capacity)
        out.extend(self.pending_nalus.drain(..));

        Ok(count)
    }

    // Requesr the next fed frame to be a keyframe (effective asynchronously)
    fn force_keyframe(&mut self) -> Result<(),CodecError> {
        self.force_next_idr = true;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CodecError> {
        if self.inner.is_none() {
            return Err(CodecError::NotInitialized);
        }
        Ok(())
    }

    fn backend_name(&self) -> &str {
        "openh264-soft"
    }
}