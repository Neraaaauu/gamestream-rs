use std::net::SocketAddr;
use gamestream_net::adaptive::{AdaptiveController, PROFILES};
use gamestream_net::crypto::{CryptoRole, CryptoSession};
use gamestream_net::protocol::NetworkFeedback;
use gamestream_net::transceiver::{NetEvent, UdpTransceiver};

#[tokio::test]
async fn test_end_to_end_udp_encrypted_streaming() {
    let key = CryptoSession::generate_key();

    let host_bind = "127.0.0.1:18554";
    let client_bind = "127.0.0.1:18555";

    let host_addr: SocketAddr = host_bind.parse().unwrap();
    let client_addr: SocketAddr = client_bind.parse().unwrap();

    let (host_sender, _) = UdpTransceiver::bind(
        host_bind,
        client_addr,
        Some(key),
        CryptoRole::Host,
    )
    .await
    .expect("Failed to bind host transceiver")
    .into_split();

    let (client_sender, mut client_receiver) = UdpTransceiver::bind(
        client_bind,
        host_addr,
        Some(key),
        CryptoRole::Client,
    )
    .await
    .expect("Failed to bind client transceiver")
    .into_split();

    // 1. Host transmits a 5000-byte video frame (will be fragmented and encrypted)
    let fake_h264_frame = vec![0x33u8; 5000];
    host_sender.send_video_frame(101, true, &fake_h264_frame)
        .await
        .expect("Failed to send video frame");

    let mut rx_buf = [0u8; 2048];
    let mut received_video = None;

    // Receive fragments until complete
    for _ in 0..10 {
        if let Ok(Some(NetEvent::Video(frame))) = client_receiver.recv_event(&mut rx_buf).await {
            received_video = Some(frame);
            break;
        }
    }

    assert!(received_video.is_some(), "Client did not reassemble video frame");
    let v_frame = received_video.unwrap();
    assert_eq!(v_frame.frame_id, 101);
    assert!(v_frame.is_keyframe);
    assert_eq!(v_frame.data.len(), 5000);
    assert_eq!(v_frame.data, fake_h264_frame);

    // 2. Host transmits an Opus audio packet
    let fake_opus = vec![0x77u8; 150];
    host_sender.send_audio_packet(1, &fake_opus)
        .await
        .expect("Failed to send audio");

    let mut received_audio = None;
    for _ in 0..5 {
        if let Ok(Some(NetEvent::Audio(audio))) = client_receiver.recv_event(&mut rx_buf).await {
            received_audio = Some(audio);
            break;
        }
    }
    assert!(received_audio.is_some());
    let a_pkt = received_audio.unwrap();
    assert_eq!(a_pkt.audio_seq, 1);
    assert_eq!(a_pkt.opus_data, fake_opus);

    // 3. Client transmits Input event (mouse movement) back to host
    client_sender.send_input_event(0x01, &[10, 20, 0, 0])
        .await
        .expect("Failed to send input");

    // 4. Client sends Feedback (RTT = 15ms, loss = 0)
    let fb = NetworkFeedback {
        ack_frame_id: 101,
        loss_rate_bp: 0,
        rtt_ms: 15,
        request_idr: false,
        suggested_profile: 0,
    };
    client_sender.send_feedback(&fb).await.expect("Failed to send feedback");

    // 5. Verify dynamic adaptation logic
    let mut adaptive = AdaptiveController::new(0); // 720p60
    assert_eq!(adaptive.current_profile().name, "720p60");
    assert_eq!(adaptive.current_profile().fps, 60);

    // Simulate severe network packet loss (3.5% loss, 120ms RTT)
    let action = adaptive.on_network_feedback(350, 120);
    assert!(action.is_some());
    let act = action.unwrap();
    assert!(act.new_bitrate_bps < PROFILES[0].rec_bitrate_bps);
}
