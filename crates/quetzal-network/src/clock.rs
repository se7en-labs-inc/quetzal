//! SNTP offset measurement. The server's originate timestamp must match our
//! request; malformed and unsynchronised replies never produce an offset.
use crate::NetworkError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

fn now() -> Result<u64, NetworkError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| NetworkError::Protocol(e.to_string()))?;
    let seconds = duration.as_secs().wrapping_add(2208988800) & 0xffff_ffff;
    Ok((seconds << 32) | ((u64::from(duration.subsec_nanos()) << 32) / 1_000_000_000))
}

async fn peer(host: &str) -> Result<i64, NetworkError> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect((host, 123)).await?;
    let mut request = [0; 48];
    request[0] = 0x23; // NTP v4 client
    let sent = now()?;
    request[40..].copy_from_slice(&sent.to_be_bytes());
    socket.send(&request).await?;
    let mut reply = [0; 512];
    let length = socket.recv(&mut reply).await?;
    let received = now()?;
    if length < 48
        || reply[0] & 7 != 4
        || reply[0] >> 6 == 3
        || !(1..=15).contains(&reply[1])
        || reply[24..32] != request[40..48]
    {
        return Err(NetworkError::Protocol("invalid NTP response".into()));
    }
    let server_received = u64::from_be_bytes(
        reply[32..40]
            .try_into()
            .map_err(|_| NetworkError::Protocol("invalid NTP timestamp".into()))?,
    );
    let server_sent = u64::from_be_bytes(
        reply[40..48]
            .try_into()
            .map_err(|_| NetworkError::Protocol("invalid NTP timestamp".into()))?,
    );
    let offset = ((i128::from(server_received) - i128::from(sent))
        + (i128::from(server_sent) - i128::from(received)))
        * 1_000_000
        / (2i128 << 32);
    i64::try_from(offset).map_err(|e| NetworkError::Protocol(e.to_string()))
}

pub async fn measure() -> Result<i64, NetworkError> {
    for host in ["time.cloudflare.com", "time.google.com", "pool.ntp.org"] {
        if let Ok(Ok(offset)) = tokio::time::timeout(Duration::from_millis(700), peer(host)).await {
            return Ok(offset);
        }
    }
    Err(NetworkError::Protocol("NTP measurement unavailable".into()))
}
