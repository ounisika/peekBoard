use std::{io::{self, Read, Write}};

use crate::{CodecError, Nalu};

// Safety limit for a single NALU payload (16 MiB)
pub const MAX_NALU_PACKET_BYTES: usize = 16 * 1024 * 1024;

// Default streaming port
pub const DEFAULT_STREAM_PORT: u16 = 9527;

// Default board IP on the RNDIS network interface
pub const DEFAULT_RNDIS_BOARD_IP: &str = "192.168.55.1";

// Write one frame 
pub fn write_nalu_packet<W: Write>(writer: &mut W, nalu: &Nalu) -> Result<(), CodecError>{
    // The reader treats zero length as protocol error
    if nalu.data.is_empty(){
        return Ok(());
    }

    // check length 
    if nalu.data.len() > MAX_NALU_PACKET_BYTES {
        return Err(CodecError::InvalidConfig(format!("Nalu payload size {} exceeds maximum allowed {} bytes", nalu.data.len(), MAX_NALU_PACKET_BYTES)));
    }

    let len_bytes = (nalu.data.len() as u32).to_be_bytes();
    writer.write_all(&len_bytes)?;
    writer.write_all(&nalu.data)?;

    Ok(())
}

// Read one frame
pub fn read_nalu_packet<R: Read>(reader: &mut R, timestamp_us: u64) -> Result<Option<Nalu>, CodecError> {

    // Read 4-byte length prefix 
    let mut len_buf =[0u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        // EOF at a frame boundary means "end of stream": return None 
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None), 
        // Other IO errors
        Err(err) => return Err(CodecError::IO(err)),
    }

    let payload_len = u32::from_be_bytes(len_buf) as usize;
    if payload_len == 0 {
        return Err(CodecError::EncoderFailure("received invalid zero-length NALU frame header".into(),));
    }
    if payload_len > MAX_NALU_PACKET_BYTES {
        return Err(CodecError::EncoderFailure(format!("received NALU length {} exceeding safety limit {}", payload_len, MAX_NALU_PACKET_BYTES)));
    }

    // Read payload 
    let mut data = vec![0u8; payload_len];
    reader.read_exact(&mut data)?;  
    let is_keyframe = detect_h264_keyframe (& data);

    Ok(Some(Nalu {data, timestamp_us, is_keyframe}))
}

//Scan H.264 Annex-B start codes to decide whether the bitstream is a keyframe 
pub fn detect_h264_keyframe(bitstream: &[u8]) -> bool {
    let mut i = 0;
    let len = bitstream.len();

    //Outer bound guarantees bitstream[i..i+3] is in range
    while i + 3 < len {

        // Recognize start codes: 3-byte 00 00 01 or 4-byte 00 00 00 01
        let nal_header_idx = if bitstream[i] == 0 && bitstream[i + 1] == 0 && bitstream[i + 2] == 1 {
            Some(i + 3)
        } else if i + 4 < len && bitstream[i] == 0 && bitstream[i + 1] == 0 && bitstream[i + 2] == 0 && bitstream[i + 3] == 1 {
            Some (i + 4)
        } else {
            None
        };

        if let Some(hdr_idx) = nal_header_idx  {
            if hdr_idx < len {
                let nal_type = bitstream[hdr_idx] & 0x1F;
                if nal_type == 5 || nal_type == 7 {
                    return true;
                }     
            }
            //skip past the header and keep scanning
            i = hdr_idx + 1;
        } else {
            i += 1;
        }
    }
    false
}