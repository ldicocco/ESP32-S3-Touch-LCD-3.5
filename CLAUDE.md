# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Project

Firmware for the **Waveshare ESP32-S3-Touch-LCD-3.5** development board
(3.5-inch 320×480 capacitive touch display), written in **Rust `no_std`**
with the **Embassy** async runtime.

- Product page: https://www.waveshare.com/esp32-s3-touch-lcd-3.5.htm
- Wiki / docs: https://docs.waveshare.com/ESP32-S3-Touch-LCD-3.5
- Schematics & datasheets: https://docs.waveshare.com/ESP32-S3-Touch-LCD-3.5/Resources-And-Documents
- Waveshare demo code (ESP-IDF + Arduino): https://github.com/waveshareteam/ESP32-S3-Touch-LCD-3.5

Sibling project (proven template for the stack, toolchain, and many
conventions here): `~/dev_esp32/ESP32-S3-5inch-Display` (Waveshare
ESP32-S3-Touch-LCD-5B, parallel-RGB panel). The big architectural
difference: that board's panel is 16-bit parallel RGB with a PSRAM
framebuffer and hand-rolled scanout; **this board's panel is a plain SPI
ST7796** — esp-hal SPI + DMA, no scanout task, far smaller buffers, none of
the PSRAM-underrun drama.

## Conventions

- **Indentation is 2 spaces** everywhere (Rust, TOML, config). Enforce via
  `rustfmt.toml` (`tab_spaces = 2`) and `.editorconfig`.
- `no_std`, no `alloc` unless a concrete need arises (PSRAM-backed heap is
  an option via `esp-alloc`).
- Async-first: peripherals driven through Embassy tasks; avoid blocking
  waits.

## Hardware reference (ESP32-S3-Touch-LCD-3.5)

Pin assignments below are extracted from Waveshare's shipping demos
(`ESP-IDF/02_lvgl_example` and the Arduino examples in the repo above) —
authoritative in practice, since the wiki has no text pin table. Anything
not yet confirmed on real hardware is marked.

### Core

| Item | Value |
|---|---|
| SoC | ESP32-S3R8, chip-down (Xtensa LX7 dual-core @ 240 MHz) |
| Flash | W25Q128JVSIQ — 16 MB Quad-SPI NOR (external, not in-package) |
| PSRAM | 8 MB stacked (**Octal** — R8 variant; needs the octal PSRAM config in esp-hal) |
| Wireless | 2.4 GHz Wi-Fi (802.11 b/g/n), BLE 5, onboard antenna |
| Power | USB-C 5 V; **AXP2101 PMU** manages all rails, Li-ion charging, power button |
| Battery | 3.7 V single cell via MX1.25 2-pin connector; separate SH1.0 header for a rechargeable RTC backup cell |
| Buttons | PWR (AXP2101 PEKEY), BOOT (GPIO0), RESET |
| Expansion | 2.54 mm GPIO header; camera FPC (OV5640/OV2640 DVP); TF-card slot; speaker MX1.25; onboard mic |

### Display

- 3.5-inch IPS, **320×480**, capacitive touch.
- **ST7796** controller on **SPI** (demo: SPI2, 80 MHz pixel/SPI clock,
  mode 0, 8-bit commands/params).
- Demo panel config: **BGR element order + color inversion on**
  (`LCD_RGB_ELEMENT_ORDER_BGR`, `invert_color(true)`), 16 bpp RGB565.
- Full RGB565 frame = 320×480×2 = **300 KB** — a full framebuffer fits in
  PSRAM trivially, and partial (stripe) buffers fit in internal SRAM; the
  demo uses PSRAM double buffers of 1/8 frame.
- Backlight: **GPIO6**, plain PWM (demo: LEDC 5 kHz, 10-bit). High = on.
- Panel reset: **TCA9554 expander pin EXIO1** (pulse low ≥100 ms in demo).
  LCD CS and RST are not on ESP32 GPIOs (CS tied active in hardware).

| Signal | GPIO |
|---|---|
| MOSI | 1 |
| MISO | 2 (not used by panel writes) |
| SCLK | 5 |
| DC | 3 |
| CS | — (hard-wired) |
| RST | TCA9554 EXIO1 |
| Backlight | 6 (PWM) |

