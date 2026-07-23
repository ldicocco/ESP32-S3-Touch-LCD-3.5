# ESP32-S3 Touch LCD 3.5

Rust `no_std` + [Embassy](https://embassy.dev/) firmware for the
[Waveshare ESP32-S3-Touch-LCD-3.5](https://www.waveshare.com/esp32-s3-touch-lcd-3.5.htm)
— an ESP32-S3 development board with a 3.5-inch 320×480 IPS capacitive
touch display.

## Hardware

| | |
|---|---|
| SoC | ESP32-S3R8 (dual-core Xtensa LX7 @ 240 MHz) |
| Memory | 16 MB flash (W25Q128), 8 MB octal PSRAM |
| Display | 3.5" IPS, 320×480, ST7796 over SPI, PWM backlight |
| Touch | FT6336 capacitive, I²C |
| Onboard | AXP2101 PMU (Li-ion charging, power button), PCF85063 RTC with backup-battery header, QMI8658 6-axis IMU, ES8311 audio codec + mic + speaker header, TCA9554 IO expander, TF-card slot (SDMMC), DVP camera connector (OV5640/OV2640), 2.54 mm GPIO header |
| Power | USB-C 5 V or 3.7 V Li-ion battery (MX1.25) |
| Wireless | 2.4 GHz Wi-Fi, BLE 5, onboard antenna |

Full pinout and board details: see [CLAUDE.md](CLAUDE.md) and the
[Waveshare wiki](https://docs.waveshare.com/ESP32-S3-Touch-LCD-3.5).

## Software stack

- [`esp-hal`](https://github.com/esp-rs/esp-hal) — bare-metal `no_std` HAL for the ESP32-S3
- [`esp-rtos`](https://github.com/esp-rs/esp-hal) + `embassy-executor` / `embassy-time` — async runtime
- [`oxivgl`](https://crates.io/crates/oxivgl) — safe Rust bindings for **LVGL 9.5**
- [`esp-println`](https://github.com/esp-rs/esp-println) / [`esp-backtrace`](https://github.com/esp-rs/esp-backtrace) — serial console and panic backtraces over USB-Serial-JTAG

## Status

Working on the board: an **LVGL 9.5 demo UI** (via oxivgl) on the 320×480
panel — title, tap-counter button, slider, live touch-coordinate label,
and LVGL's FPS/CPU overlay — driven by the FT6336 capacitive touch
controller. `cargo run --release` builds and flashes it.

Standalone diagnostics, useful when bring-up breaks:

- `cargo run --release --bin i2c-scan` — I²C bus scan (expects all six
  onboard devices) + backlight blink
- `cargo run --release --bin lcd-test` — ST7796 init + color bars, no LVGL
- `cargo run --release --bin touch-test` — FT6336 coordinates on serial

Not yet touched: AXP2101 PMU, RTC, IMU, SD card, audio, camera, Wi-Fi. See the bring-up plan in
[CLAUDE.md](CLAUDE.md). The sibling project
[ESP32-S3-5inch-Display](../ESP32-S3-5inch-Display) (same stack, parallel
RGB panel) serves as the reference implementation.

## Getting started

The ESP32-S3 uses the Xtensa architecture, which requires the Espressif
Rust toolchain:

```sh
cargo install espup espflash
espup install
. ~/export-esp.sh
```

Build, flash, and monitor:

```sh
cargo run --release
```

(the Cargo runner is `espflash flash --monitor`). The board flashes over
its USB-C port via the built-in USB-Serial-JTAG. If it doesn't enter the
bootloader automatically, hold **BOOT** while powering on (or while
pressing **RESET**).

## Conventions

- 2-space indentation everywhere (`rustfmt.toml`, `.editorconfig`)
- `no_std`, async-first via Embassy

## License

See [LICENSE](LICENSE).
