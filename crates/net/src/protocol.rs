use anyhow::{bail, Result};

pub const MAGIC: [u8; 2] = [0x47, 0x53]; // "GS"
pub const HEADER_LEN: usize = 20;
pub const MAX_FRAGMENT_PAYLOAD: usize = 1200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    VideoFragment = 0x01,
    Audio = 0x02,
    Input = 0x03,
    Ping = 0x04,
    Feedback = 0x05,
    Punch = 0x06,
}

impl TryFrom<u8> for PacketType {
    type Error = anyhow::Error;

    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            0x01 => Ok(PacketType::VideoFragment),
            0x02 => Ok(PacketType::Audio),
            0x03 => Ok(PacketType::Input),
            0x04 => Ok(PacketType::Ping),
            0x05 => Ok(PacketType::Feedback),
            0x06 => Ok(PacketType::Punch),
            _ => bail!("Unknown packet type: 0x{:02x}", val),
        }
    }
}

pub mod flags {
    pub const KEYFRAME: u8 = 0x01;
    pub const ENCRYPTED: u8 = 0x02;
    pub const LAST_FRAGMENT: u8 = 0x04;
}

#[derive(Debug, Clone)]
pub struct PacketHeader {
    pub packet_type: PacketType,
    pub flags: u8,
    pub sequence: u64,
    pub timestamp_us: u64,
}

impl PacketHeader {
    pub fn is_keyframe(&self) -> bool {
        (self.flags & flags::KEYFRAME) != 0
    }

    pub fn is_encrypted(&self) -> bool {
        (self.flags & flags::ENCRYPTED) != 0
    }

    pub fn is_last_fragment(&self) -> bool {
        (self.flags & flags::LAST_FRAGMENT) != 0
    }

    pub fn encode(&self, out: &mut [u8]) -> Result<()> {
        if out.len() < HEADER_LEN {
            bail!("Output buffer too small for header");
        }
        out[0..2].copy_from_slice(&MAGIC);
        out[2] = self.packet_type as u8;
        out[3] = self.flags;
        out[4..12].copy_from_slice(&self.sequence.to_be_bytes());
        out[12..20].copy_from_slice(&self.timestamp_us.to_be_bytes());
        Ok(())
    }

    pub fn decode(src: &[u8]) -> Result<Self> {
        if src.len() < HEADER_LEN {
            bail!("Buffer too short for packet header ({} bytes)", src.len());
        }
        if src[0..2] != MAGIC {
            bail!("Invalid magic header: {:02x?}", &src[0..2]);
        }
        let packet_type = PacketType::try_from(src[2])?;
        let flags = src[3];
        let sequence = u64::from_be_bytes(src[4..12].try_into().unwrap());
        let timestamp_us = u64::from_be_bytes(src[12..20].try_into().unwrap());

        Ok(Self {
            packet_type,
            flags,
            sequence,
            timestamp_us,
        })
    }
}

/// Video fragment payload representation
#[derive(Debug, Clone)]
pub struct VideoFragment {
    pub frame_id: u32,
    pub frag_idx: u16,
    pub total_frags: u16,
    pub data: Vec<u8>,
}

impl VideoFragment {
    pub const FRAGMENT_HEADER_LEN: usize = 8;

    pub fn encode_payload(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::FRAGMENT_HEADER_LEN + self.data.len());
        buf.extend_from_slice(&self.frame_id.to_be_bytes());
        buf.extend_from_slice(&self.frag_idx.to_be_bytes());
        buf.extend_from_slice(&self.total_frags.to_be_bytes());
        buf.extend_from_slice(&self.data);
        buf
    }

    pub fn decode_payload(src: &[u8]) -> Result<Self> {
        if src.len() < Self::FRAGMENT_HEADER_LEN {
            bail!("Fragment body too short");
        }
        let frame_id = u32::from_be_bytes(src[0..4].try_into().unwrap());
        let frag_idx = u16::from_be_bytes(src[4..6].try_into().unwrap());
        let total_frags = u16::from_be_bytes(src[6..8].try_into().unwrap());
        let data = src[Self::FRAGMENT_HEADER_LEN..].to_vec();

        Ok(Self {
            frame_id,
            frag_idx,
            total_frags,
            data,
        })
    }
}

