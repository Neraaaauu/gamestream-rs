use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::net::UdpSocket;
use tracing::{debug, warn};

use crate::crypto::{CryptoRole, CryptoSession};
use crate::fragment::{CompleteFrame, FrameAssembler, VideoFragmenter};
use crate::protocol::{
    flags, AudioPacket, NetworkFeedback, PacketHeader, PacketType, VideoFragment, HEADER_LEN,
};

/// High-level event produced by the UDP transceiver
pub enum NetEvent {
    Video(CompleteFrame),
    Audio(AudioPacket),
    Input { kind: u8, data: Vec<u8> },
    Feedback(NetworkFeedback),
    Ping { timestamp_us: u64 },
}

/// Transceiver wrapper that binds socket and splits into zero-lock sender and receiver
pub struct UdpTransceiver {
    sender: UdpSender,
    receiver: UdpReceiver,
}

impl UdpTransceiver {
    /// Bind a UDP socket and configure peer address
    pub async fn bind(
        local_bind: &str,
        peer_addr: SocketAddr,
        crypto_key: Option<[u8; 32]>,
        role: CryptoRole,
    ) -> Result<Self> {
        let socket = Arc::new(
            UdpSocket::bind(local_bind)
                .await
                .context(format!("Failed to bind UDP socket to {}", local_bind))?,
        );

        let crypto = crypto_key.map(|k| Arc::new(CryptoSession::new(&k, role)));
        let sequence = Arc::new(AtomicU64::new(1));

        let sender = UdpSender {
            socket: socket.clone(),
            peer_addr,
            crypto: crypto.clone(),
            sequence,
            fragmenter: VideoFragmenter::default(),
        };

        let receiver = UdpReceiver {
            socket,
            crypto,
            assembler: FrameAssembler::new(),
        };

        Ok(Self { sender, receiver })
    }

    /// Split into concurrent sender and receiver (zero lock contention)
    pub fn into_split(self) -> (UdpSender, UdpReceiver) {
        (self.sender, self.receiver)
    }

    pub fn sender(&self) -> UdpSender {
        self.sender.clone()
    }

    pub fn receiver_mut(&mut self) -> &mut UdpReceiver {
        &mut self.receiver
    }
}

/// Zero-lock UDP Sender (Cloneable, can be called concurrently from multiple tasks)
#[derive(Clone)]
pub struct UdpSender {
    socket: Arc<UdpSocket>,
    peer_addr: SocketAddr,
    crypto: Option<Arc<CryptoSession>>,
    sequence: Arc<AtomicU64>,
    fragmenter: VideoFragmenter,
}

impl UdpSender {
    pub fn socket_ref(&self) -> Arc<UdpSocket> {
        self.socket.clone()
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    pub fn set_peer_addr(&mut self, addr: SocketAddr) {
        self.peer_addr = addr;
    }

    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::Relaxed)
    }

    fn current_timestamp_us(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }

    /// Send a complete H.264 video frame, fragmented into MTU packets
    pub async fn send_video_frame(&self, frame_id: u32, is_keyframe: bool, data: &[u8]) -> Result<()> {
        let fragments = self.fragmenter.fragment(frame_id, data);
        let total_frags = fragments.len();

        for (idx, frag) in fragments.into_iter().enumerate() {
            let seq = self.next_sequence();
            let mut flg = 0u8;
            if is_keyframe {
                flg |= flags::KEYFRAME;
            }
            if idx == total_frags - 1 {
                flg |= flags::LAST_FRAGMENT;
            }

            let raw_payload = frag.encode_payload();
            let (payload, encrypted_flag) = if let Some(ref crypto) = self.crypto {
                let enc = crypto.encrypt(seq, &raw_payload)?;
                (enc, flags::ENCRYPTED)
            } else {
                (raw_payload, 0)
            };

            flg |= encrypted_flag;

            let header = PacketHeader {
                packet_type: PacketType::VideoFragment,
                flags: flg,
                sequence: seq,
                timestamp_us: self.current_timestamp_us(),
            };

            let mut packet = vec![0u8; HEADER_LEN + payload.len()];
            header.encode(&mut packet[0..HEADER_LEN])?;
            packet[HEADER_LEN..].copy_from_slice(&payload);

            self.socket
                .send_to(&packet, self.peer_addr)
                .await
                .context("Failed to send video fragment UDP packet")?;
        }

        Ok(())
    }

    /// Send an Opus audio packet (fits in a single UDP packet)
    pub async fn send_audio_packet(&self, audio_seq: u32, opus_data: &[u8]) -> Result<()> {
        let seq = self.next_sequence();
        let audio_pkt = AudioPacket {
            audio_seq,
            opus_data: opus_data.to_vec(),
        };
        let raw_payload = audio_pkt.encode_payload();

        let mut flg = 0u8;
        let (payload, encrypted_flag) = if let Some(ref crypto) = self.crypto {
            let enc = crypto.encrypt(seq, &raw_payload)?;
            (enc, flags::ENCRYPTED)
        } else {
            (raw_payload, 0)
        };
        flg |= encrypted_flag;

        let header = PacketHeader {
            packet_type: PacketType::Audio,
            flags: flg,
            sequence: seq,
            timestamp_us: self.current_timestamp_us(),
        };

        let mut packet = vec![0u8; HEADER_LEN + payload.len()];
        header.encode(&mut packet[0..HEADER_LEN])?;
        packet[HEADER_LEN..].copy_from_slice(&payload);

        self.socket
            .send_to(&packet, self.peer_addr)
            .await
            .context("Failed to send audio UDP packet")?;

        Ok(())
    }

    /// Send an Input event (mouse, keyboard, gamepad)
    pub async fn send_input_event(&self, kind: u8, data: &[u8]) -> Result<()> {
        let seq = self.next_sequence();
        let mut raw = Vec::with_capacity(1 + data.len());
        raw.push(kind);
        raw.extend_from_slice(data);

        let mut flg = 0u8;
        let (payload, encrypted_flag) = if let Some(ref crypto) = self.crypto {
            let enc = crypto.encrypt(seq, &raw)?;
            (enc, flags::ENCRYPTED)
        } else {
            (raw, 0)
        };
        flg |= encrypted_flag;

        let header = PacketHeader {
            packet_type: PacketType::Input,
            flags: flg,
            sequence: seq,
            timestamp_us: self.current_timestamp_us(),
        };

        let mut packet = vec![0u8; HEADER_LEN + payload.len()];
        header.encode(&mut packet[0..HEADER_LEN])?;
        packet[HEADER_LEN..].copy_from_slice(&payload);

        self.socket
            .send_to(&packet, self.peer_addr)
            .await
            .context("Failed to send input UDP packet")?;

        Ok(())
    }

    /// Send network feedback (loss stats and RTT)
    pub async fn send_feedback(&self, feedback: &NetworkFeedback) -> Result<()> {
        let seq = self.next_sequence();
        let raw = feedback.encode();

        let header = PacketHeader {
            packet_type: PacketType::Feedback,
            flags: 0,
            sequence: seq,
            timestamp_us: self.current_timestamp_us(),
        };

        let mut packet = vec![0u8; HEADER_LEN + raw.len()];
        header.encode(&mut packet[0..HEADER_LEN])?;
        packet[HEADER_LEN..].copy_from_slice(&raw);

        self.socket
            .send_to(&packet, self.peer_addr)
            .await
            .context("Failed to send feedback UDP packet")?;

        Ok(())
    }
}

