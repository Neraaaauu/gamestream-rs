use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use anyhow::{bail, Context, Result};
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

const STUN_MAGIC_COOKIE: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_RESPONSE: u16 = 0x0101;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// Default public STUN servers for NAT traversal discovery
pub const DEFAULT_STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun2.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

/// Query a STUN server to discover our public reflexive SocketAddr (NAT Traversal RFC 5389)
pub async fn query_stun(socket: &UdpSocket, stun_server: &str) -> Result<SocketAddr> {
    let server_addr: SocketAddr = tokio::net::lookup_host(stun_server)
        .await
        .context(format!("Failed to resolve STUN server host: {}", stun_server))?
        .next()
        .context("No address found for STUN server")?;

    debug!("Sending RFC 5389 STUN Binding Request to {} ({})", stun_server, server_addr);

    // Build 20-byte STUN Binding Request header
    let mut request = [0u8; 20];
    request[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    request[2..4].copy_from_slice(&0u16.to_be_bytes()); // Length: 0 attributes
    request[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());

    // Generate random 12-byte transaction ID using system time / pseudo-random
    let tx_id = generate_transaction_id();
    request[8..20].copy_from_slice(&tx_id);

    // Send request to STUN server
    socket.send_to(&request, server_addr).await
        .context("Failed to send STUN request packet")?;

    let mut buf = [0u8; 512];
    let timeout = Duration::from_millis(2000);

    let (len, from_addr) = tokio::time::timeout(timeout, socket.recv_from(&mut buf))
        .await
        .context("STUN request timed out (2000 ms)")?
        .context("Failed to receive STUN response")?;

    if len < 20 {
        bail!("STUN response too short ({} bytes)", len);
    }

    let msg_type = u16::from_be_bytes([buf[0], buf[1]]);
    let msg_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    let cookie = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);

    if msg_type != BINDING_RESPONSE {
        bail!("Unexpected STUN response type: 0x{:04x}", msg_type);
    }
    if cookie != STUN_MAGIC_COOKIE {
        bail!("Invalid STUN magic cookie: 0x{:08x}", cookie);
    }
    if buf[8..20] != tx_id {
        bail!("STUN transaction ID mismatch");
    }

    // Parse attributes in body
    let mut offset = 20;
    let end = (20 + msg_len).min(len);

    while offset + 4 <= end {
        let attr_type = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
        let attr_len = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]) as usize;
        offset += 4;

        if offset + attr_len > end {
            break;
        }

        let attr_value = &buf[offset..offset + attr_len];
        // Attributes are padded to 4-byte boundaries
        offset += (attr_len + 3) & !3;

        if attr_type == ATTR_XOR_MAPPED_ADDRESS && attr_value.len() >= 8 {
            let family = attr_value[1];
            let xor_port = u16::from_be_bytes([attr_value[2], attr_value[3]]);
            let port = xor_port ^ ((STUN_MAGIC_COOKIE >> 16) as u16);

            if family == 0x01 && attr_value.len() >= 8 {
                // IPv4
                let xor_ip = u32::from_be_bytes([
                    attr_value[4],
                    attr_value[5],
                    attr_value[6],
                    attr_value[7],
                ]);
                let ip_u32 = xor_ip ^ STUN_MAGIC_COOKIE;
                let ip = Ipv4Addr::from(ip_u32);
                let addr = SocketAddr::new(IpAddr::V4(ip), port);
                info!("STUN Reflexive Public Address resolved (XOR-MAPPED): {}", addr);
                return Ok(addr);
            }
        } else if attr_type == ATTR_MAPPED_ADDRESS && attr_value.len() >= 8 {
            let family = attr_value[1];
            let port = u16::from_be_bytes([attr_value[2], attr_value[3]]);
            if family == 0x01 && attr_value.len() >= 8 {
                let ip = Ipv4Addr::new(attr_value[4], attr_value[5], attr_value[6], attr_value[7]);
                let addr = SocketAddr::new(IpAddr::V4(ip), port);
                info!("STUN Reflexive Public Address resolved (MAPPED): {}", addr);
                return Ok(addr);
            }
        }
    }

    bail!("No mapped address attribute found in STUN response from {}", from_addr)
}

/// Tries multiple fallback STUN servers until one succeeds
pub async fn discover_public_endpoint(socket: &UdpSocket) -> Result<SocketAddr> {
    for server in DEFAULT_STUN_SERVERS {
        match query_stun(socket, server).await {
            Ok(addr) => return Ok(addr),
            Err(e) => {
                warn!("Failed to query STUN server {}: {}", server, e);
            }
        }
    }
    bail!("All STUN servers failed to resolve public endpoint")
}

/// Send UDP hole punch probes to open firewall state table for P2P connection
pub async fn punch_hole(socket: &UdpSocket, target: SocketAddr, count: usize) -> Result<()> {
    let punch_msg = b"GSPUNCH";
    debug!("Sending {} hole-punch UDP packets to {}", count, target);
    for _ in 0..count {
        let _ = socket.send_to(punch_msg, target).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

fn generate_transaction_id() -> [u8; 12] {
    let mut id = [0u8; 12];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = now.as_nanos();
    id[0..8].copy_from_slice(&(nanos as u64).to_be_bytes());
    id[8..12].copy_from_slice(&((nanos >> 64) as u32 ^ 0xA5C3_5D2F).to_be_bytes());
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stun_packet_header_format() {
        let tx = generate_transaction_id();
        assert_eq!(tx.len(), 12);
        // Ensure non-zero
        assert!(tx.iter().any(|&b| b != 0));
    }
}
