use std::time::{Duration, Instant};
use tracing::info;

/// Fixed resolution profiles maintaining non-negotiable 60 FPS
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamProfile {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub min_bitrate_bps: i64,
    pub rec_bitrate_bps: i64,
    pub max_bitrate_bps: i64,
}

pub const PROFILES: [StreamProfile; 4] = [
    // 0: 720p60 (Standard HD)
    StreamProfile {
        name: "720p60",
        width: 1280,
        height: 720,
        fps: 60,
        min_bitrate_bps: 2_500_000,
        rec_bitrate_bps: 4_500_000,
        max_bitrate_bps: 6_000_000,
    },
    // 1: 576p60 (Sub-HD 1024x576)
    StreamProfile {
        name: "576p60",
        width: 1024,
        height: 576,
        fps: 60,
        min_bitrate_bps: 1_800_000,
        rec_bitrate_bps: 3_000_000,
        max_bitrate_bps: 4_500_000,
    },
    // 2: 480p60 (Sub-HD 854x480)
    StreamProfile {
        name: "480p60",
        width: 854,
        height: 480,
        fps: 60,
        min_bitrate_bps: 1_200_000,
        rec_bitrate_bps: 2_000_000,
        max_bitrate_bps: 3_000_000,
    },
    // 3: 360p60 (Sub-HD 640x360)
    StreamProfile {
        name: "360p60",
        width: 640,
        height: 360,
        fps: 60,
        min_bitrate_bps: 600_000,
        rec_bitrate_bps: 1_200_000,
        max_bitrate_bps: 1_800_000,
    },
];

/// Dynamic adaptation decisions produced by the controller
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdaptationAction {
    pub profile_changed: bool,
    pub new_profile: StreamProfile,
    pub new_bitrate_bps: i64,
    pub request_keyframe: bool,
}

/// Dynamic Bitrate & Resolution Adaptation Controller (60 FPS locked)
pub struct AdaptiveController {
    current_profile_idx: usize,
    current_bitrate_bps: i64,
    last_loss_event: Instant,
    stable_start: Instant,
}

impl Default for AdaptiveController {
    fn default() -> Self {
        Self::new(0) // Default to 720p60
    }
}

impl AdaptiveController {
    pub fn new(initial_profile_idx: usize) -> Self {
        let idx = initial_profile_idx.min(PROFILES.len() - 1);
        let prof = &PROFILES[idx];
        let now = Instant::now();

        Self {
            current_profile_idx: idx,
            current_bitrate_bps: prof.rec_bitrate_bps,
            last_loss_event: now,
            stable_start: now,
        }
    }

    pub fn current_profile(&self) -> StreamProfile {
        PROFILES[self.current_profile_idx]
    }

    pub fn current_bitrate(&self) -> i64 {
        self.current_bitrate_bps
    }

    /// Process feedback from network (loss rate in basis points: 100 = 1.0%, RTT in ms)
    pub fn on_network_feedback(&mut self, loss_rate_bp: u16, rtt_ms: u16) -> Option<AdaptationAction> {
        let prof = PROFILES[self.current_profile_idx];
        let is_congested = loss_rate_bp > 200 || rtt_ms > 85;

        if is_congested {
            self.last_loss_event = Instant::now();
            self.stable_start = Instant::now();

            if self.current_bitrate_bps > prof.min_bitrate_bps {
                // Step 1: Reduce bitrate by 20%
                let reduced = (self.current_bitrate_bps as f64 * 0.80) as i64;
                self.current_bitrate_bps = reduced.max(prof.min_bitrate_bps);
                info!(
                    "Congestion detected (loss: {:.2}%, RTT: {}ms). Reducing bitrate to {:.2} Mbps within {}",
                    loss_rate_bp as f64 / 100.0,
                    rtt_ms,
                    self.current_bitrate_bps as f64 / 1_000_000.0,
                    prof.name
                );

                Some(AdaptationAction {
                    profile_changed: false,
                    new_profile: prof,
                    new_bitrate_bps: self.current_bitrate_bps,
                    request_keyframe: false,
                })
            } else if self.current_profile_idx + 1 < PROFILES.len() {
                // Step 2: At minimum bitrate and congestion continues -> Downscale resolution, keep 60 FPS!
                self.current_profile_idx += 1;
                let new_prof = PROFILES[self.current_profile_idx];
                self.current_bitrate_bps = new_prof.rec_bitrate_bps;
                info!(
                    "Severe congestion. DOWNSCALING to {} ({}x{}) while preserving 60 FPS!",
                    new_prof.name, new_prof.width, new_prof.height
                );

                Some(AdaptationAction {
                    profile_changed: true,
                    new_profile: new_prof,
                    new_bitrate_bps: self.current_bitrate_bps,
                    request_keyframe: true,
                })
            } else {
                None
            }
        } else if loss_rate_bp == 0 && rtt_ms < 50 {
            // Channel is healthy. Check how long it has been stable
            if self.stable_start.elapsed() >= Duration::from_secs(4) {
                self.stable_start = Instant::now();

                if self.current_bitrate_bps < prof.max_bitrate_bps {
                    // Probe upward: +10% bitrate
                    let boosted = (self.current_bitrate_bps as f64 * 1.10) as i64;
                    self.current_bitrate_bps = boosted.min(prof.max_bitrate_bps);
                    info!(
                        "Network stable. Increasing bitrate to {:.2} Mbps within {}",
                        self.current_bitrate_bps as f64 / 1_000_000.0,
                        prof.name
                    );

                    Some(AdaptationAction {
                        profile_changed: false,
                        new_profile: prof,
                        new_bitrate_bps: self.current_bitrate_bps,
                        request_keyframe: false,
                    })
                } else if self.current_profile_idx > 0 {
                    // Maximum bitrate reached and channel is clear -> Upgrade resolution!
                    self.current_profile_idx -= 1;
                    let new_prof = PROFILES[self.current_profile_idx];
                    self.current_bitrate_bps = new_prof.rec_bitrate_bps;
                    info!(
                        "Network excellent. UPSCALING to {} ({}x{}) at 60 FPS!",
                        new_prof.name, new_prof.width, new_prof.height
                    );

                    Some(AdaptationAction {
                        profile_changed: true,
                        new_profile: new_prof,
                        new_bitrate_bps: self.current_bitrate_bps,
                        request_keyframe: true,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_profiles_guarantee_60fps() {
        for prof in &PROFILES {
            assert_eq!(prof.fps, 60, "Profile {} must have 60 FPS", prof.name);
            assert!(prof.min_bitrate_bps < prof.rec_bitrate_bps);
            assert!(prof.rec_bitrate_bps < prof.max_bitrate_bps);
        }
    }

    #[test]
    fn test_adaptation_downscaling_on_severe_loss() {
        let mut controller = AdaptiveController::new(0); // 720p60
        assert_eq!(controller.current_profile().name, "720p60");

        // Force to minimum bitrate
        controller.current_bitrate_bps = PROFILES[0].min_bitrate_bps;

        // Send severe packet loss feedback (5%)
        let action = controller.on_network_feedback(500, 100);
        assert!(action.is_some());
        let action = action.unwrap();

        // Must downscale to 576p60 without sacrificing 60 FPS
        assert!(action.profile_changed);
        assert_eq!(action.new_profile.name, "576p60");
        assert_eq!(action.new_profile.fps, 60);
        assert!(action.request_keyframe);
    }
}
