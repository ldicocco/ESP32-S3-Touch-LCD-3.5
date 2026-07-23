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
- Panel driver: `mipidsi` crate supports ST7796 and is the obvious
  candidate; otherwise a minimal in-repo init-sequence + DMA blit is ~100
  lines. Match the demo's BGR + inversion settings.
- Touch: FT6336 is register-compatible with FT6x06/FT6236 — candidate
  crates `ft6x06`/`ft6x36`, or a minimal in-repo driver (the GT911 driver
  in the 5-inch repo is a good size template).
- Expander: TCA9554 is PCA9554-compatible → the `port-expander` crate
  (PCA9554 support) or a trivial in-repo driver.
- PMU: AXP2101 — `axp2101` crates exist but are young; verify or write a
  minimal one (rails + charger config only).
- **`embedded-hal-bus`** — the I²C bus is shared by six devices; an async
  or `CriticalSectionDevice` sharing strategy is required from day one.

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

- `src/bin/main.rs` — entry point (`#[esp_rtos::main]`); currently a 1 Hz
  heartbeat over USB-Serial-JTAG (verified on the board).
- `src/bin/i2c-scan.rs` — diagnostic: I²C bus scan with expected-device
  check + backlight (GPIO6) blink
  (`cargo run --release --bin i2c-scan`; verified on the board).
- `src/bin/lcd-test.rs` — diagnostic: TCA9554 reset pulse → ST7796 init →
  eight color bars (`cargo run --release --bin lcd-test`; runs error-free
  on the board).
- `src/lib.rs` — `#![no_std]` lib: `display`, `tca9554` modules.
- `src/display.rs` — ST7796 blocking-SPI driver: Waveshare's vendored init
  sequence (from their `esp_lcd_st7796` component — panel-specific gamma /
  power tables, NOT Espressif's upstream defaults), MADCTL = MX|BGR
  (`0x48`), COLMOD 16 bpp, inversion on, `set_window` + `push_pixels`
  (RGB565 big-endian on the wire).
- `src/tca9554.rs` — minimal TCA9554 driver (shadowed OUTPUT/CONFIG regs,
  generic over `embedded_hal::i2c::I2c`).
- `build.rs` — generated; `linkall.x` linker script + friendly
  linker-error hints. Don't modify casually.

Bring-up plan, remaining steps (mirroring the 5-inch project's proven
order):

1. ~~Scaffold~~ ✓  2. ~~Heartbeat~~ ✓  3. ~~I²C scan + backlight~~ ✓
4. ~~Panel color bars~~ ✓ (runs error-free; **visual check of color order
   and orientation still pending** — if red/blue are swapped, flip the BGR
   bit in `MADCTL_PORTRAIT`)
5. FT6336 touch poll → serial coordinates.
6. LVGL via oxivgl: SPI flush callback, stripe draw buffers in internal
   SRAM (also add the oxivgl `[env]` vars to `.cargo/config.toml` and
   `lv-conf/` at this point — deliberately not present yet, and
   `build-std` is still `["core"]` without `alloc`).
7. Then the rest as needed: AXP2101, RTC, IMU, SD, audio, Wi-Fi.

Keep standalone diagnostic binaries in `src/bin/` (the 5-inch repo's
`backlight-test` / `scanout-test` pattern) — they pay for themselves the
first time the panel is black.

API gotcha (embassy-executor 0.10): a `#[embassy_executor::task]` fn
returns `Result<SpawnToken, SpawnError>` and `Spawner::spawn(token)`
returns `()` — i.e. `spawner.spawn(my_task().expect("pool exhausted"))`.
