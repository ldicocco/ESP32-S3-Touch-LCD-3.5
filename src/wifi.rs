//! Wi-Fi station bring-up (esp-radio + embassy-net DHCP), with connection
//! state published for the UI.
//!
//! Credentials are compile-time env vars — build with
//! `WIFI_SSID=MyAP WIFI_PASSWORD=secret cargo run --release` (or set them
//! in `.cargo/config.toml` `[env]`). Without credentials the radio does a
//! one-shot AP scan (logged to serial as a hardware check) and stays idle.

use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::wifi::scan::ScanConfig;
use esp_radio::wifi::sta::StationConfig;
use esp_radio::wifi::{Config, ControllerConfig, Interface, WifiController};
use static_cell::StaticCell;

pub const SSID: Option<&str> = option_env!("WIFI_SSID");
const PASSWORD: Option<&str> = option_env!("WIFI_PASSWORD");

pub const STATE_UNCONFIGURED: u8 = 0;
pub const STATE_CONNECTING: u8 = 1;
pub const STATE_CONNECTED: u8 = 2;

/// Connection state (STATE_*), for the UI.
pub static WIFI_STATE: AtomicU8 = AtomicU8::new(STATE_UNCONFIGURED);
/// IPv4 address as big-endian u32 (0 = none), for the UI.
pub static WIFI_IP: AtomicU32 = AtomicU32::new(0);

/// Sockets: DHCP + DNS (both internal to the stack) + the SNTP UDP socket,
/// plus one spare.
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();

/// Initializes the radio and network stack and spawns the Wi-Fi tasks.
/// Call after `esp_rtos::start` and after the heap exists (the radio
/// allocates from `esp-alloc`).
/// WPA keys are commonly quoted as dash-separated groups of four
/// characters (`XXXX-XXXX-…`, FRITZ!Box style) while the actual PSK has no
/// dashes. If — and only if — the whole password matches that shape, the
/// dashes are stripped (logged). A genuine password that happens to have
/// this exact shape would be mangled; the log line is the tell.
fn effective_password() -> alloc::string::String {
  let pw = PASSWORD.unwrap_or("");
  let grouped = pw.contains('-') && pw.split('-').all(|g| g.len() == 4);
  if grouped {
    log::info!("Wi-Fi password looks like dash-separated groups of four; stripping dashes");
    pw.chars().filter(|c| *c != '-').collect()
  } else {
    pw.into()
  }
}

pub fn start(spawner: &Spawner, wifi: WIFI<'static>) {
  let password = effective_password();
  if let Some(ssid) = SSID {
    // Password length only — enough to catch an empty/mangled env var
    // (the classic cause of FourWayHandshakeTimeout) without leaking it.
    log::info!(
      "Wi-Fi credentials: ssid {:?} ({} chars), password {} chars",
      ssid,
      ssid.len(),
      password.len()
    );
  }
  let station_config = Config::Station(
    StationConfig::default()
      .with_ssid(SSID.unwrap_or("unconfigured"))
      .with_password(password),
  );
  let (controller, interfaces) = esp_radio::wifi::new(
    wifi,
    ControllerConfig::default().with_initial_config(station_config),
  )
  .expect("Wi-Fi init failed");

  let rng = Rng::new();
  let seed = ((rng.random() as u64) << 32) | rng.random() as u64;
  let (stack, runner) = embassy_net::new(
    interfaces.station,
    embassy_net::Config::dhcpv4(Default::default()),
    STACK_RESOURCES.init(StackResources::new()),
    seed,
  );
  spawner.spawn(net_task(runner).expect("net task pool exhausted"));

  if SSID.is_some() {
    WIFI_STATE.store(STATE_CONNECTING, Ordering::Relaxed);
    spawner.spawn(connection_task(controller).expect("connection task pool exhausted"));
    spawner.spawn(ip_task(stack).expect("ip task pool exhausted"));
    // With a network there's a time source: keep the RTC synced via SNTP.
    spawner.spawn(crate::sntp::sntp_task(stack).expect("sntp task pool exhausted"));
  } else {
    log::info!("Wi-Fi unconfigured (set WIFI_SSID/WIFI_PASSWORD at build time); scanning only");
    spawner.spawn(scan_task(controller).expect("scan task pool exhausted"));
  }
}

/// Keeps the station associated, retrying on failure/disconnect.
#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>) {
  let ssid = SSID.unwrap_or_default();
  loop {
    log::info!("Wi-Fi connecting to {ssid:?}...");
    WIFI_STATE.store(STATE_CONNECTING, Ordering::Relaxed);
    match controller.connect_async().await {
      Ok(info) => {
        log::info!("Wi-Fi connected: {info:?}");
        WIFI_STATE.store(STATE_CONNECTED, Ordering::Relaxed);
        let info = controller.wait_for_disconnect_async().await.ok();
        log::warn!("Wi-Fi disconnected: {info:?}");
        WIFI_STATE.store(STATE_CONNECTING, Ordering::Relaxed);
      }
      Err(e) => log::warn!("Wi-Fi connect failed: {e:?}"),
    }
    Timer::after(Duration::from_secs(5)).await;
  }
}

/// One-shot AP scan for the unconfigured case; parks afterwards, keeping
/// the controller (and thus the radio) alive.
#[embassy_executor::task]
async fn scan_task(mut controller: WifiController<'static>) {
  match controller
    .scan_async(&ScanConfig::default().with_max(10))
    .await
  {
    Ok(aps) => {
      log::info!("Wi-Fi scan: {} access points", aps.len());
      for ap in &aps {
        log::info!(
          "  {:?} ch {} rssi {} {:?}",
          ap.ssid,
          ap.channel,
          ap.signal_strength,
          ap.auth_method
        );
      }
    }
    Err(e) => log::warn!("Wi-Fi scan failed: {e:?}"),
  }
  loop {
    Timer::after(Duration::from_secs(3600)).await;
  }
}

/// Tracks DHCP state, publishing the IPv4 address for the UI.
#[embassy_executor::task]
async fn ip_task(stack: Stack<'static>) {
  loop {
    stack.wait_config_up().await;
    if let Some(config) = stack.config_v4() {
      let octets = config.address.address().octets();
      WIFI_IP.store(u32::from_be_bytes(octets), Ordering::Relaxed);
      log::info!("Wi-Fi got IP: {}", config.address);
    }
    stack.wait_config_down().await;
    WIFI_IP.store(0, Ordering::Relaxed);
    log::warn!("Wi-Fi lost IP config");
  }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
  runner.run().await
}