### Touch

- **FT6336** capacitive controller, I²C address **0x38**.
- The demo wires **no INT and no RST GPIO** (polling; reset presumably
  shared with panel reset on the expander) — verify against the schematic
  if an interrupt pin is wanted.

### Shared I²C bus (GPIO8 = SDA, GPIO7 = SCL)

Note the order — **SDA 8, SCL 7** (the 5-inch board is 8/9).

| Device | Address | Purpose |
|---|---|---|
| FT6336 | `0x38` | Touch |
| TCA9554 | `0x20` | IO expander (A2..A0 = 000) |
| AXP2101 | `0x34` | PMU |
| PCF85063 | `0x51` | RTC |
| QMI8658 | `0x6B` | 6-axis IMU (accel + gyro) |
| ES8311 | `0x18` | Audio codec |
| Camera SCCB | per sensor | OV5640/OV2640 share this bus (SIOD 8 / SIOC 7) |

### TCA9554 IO expander

Standard-protocol TCA9554 (unlike the 5-inch board's weird CH422G) — input
port / output port / polarity / config registers at `0x00`–`0x03`.

| Pin | Function |
|---|---|
| EXIO1 | LCD_RST (panel reset, demo-verified) |
| others | not exercised by the demos — check the schematic PDF before use (candidates: touch reset, camera PWDN) |

### AXP2101 PMU

The demo brings up basically every rail at boot (DC1/DC3 3.3 V, ALDO1–4
3.3 V, BLDO1 1.5 V, BLDO2 2.8 V — the B/ALDO rails feed camera + codec),
disables TS-pin measurement (no battery NTC — **charging misbehaves if TS
measure is left on**), sets charge current 200 mA, target 4.1 V, and
enables RTC button-battery charging at 3.3 V. The board boots with PMU
hardware defaults, so treat full PMU init as required only once
battery/camera/audio matter — but keep `disableTSPinMeasure` in mind as
soon as a battery is attached.

Power button behavior (demo config): 128 ms press to power on, 4 s hold to
power off. PMU IRQ pin: not wired in the demos — check schematic.

### Audio (ES8311 codec + mic + speaker)

I²S full-duplex, codec is both DAC (speaker via MX1.25 header) and ADC
(onboard analog mic):

| Signal | GPIO |
|---|---|
| MCLK | 12 |
| BCLK | 13 |
| LRCK / WS | 15 |
| SDOUT (ESP32 → codec, playback) | 16 |
| SDIN (codec → ESP32, mic) | 14 |

Demo runs 48 kHz, MCLK = 256 × Fs. No separate PA-enable GPIO in the demo
(`pa_pin = NC`).

### TF card (SDMMC, 1-bit)

| Signal | GPIO |
|---|---|
| CLK | 11 |
| CMD | 10 |
| D0 | 9 |

SDMMC peripheral in 1-bit mode (not SPI), 20 MHz default.

### Camera (DVP, optional OV5640/OV2640 on FPC)

XCLK 38, PCLK 41, VSYNC 17, HREF 18; data D2–D9 = 45, 47, 48, 46, 42, 40,
39, 21. SCCB on the shared I²C bus. PWDN/RESET not on ESP32 GPIOs
(expander candidates). Note the camera occupies strapping pins 45/46 —
absent camera use they're free.

### Other

- **USB-C**: native USB on GPIO19 (D−) / GPIO20 (D+) — USB-Serial-JTAG for
  flash + console.
- **BOOT button** on GPIO0 (usable as a user button at runtime; demo uses
  `iot_button`).
- **PCF85063 RTC** with dedicated backup-battery header (charged by the
  AXP2101 button-battery charger).
- IMU/RTC interrupt pins: not used by demos, check schematic.

## Software stack (planned — mirror the 5-inch project)

- **`esp-hal`** (1.1.x, features `esp32s3` + `unstable`, octal PSRAM
  config) — `no_std` HAL. Display over SPI + DMA.
- **`esp-rtos`** (features `embassy`, `esp32s3`) + **`embassy-executor`**
  (0.10), **`embassy-time`** — async runtime. Entry point `#[esp_rtos::main]`,
  scheduler started with `esp_rtos::start(timer, software_interrupt0)`.
- **`esp-println`** / **`esp-backtrace`** — console + panic backtraces over
  USB-Serial-JTAG.
- **`esp-bootloader-esp-idf`** — `esp_app_desc!()` in `main.rs` is required.
- **`oxivgl`** (pin exact version, features `esp32s3` + `log-04`) — safe
  bindings for **LVGL 9.5**; the 5-inch project uses `=0.6.1` /
  `oxivgl-sys =0.2.4`. oxivgl targets SPI panels natively
  (`LV_COLOR_FORMAT_RGB565_SWAPPED` default byte order — likely correct
  as-is for ST7796 over SPI, unlike the DPI panel which needed the
  override).
- Panel driver: minimal in-repo ST7796 (`src/display.rs`) — Waveshare's
  vendored init sequence, BGR + inversion, blocking SPI.
- Touch: minimal in-repo FT6336 driver (`src/ft6336.rs`), poll-only.
- Expander: minimal in-repo TCA9554 driver (`src/tca9554.rs`).
- PMU: AXP2101 — `axp2101` crates exist but are young; verify or write a
  minimal one (rails + charger config only). Not needed for USB-powered
  display work (the board boots on PMU hardware defaults).
- I²C sharing: **no `embedded-hal-bus`** — a single sensor-hub task
  (`src/sensors.rs`) owns the bus and multiplexes touch/IMU/PMU/RTC on a
  15 ms ticker; drivers are constructed transiently per poll via the
  `&mut bus` blanket `embedded_hal::i2c::I2c` impl. Revisit only if a
  device ever needs bus access from a second task.
- **`esp-radio`** (0.18, features `wifi` + `esp-alloc` + `unstable`) +
  **`embassy-net`** (0.9, DHCP/TCP/UDP) — Wi-Fi STA. Needs esp-rtos
  features `esp-alloc` + `esp-radio` (the radio blob rides on the esp-rtos
  scheduler; `esp_radio::wifi::new` must run after `esp_rtos::start`).
  The driver allocates ~46 KiB from the esp-alloc heap at init.

## Toolchain

ESP32-S3 is **Xtensa**, not RISC-V — it needs the espressif Rust toolchain
(already installed on this machine):

```sh
cargo install espup espflash
espup install            # installs the `esp` toolchain channel
# per shell: . ~/export-esp.sh (or source it from your shell profile)
```

- Build target: `xtensa-esp32s3-none-elf`, `channel = "esp"` in
  `rust-toolchain.toml`.
- Flash/monitor: `cargo run --release` with runner
  `espflash flash --monitor --chip esp32s3`; board enumerates via
  USB-Serial-JTAG on the USB-C port (`/dev/cu.usbmodem*` on macOS).
- **Every build shell needs `. ~/export-esp.sh` first** or the build fails
  to find the compiler.
- Flash mode/speed: `--flash-mode dio --flash-freq 80mhz --flash-size 16mb`
  (in the Cargo runner) **boots, verified on hardware**. The W25Q128
  nominally supports QIO — untested, try separately if flash bandwidth ever
  matters.
- Non-interactive monitoring (agents/CI): `espflash monitor
  --non-interactive --elf target/…/<binary>` — plain `cargo run` needs a
  real TTY.

### LVGL (oxivgl) build requirements

Same setup as the 5-inch project (copy its `.cargo/config.toml` `[env]`
block and `lv-conf/lv_conf.h`, then adjust resolution-dependent values):

- `DEP_LV_CONFIG_PATH` → `lv-conf/` (holds `lv_conf.h`). Changing
  `lv_conf.h` triggers a full LVGL C rebuild.
- `BINDGEN_EXTRA_CLANG_ARGS` → `--sysroot=…/xtensa-esp-elf/esp-15.2.0_20250920/…`
  so bindgen's clang finds libc headers. **Version-tied path** — bump the
  `esp-XX.Y.Z` segment after `espup update`.
- `CC_xtensa_esp32s3_none_elf = xtensa-esp32s3-elf-gcc` — without the pin,
  `cc` picks a compiler that emits big-endian Xtensa objects (link
  failure).
- Build-host prerequisites: esp-clang (auto-detected `LIBCLANG_PATH`),
  network on first build (LVGL tarball), python3 with `pypng` + `lz4`. A
  killed first build can leave a truncated LVGL tree — delete
  `target/**/oxivgl-sys-*/out/lvgl-9.5.0` to fix.

### rust-analyzer workaround (esp toolchain)

rust-analyzer passes `--lockfile-path` to `cargo metadata`; the Espressif
cargo fork rejects the flag, so RA silently indexes no dependencies. Fix in
place on this machine: `~/.rustup/toolchains/esp/bin/cargo` is a shell shim
(real binary at `cargo-bin` next to it) converting the flag to the
`CARGO_LOCKFILE_PATH` env var. Shim source lives in the 5-inch repo at
`tools/ra-bin/cargo`.

**`espup update` overwrites the toolchain and removes the shim** — if RA
regresses after an update, reinstall:

```sh
mv ~/.rustup/toolchains/esp/bin/cargo ~/.rustup/toolchains/esp/bin/cargo-bin
cp ~/dev_esp32/ESP32-S3-5inch-Display/tools/ra-bin/cargo ~/.rustup/toolchains/esp/bin/cargo
chmod +x ~/.rustup/toolchains/esp/bin/cargo
```

Also mirror the 5-inch repo's `rust-analyzer.toml` + `.vscode/settings.json`
(`rust-analyzer.cargo.allTargets` / `check.allTargets` = false): without
them the editor runs `cargo check --all-targets`, which can't build the
test harness for this target and shows a bogus "can't find crate for
`test`" squiggle.

## Repository state

Cargo project scaffolded with `esp-generate 1.3.0` (options: `unstable-hal`,
`embassy`, `log`, `esp-backtrace`), conventions files mirrored from the
5-inch repo. **Hardware-verified (2026-07-23):** the board boots with
`--flash-mode dio --flash-freq 80mhz`, and the `i2c-scan` diagnostic found
all six expected devices at the predicted addresses (0x18, 0x20, 0x34,
0x38, 0x51, 0x6B) on SDA=GPIO8 / SCL=GPIO7 — the pin/address tables above
are confirmed, not just demo-derived.

Current firmware (runs on the board, all hardware-verified 2026-07-23):
**four-tab LVGL 9.5 demo UI via oxivgl 0.6.1** — `Home` (RTC clock/date,
Wi-Fi card, AXP2101 battery gauge ring, uptime), `IMU` (live 3-axis accel
chart 6 s @ 10 Hz, numeric readouts, bubble level, gyro line), `Play`
(tap-counter button, slider, switch+LED, roller, touch coords), `System`
(backlight brightness slider — actually dims the panel via LEDC —, 1 Hz
stats table: IP/heaps/uptime), bottom tab bar, FPS/CPU overlay — plus
**Wi-Fi STA** (scan, WPA2 connect, DHCP; credentials via
`WIFI_SSID=x WIFI_PASSWORD=y cargo run --release`) and a 1 Hz debug-level
heartbeat. LVGL pool after create: ~20 KiB used / 25 KiB free (of 48 KiB).

- `src/bin/main.rs` — entry point (`#[esp_rtos::main]`); heap = 73 744 B
  reclaimed dram2 + 64 KiB .bss (Wi-Fi is the big customer), TCA9554 panel
  reset → bus release, **LEDC backlight** (LowSpeed timer0, 5 kHz,
  10-bit, channel0 on GPIO6; `Ledc` + timer in `StaticCell`s so the
  channel is `'static`), ST7796 init, interrupt executor (flush), sensor
  hub task, `wifi::start`, then `oxivgl::view::run_app` (never returns).
- `src/bin/i2c-scan.rs` — diagnostic: I²C bus scan with expected-device
  check + backlight (GPIO6) blink
  (`cargo run --release --bin i2c-scan`; verified on the board).
- `src/bin/lcd-test.rs` — diagnostic: TCA9554 reset pulse → ST7796 init →
  eight color bars (`cargo run --release --bin lcd-test`; runs error-free
  on the board).
- `src/bin/touch-test.rs` — diagnostic: FT6336 info + 20 ms coordinate
  poll to serial (`cargo run --release --bin touch-test`; verified on the
  board — chip id 0x64, fw 0x10, vendor 0x11, live coordinates confirmed).
- `src/bin/sensor-test.rs` — diagnostic: QMI8658 + AXP2101 + PCF85063
  bring-up and 1 s readings to serial; sets the RTC once at boot when
  built with `RTC_SET="YYYY-MM-DD HH:MM:SS"` (verified on the board:
  whoami 0x05 rev 0x7c, gravity ≈ −950 mg on Z lying flat, VBUS ≈ 5.19 V,
  battery-absent flags, RTC ticking; status1=0x20/status2=0x15 on USB
  power confirm the STATUS bit layout).
- `src/lib.rs` — `#![no_std]` lib: `axp2101`, `display`, `ft6336`,
  `pcf85063`, `qmi8658`, `sensors`, `tca9554`, `touch`, `ui`, `wifi`.
- `src/display.rs` — ST7796 blocking-SPI driver: Waveshare's vendored init
  sequence (from their `esp_lcd_st7796` component — panel-specific gamma /
  power tables, NOT Espressif's upstream defaults), MADCTL = MX|BGR
  (`0x48`), COLMOD 16 bpp, inversion on, `set_window` + `push_pixels`
  (RGB565 big-endian on the wire). Plus the oxivgl flush endpoint:
  `DisplayOutput for St7796` (stripe → set_window → SPI burst) and
  `flush_task`.
- `src/ft6336.rs` — minimal FT6336 touch driver, poll-only (no INT/RST
  GPIOs on this board): count reg 0x02, 6-byte point records from 0x03,
  up to 2 points, raw panel coordinates.
- `src/tca9554.rs` — minimal TCA9554 driver (shadowed OUTPUT/CONFIG regs,
  generic over `embedded_hal::i2c::I2c`).
- `src/qmi8658.rs` — minimal QMI8658 IMU driver: whoami (0x05), reset
  (0x60=0xB0, needs ~15 ms — `reset()`/`configure()` split so the caller
  waits), CTRL1 auto-increment, ±4 g / ±512 dps @ 125 Hz, 12-byte LE burst
  from AX_L 0x35; `accel_mg`/`gyro_mdps` converters. Register values
  hardware-verified.
- `src/axp2101.rs` — minimal AXP2101 PMU driver, read-mostly: `init()`
  writes only reg 0x30=0x0D (VBAT/VBUS/VSYS ADC on, **TS measure off** —
  the battery-charging gotcha is retired up front); status bits
  (STATUS1 bit5 VBUS-good / bit3 battery-present, STATUS2 bits6:5
  direction / bits2:0 charge state), 14-bit 1 mV/LSB VBAT/VBUS reads,
  fuel-gauge percent 0xA4 (garbage without battery — gate on the
  battery-present bit). Rails untouched (hardware defaults).
- `src/pcf85063.rs` — minimal PCF85063 RTC driver: 7-byte BCD burst at
  0x04, seconds bit7 = oscillator-stop → `read() -> (DateTime, valid)`;
  `set()` clears OS.
- `src/touch.rs` — shared touch state only (`TOUCH_STATE` for the LVGL
  pointer indev, packed `TOUCH_XY` + helpers); polling lives in the hub.
- `src/sensors.rs` — **the sensor hub**: one task owns the I²C bus and
  multiplexes on a 15 ms ticker — touch every tick, IMU every 7th
  (~10 Hz), PMU/RTC staggered at ~1 s (ticks %67==0 / ==33). Publishes
  atomics for the UI (`IMU_SEQ` sample counter + per-axis mg/mdps,
  `BAT_MV`/`BAT_PERCENT` (0xFF = no battery)/`PMU_FLAGS`, packed
  `RTC_HMS`/`RTC_DATE`); consumes `BACKLIGHT_PCT` (applies LEDC duty,
  clamped ≥5 %) and `RTC_SET` (time-of-day command, `swap(0)` handshake —
  the future NTP hook). Drivers are constructed transiently per poll via
  the `&mut bus` blanket I2c impl; a device that NACKs init or fails 5
  consecutive polls is marked absent (never panics).
- `src/ui/` — the four-tab UI: `mod.rs` (`DemoView`, shared `Theme` of
  Rc-backed `Style`s — oxivgl deprecates per-object inline style setters —
  bottom Tabview, one 1 Hz `oxivgl::timer::Timer` fanned out as a bool),
  `home.rs`, `imu.rs`, `play.rs`, `system.rs`. Each tab: `create(pane,
  theme)` + `update(active, ...)` diffing atomics against `last_*` caches;
  chart is fed even when hidden (continuous history), label/bubble churn
  only when visible. The backlight slider writes `BACKLIGHT_PCT`.
- `src/wifi.rs` — Wi-Fi STA: `wifi::start` builds the esp-radio
  controller + embassy-net stack (DHCP); with compile-time credentials it
  spawns connect/net/ip tasks (state + IPv4 published via atomics for the
  UI), without them it does a one-shot AP scan (serial) and parks. Open
  networks unsupported (assumes WPA2-personal). Passwords shaped exactly
  like dash-separated groups of four (`XXXX-XXXX-…`, router-label style)
  get their dashes stripped automatically (logged) — a genuine password
  of that shape would be mangled by this.
- `lv-conf/lv_conf.h` — LVGL v9.5 config (copied from the 5-inch repo;
  **48 KiB `LV_MEM_SIZE`** — bumped for the 4-tab UI —, Montserrat 8–48,
  perf monitor on). Changing it triggers a full LVGL C rebuild.
- `build.rs` — generated; `linkall.x` linker script + friendly
  linker-error hints. Don't modify casually.

Bring-up: scaffold, heartbeat, I²C scan + backlight, color bars, touch,
LVGL, Wi-Fi, **QMI8658 IMU, AXP2101 PMU (ADC/status subset), PCF85063
RTC, LEDC backlight PWM** — all hardware-verified 2026-07-23. Remaining:
SD, audio, camera, full PMU battery/rail config — as needed. **Visual
checks pending:** color order (lcd-test bars: if red/blue swapped, flip
the BGR bit in `MADCTL_PORTRAIT`), touch↔display alignment (tap the demo
button; if x feels mirrored, mirror x in the hub's touch poll), and the
bubble-level x/y signs (tilt the board; flip signs in `ui/imu.rs` if it
moves the wrong way).

Display / LVGL architecture notes (much simpler than the 5-inch board —
no PSRAM framebuffer, no scanout, no bounce buffers):

- **Flush path:** LVGL renders 40-line stripes into two static internal-RAM
  draw buffers (`LvglBuffers<LVGL_BUF_BYTES>`, 2 × 25 KiB .bss);
  `flush_task` (interrupt executor, `software_interrupt1`,
  `Priority::min()`) drains oxivgl's flush channel and pushes each stripe
  over blocking SPI (~2.6 ms per stripe at 80 MHz). oxivgl's flush
  wait-callback spins inside the LVGL task until the flush task acks, so
  the two must never share an executor. Upgrade path if flush time ever
  dominates: DMA + async SPI writes.
- **Byte order:** oxivgl registers the display as
  `LV_COLOR_FORMAT_RGB565_SWAPPED` — which IS the ST7796's SPI wire order
  (big-endian byte pairs), so stripes are written verbatim and no
  color-format override is needed (the 5-inch DPI panel needs the
  opposite).
- **Memory:** global allocator = 73 744 B internal-RAM heap (reclaimed
  dram2) + 64 KiB .bss; LVGL widget memory from its 48 KiB static pool
  (4-tab UI uses ~20 KiB after create). PSRAM is entirely unused so far —
  not even mapped.
- **I²C ownership:** TCA9554 does the panel-reset pulse, then `release()`s
  the bus to the sensor hub task (exclusive owner; touch 15 ms, IMU
  ~105 ms, PMU/RTC ~1 s staggered — worst-case bus traffic <1.5 ms/tick
  at 400 kHz, so touch latency is unaffected).

Keep standalone diagnostic binaries in `src/bin/` (`i2c-scan`, `lcd-test`,
`touch-test`, `sensor-test`) — they pay for themselves the first time the
panel is black.

API gotcha (embassy-executor 0.10): a `#[embassy_executor::task]` fn
returns `Result<SpawnToken, SpawnError>` and `Spawner::spawn(token)`
returns `()` — i.e. `spawner.spawn(my_task().expect("pool exhausted"))`.
