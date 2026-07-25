//! SNTP client (RFC 4330, client mode): once the network is up it asks an
//! NTP pool server for UTC, applies the compile-time timezone offset, and
//! hands the timestamp to the sensor hub ([`RTC_SET_EPOCH`]) to write into
//! the PCF85063 RTC. Re-syncs every 24 h (the PCF85063 drifts tens of
//! ppm). Only spawned when Wi-Fi credentials are configured.
//!
//! The RTC stores *local* time: set `TZ_OFFSET_MINUTES` at build time
//! (e.g. 120 for CEST, 60 for CET; default 0 = UTC). The offset is fixed —
//! no DST logic; rebuild when the clocks change.

use core::sync::atomic::Ordering;

use embassy_net::dns::DnsQueryType;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{IpEndpoint, Stack};
use embassy_time::{Duration, Timer, with_timeout};

use crate::sensors::RTC_SET_EPOCH;

const NTP_SERVER: &str = "pool.ntp.org";
const NTP_PORT: u16 = 123;
/// Local UDP port for the client socket (smoltcp can't bind port 0).
const LOCAL_PORT: u16 = 50123;
/// Seconds from the NTP era (1900-01-01) to the RTC epoch (2000-01-01).
const NTP_TO_Y2K: u32 = 3_155_673_600;

const TZ_OFFSET_MINUTES: Option<&str> = option_env!("TZ_OFFSET_MINUTES");

const RESYNC: Duration = Duration::from_secs(24 * 3600);
const RETRY: Duration = Duration::from_secs(60);
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(15);

/// Waits for the network, then keeps the RTC synced; see module docs.
#[embassy_executor::task]
pub async fn sntp_task(stack: Stack<'static>) {
  let tz_minutes: i32 = match TZ_OFFSET_MINUTES.map(str::parse) {
    None => 0,
    Some(Ok(v)) => v,
    Some(Err(_)) => {
      log::warn!(
        "TZ_OFFSET_MINUTES {:?} unparseable; using UTC",
        TZ_OFFSET_MINUTES.unwrap_or_default()
      );
      0
    }
  };
  loop {
    stack.wait_config_up().await;
    let interval = match with_timeout(ATTEMPT_TIMEOUT, sync_once(stack, tz_minutes)).await {
      Ok(Ok(())) => RESYNC,
      Ok(Err(e)) => {
        log::warn!("SNTP sync failed: {e}");
        RETRY
      }
      Err(_) => {
        log::warn!("SNTP sync timed out");
        RETRY
      }
    };
    Timer::after(interval).await;
  }
}

/// One DNS lookup + one SNTP round trip; publishes to the hub on success.
async fn sync_once(stack: Stack<'static>, tz_minutes: i32) -> Result<(), &'static str> {
  let addrs = stack
    .dns_query(NTP_SERVER, DnsQueryType::A)
    .await
    .map_err(|_| "DNS query failed")?;
  let addr = *addrs.first().ok_or("DNS returned no addresses")?;

  let mut rx_meta = [PacketMetadata::EMPTY; 2];
  let mut rx_buf = [0u8; 128];
  let mut tx_meta = [PacketMetadata::EMPTY; 2];
  let mut tx_buf = [0u8; 128];
  let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
  socket.bind(LOCAL_PORT).map_err(|_| "UDP bind failed")?;

  // 48-byte client request: LI 0, version 4, mode 3; everything else zero.
  let mut pkt = [0u8; 48];
  pkt[0] = 0x23;
  socket
    .send_to(&pkt, IpEndpoint::new(addr, NTP_PORT))
    .await
    .map_err(|_| "UDP send failed")?;

  let mut resp = [0u8; 48];
  let (n, meta) = socket
    .recv_from(&mut resp)
    .await
    .map_err(|_| "UDP recv failed")?;
  if meta.endpoint.addr != addr {
    return Err("reply from unexpected host");
  }
  if n < 44 {
    return Err("short NTP response");
  }
  if resp[0] & 0x07 != 4 {
    return Err("not a server reply");
  }
  if resp[1] == 0 {
    return Err("NTP kiss-of-death (stratum 0)");
  }

  // Server transmit timestamp: seconds since 1900, big-endian, at offset 40.
  let ntp_secs = u32::from_be_bytes([resp[40], resp[41], resp[42], resp[43]]);
  let utc = ntp_secs
    .checked_sub(NTP_TO_Y2K)
    .ok_or("NTP time before 2000")?;
  let local = (i64::from(utc) + i64::from(tz_minutes) * 60).clamp(1, i64::from(u32::MAX)) as u32;
  RTC_SET_EPOCH.store(local, Ordering::Relaxed);
  log::info!("SNTP: time received from {addr} (tz offset {tz_minutes} min), RTC update queued");
  Ok(())
}