/// UDP Receiver running independently without locking the sender
pub struct UdpReceiver {
    socket: Arc<UdpSocket>,
    crypto: Option<Arc<CryptoSession>>,
    assembler: FrameAssembler,
}

impl UdpReceiver {
    /// Receive and process next UDP packet
    pub async fn recv_event(&mut self, buf: &mut [u8]) -> Result<Option<NetEvent>> {
        let (len, from_addr) = self.socket.recv_from(buf).await?;

        // Ignore punch packets
        if &buf[0..len.min(7)] == b"GSPUNCH" {
            debug!("Received UDP hole punch probe from {}", from_addr);
            return Ok(None);
        }

        if len < HEADER_LEN {
            return Ok(None);
        }

        let header = match PacketHeader::decode(&buf[0..HEADER_LEN]) {
            Ok(h) => h,
            Err(e) => {
                debug!("Failed to decode packet header from {}: {}", from_addr, e);
                return Ok(None);
            }
        };

        let payload_raw = &buf[HEADER_LEN..len];

        // Decrypt if encrypted flag is set
        let decrypted_data;
        let payload = if header.is_encrypted() {
            if let Some(ref crypto) = self.crypto {
                match crypto.decrypt(header.sequence, payload_raw) {
                    Ok(dec) => {
                        decrypted_data = dec;
                        &decrypted_data[..]
                    }
                    Err(e) => {
                        warn!("Failed to decrypt packet seq {}: {}", header.sequence, e);
                        return Ok(None);
                    }
                }
            } else {
                warn!("Received encrypted packet but no crypto key configured");
                return Ok(None);
            }
        } else {
            payload_raw
        };

        match header.packet_type {
            PacketType::VideoFragment => {
                let frag = VideoFragment::decode_payload(payload)?;
                let complete = self.assembler.push_fragment(frag, header.is_keyframe());
                Ok(complete.map(NetEvent::Video))
            }
            PacketType::Audio => {
                let audio = AudioPacket::decode_payload(payload)?;
                Ok(Some(NetEvent::Audio(audio)))
            }
            PacketType::Input => {
                if payload.is_empty() {
                    return Ok(None);
                }
                let kind = payload[0];
                let data = payload[1..].to_vec();
                Ok(Some(NetEvent::Input { kind, data }))
            }
            PacketType::Feedback => {
                let fb = NetworkFeedback::decode(payload)?;
                Ok(Some(NetEvent::Feedback(fb)))
            }
            PacketType::Ping => Ok(Some(NetEvent::Ping {
                timestamp_us: header.timestamp_us,
            })),
            PacketType::Punch => Ok(None),
        }
    }

    /// Current loss rate in basis points (100 = 1.0%)
    pub fn current_loss_rate_bp(&self) -> u16 {
        self.assembler.loss_rate_basis_points()
    }
}