/// Audio packet payload representation
#[derive(Debug, Clone)]
pub struct AudioPacket {
    pub audio_seq: u32,
    pub opus_data: Vec<u8>,
}

impl AudioPacket {
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.opus_data.len());
        buf.extend_from_slice(&self.audio_seq.to_be_bytes());
        buf.extend_from_slice(&self.opus_data);
        buf
    }

    pub fn decode_payload(src: &[u8]) -> Result<Self> {
        if src.len() < 4 {
            bail!("Audio packet body too short");
        }
        let audio_seq = u32::from_be_bytes(src[0..4].try_into().unwrap());
        let opus_data = src[4..].to_vec();
        Ok(Self {
            audio_seq,
            opus_data,
        })
    }
}

/// Network feedback from client to host for dynamic bitrate/resolution adaptation
#[derive(Debug, Clone, Copy)]
pub struct NetworkFeedback {
    pub ack_frame_id: u32,
    pub loss_rate_bp: u16,   // Basis points (100 = 1.0%)
    pub rtt_ms: u16,
    pub request_idr: bool,
    pub suggested_profile: u8,
}

impl NetworkFeedback {
    pub const LEN: usize = 10;

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut buf = [0u8; Self::LEN];
        buf[0..4].copy_from_slice(&self.ack_frame_id.to_be_bytes());
        buf[4..6].copy_from_slice(&self.loss_rate_bp.to_be_bytes());
        buf[6..8].copy_from_slice(&self.rtt_ms.to_be_bytes());
        buf[8] = if self.request_idr { 1 } else { 0 };
        buf[9] = self.suggested_profile;
        buf
    }

    pub fn decode(src: &[u8]) -> Result<Self> {
        if src.len() < Self::LEN {
            bail!("Feedback buffer too short");
        }
        let ack_frame_id = u32::from_be_bytes(src[0..4].try_into().unwrap());
        let loss_rate_bp = u16::from_be_bytes(src[4..6].try_into().unwrap());
        let rtt_ms = u16::from_be_bytes(src[6..8].try_into().unwrap());
        let request_idr = src[8] != 0;
        let suggested_profile = src[9];

        Ok(Self {
            ack_frame_id,
            loss_rate_bp,
            rtt_ms,
            request_idr,
            suggested_profile,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_header_encode_decode() {
        let header = PacketHeader {
            packet_type: PacketType::VideoFragment,
            flags: flags::KEYFRAME | flags::ENCRYPTED,
            sequence: 123456789,
            timestamp_us: 987654321,
        };
        let mut buf = [0u8; HEADER_LEN];
        header.encode(&mut buf).unwrap();

        let decoded = PacketHeader::decode(&buf).unwrap();
        assert_eq!(decoded.packet_type, PacketType::VideoFragment);
        assert!(decoded.is_keyframe());
        assert!(decoded.is_encrypted());
        assert!(!decoded.is_last_fragment());
        assert_eq!(decoded.sequence, 123456789);
        assert_eq!(decoded.timestamp_us, 987654321);
    }

    #[test]
    fn test_video_fragment_encode_decode() {
        let frag = VideoFragment {
            frame_id: 42,
            frag_idx: 3,
            total_frags: 10,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        let payload = frag.encode_payload();
        let decoded = VideoFragment::decode_payload(&payload).unwrap();
        assert_eq!(decoded.frame_id, 42);
        assert_eq!(decoded.frag_idx, 3);
        assert_eq!(decoded.total_frags, 10);
        assert_eq!(decoded.data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn test_feedback_encode_decode() {
        let fb = NetworkFeedback {
            ack_frame_id: 1024,
            loss_rate_bp: 150,
            rtt_ms: 25,
            request_idr: true,
            suggested_profile: 2,
        };
        let bytes = fb.encode();
        let decoded = NetworkFeedback::decode(&bytes).unwrap();
        assert_eq!(decoded.ack_frame_id, 1024);
        assert_eq!(decoded.loss_rate_bp, 150);
        assert_eq!(decoded.rtt_ms, 25);
        assert!(decoded.request_idr);
        assert_eq!(decoded.suggested_profile, 2);
    }
}
