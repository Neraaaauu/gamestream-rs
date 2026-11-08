use crate::protocol::{VideoFragment, MAX_FRAGMENT_PAYLOAD};
use std::collections::HashMap;

/// Result of complete frame reassembly
#[derive(Debug)]
pub struct CompleteFrame {
    pub frame_id: u32,
    pub is_keyframe: bool,
    pub data: Vec<u8>,
}

/// Splits H.264 video frames into MTU-safe fragments
#[derive(Debug, Clone, Copy)]
pub struct VideoFragmenter {
    mtu_payload: usize,
}

impl Default for VideoFragmenter {
    fn default() -> Self {
        Self {
            mtu_payload: MAX_FRAGMENT_PAYLOAD,
        }
    }
}

impl VideoFragmenter {
    pub fn new(mtu_payload: usize) -> Self {
        Self { mtu_payload }
    }

    pub fn fragment(&self, frame_id: u32, data: &[u8]) -> Vec<VideoFragment> {
        if data.is_empty() {
            return Vec::new();
        }

        let chunk_size = self.mtu_payload;
        let total_frags = data.len().div_ceil(chunk_size) as u16;
        let mut fragments = Vec::with_capacity(total_frags as usize);

        for (idx, chunk) in data.chunks(chunk_size).enumerate() {
            fragments.push(VideoFragment {
                frame_id,
                frag_idx: idx as u16,
                total_frags,
                data: chunk.to_vec(),
            });
        }

        fragments
    }
}

/// In-progress frame reassembly buffer
struct PendingFrame {
    total_frags: u16,
    received_count: u16,
    fragments: HashMap<u16, Vec<u8>>,
    is_keyframe: bool,
}

/// Low-latency frame reassembler with zero-stalling drop policy
pub struct FrameAssembler {
    highest_frame_id: u32,
    pending: Option<(u32, PendingFrame)>,
    // Statistics for loss reporting
    total_packets_expected: u64,
    total_packets_received: u64,
}

impl Default for FrameAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self {
            highest_frame_id: 0,
            pending: None,
            total_packets_expected: 0,
            total_packets_received: 0,
        }
    }

    /// Process an incoming fragment. Returns Some(CompleteFrame) when all fragments for the newest frame arrive.
    pub fn push_fragment(&mut self, frag: VideoFragment, is_keyframe: bool) -> Option<CompleteFrame> {
        self.total_packets_received += 1;

        let frame_id = frag.frame_id;

        // Discard fragments from older frames (strict low-latency: never stall on old frames)
        if frame_id < self.highest_frame_id {
            return None;
        }

        // New newer frame arrived: drop previous incomplete frame to keep latency minimal
        if frame_id > self.highest_frame_id {
            if let Some((_, old_pending)) = self.pending.take() {
                self.total_packets_expected += old_pending.total_frags as u64;
            }
            self.highest_frame_id = frame_id;
            self.pending = Some((
                frame_id,
                PendingFrame {
                    total_frags: frag.total_frags,
                    received_count: 0,
                    fragments: HashMap::with_capacity(frag.total_frags as usize),
                    is_keyframe,
                },
            ));
        }

        let pending_entry = self.pending.as_mut().unwrap();
        let pending = &mut pending_entry.1;

        // If duplicate fragment, ignore
        if pending.fragments.contains_key(&frag.frag_idx) {
            return None;
        }

        pending.fragments.insert(frag.frag_idx, frag.data);
        pending.received_count += 1;
        if is_keyframe {
            pending.is_keyframe = true;
        }

        // Check if all fragments arrived
        if pending.received_count >= pending.total_frags {
            let (_, mut completed) = self.pending.take().unwrap();
            self.total_packets_expected += completed.total_frags as u64;

            // Assemble sequentially
            let mut assembled = Vec::new();
            for idx in 0..completed.total_frags {
                let part = completed.fragments.remove(&idx)?;
                assembled.extend_from_slice(&part);
            }

            Some(CompleteFrame {
                frame_id,
                is_keyframe: completed.is_keyframe,
                data: assembled,
            })
        } else {
            None
        }
    }

    /// Calculate packet loss rate in basis points (100 = 1.0%)
    pub fn loss_rate_basis_points(&self) -> u16 {
        if self.total_packets_expected == 0 {
            return 0;
        }
        if self.total_packets_received >= self.total_packets_expected {
            return 0;
        }
        let lost = self.total_packets_expected - self.total_packets_received;
        let bp = (lost as f64 / self.total_packets_expected as f64) * 10000.0;
        bp.min(10000.0) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fragment_and_reassemble() {
        let fragmenter = VideoFragmenter::new(100);
        let sample_data = vec![0xAB; 350]; // Should split into 4 fragments: 100, 100, 100, 50
        let fragments = fragmenter.fragment(1, &sample_data);

        assert_eq!(fragments.len(), 4);
        assert_eq!(fragments[0].data.len(), 100);
        assert_eq!(fragments[3].data.len(), 50);

        let mut assembler = FrameAssembler::new();
        let mut completed = None;

        for frag in fragments {
            completed = assembler.push_fragment(frag, false);
        }

        assert!(completed.is_some());
        let frame = completed.unwrap();
        assert_eq!(frame.frame_id, 1);
        assert_eq!(frame.data.len(), 350);
        assert_eq!(frame.data, sample_data);
    }

    #[test]
    fn test_stale_frame_dropped_on_new_arrival() {
        let fragmenter = VideoFragmenter::new(100);
        let data1 = vec![1u8; 300];
        let data2 = vec![2u8; 300];

        let frags1 = fragmenter.fragment(1, &data1);
        let frags2 = fragmenter.fragment(2, &data2);

        let mut assembler = FrameAssembler::new();
        // Send only 1 fragment of frame 1 (incomplete)
        assert!(assembler.push_fragment(frags1[0].clone(), false).is_none());

        // Now frame 2 arrives completely
        let mut completed2 = None;
        for frag in frags2 {
            completed2 = assembler.push_fragment(frag, true);
        }

        assert!(completed2.is_some());
        let frame2 = completed2.unwrap();
        assert_eq!(frame2.frame_id, 2);
        assert!(frame2.is_keyframe);
        assert_eq!(frame2.data, data2);

        // A late fragment from frame 1 arrives now - should be discarded
        assert!(assembler.push_fragment(frags1[1].clone(), false).is_none());
    }
}
