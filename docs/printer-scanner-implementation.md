# Printer + scanner on the ESP32-S3-Touch-LCD-3.5 — implementation plan

This document assumes the board decision has been taken: the project moves
to (or stays on) the **Waveshare ESP32-S3-Touch-LCD-3.5** — this
repository's board. It describes, at the physical and the firmware level,
how the two POS peripherals are attached and driven:

- **58 mm thermal printer**, EM5820-class **TTL** module, ESC/POS command
  set.
- **NETUM NT-EM61** fixed-mount 1D/2D scanner, **TTL interface variant**
  (not RS485 — that was the 5B's constraint, not this board's).

For the rationale and the board comparison see the design discussion in
the sibling repo:
[printer-scanner-integration.md](../../ESP32-S3-5inch-Display/docs/printer-scanner-integration.md)
(§6 analyses this board, §7 re-ranks the candidates). The 5B counterpart
of this file is
[printer-scanner-implementation.md](../../ESP32-S3-5inch-Display/docs/printer-scanner-implementation.md);
sections that carry over unchanged are referenced rather than repeated,
and the deltas are the point of this document.

The design discussion left two **load-bearing open items** for this board
(§6 "Open items on the 3.5"): whether GPIO43/44 reach the header, and
whether a CH343P USB-UART bridge sits on them. **Both are now resolved
from the board schematic** (fetched from Waveshare 2026-07-25, §2) — in
this board's favour. Facts below are marked **hardware-verified** (from
[../CLAUDE.md](../CLAUDE.md), confirmed on the bench 2026-07-23),
**schematic-verified** (read off the schematic PDF, not yet beeped out on
real hardware), or **(verify)** (needs the actual peripheral module on
the bench). Open verifications are collected in §11.

This doc targets the exact board this repo runs on (ESP32-S3R8, 16 MB
flash, ST7796 SPI panel). Other SKUs in Waveshare's 3.5" family are not
considered.

---

## 1. System overview

```
                     12 V / 3 A PSU
                     │
        ┌────────────┼──────────────────┐
        │            │                  │
        ▼            ▼                  ▼
   buck 12→5 V   buck 12→8 V, ≥3 A   NT-EM61 VCC (5 V leg, from the 5 V buck)
        │            │
        ▼            ▼
   header VBUS   EM5820 VH (≈8 V, 2 A peak)
   (pin 2 → AXP2101)

ESP32-S3 UART1 ── header J8, odd row pins 3/5/7/9 ──── printer (TTL)
   GPIO21 TX  ─────────────────► printer RX      9600 8N1
   GPIO38 RX  ◄───────────────── printer TX      (DLE EOT status)
   GPIO39 CTS ◄───────────────── printer BUSY    (hardware flow control)
   GND (pin 3) ──────────────────  printer GND    (signal reference)

ESP32-S3 UART0 ── header J8 pins 25/27/29 ──────────── scanner (TTL)
   GPIO43 TX  ─────────────────► scanner RX      (trigger/beep cmds;
   GPIO44 RX  ◄───────────────── scanner TX       boot-ROM noise lands
   GND (pin 29) ─────────────────  scanner GND     here — tolerated, §5.4)
```

Data flow, firmware side:

```
scan:   UART0 RX ─► scanner task (line assembly) ─► Channel ─► UI card
print:  UI/business code ─► escpos builder ─► spool ring ─► printer task
          ─► UART1 TX FIFO (CTS-throttled in hardware) ─► printer
status: printer task (~1 Hz) DLE EOT poll ─► PRINTER_FLAGS atomic ─► UI
```

The display/LVGL/Wi-Fi stack is untouched. Both peripherals ride on
**native UART controllers** driven by two new Embassy tasks on the thread
executor. Compared with the 5B plan, an entire layer vanishes:

| 5B plan | here |
|---|---|
| SC16IS752 I²C↔UART bridge (its §5, a whole chapter) | — no bridge, no crystal to read, no register banking |
| Printer serviced from the sensor-hub I²C tick, 64 B / 15 ms | printer task owns UART1; **the hub and the I²C bus are untouched** |
| RS485 scanner variant + SP3485 + termination | plain TTL, three wires |
| Throughput ceiling ≈4.3 KB/s (I²C leg) | ≥11.5 KB/s at 115200, native |

What appears in exchange: **level-shifting diligence** (the SC16IS752 had
5 V tolerant inputs; the ESP32-S3 does **not** — §5.2) and a **DIY power
tree** (no 7–36 V VIN terminal on this board — but the header exposes
VBUS, which solves it cleanly, §4).

---

## 2. What the schematic settles

Source: `ESP32-S3-Touch-LCD-3.5-Schematic.pdf` from Waveshare's
[resources page](https://docs.waveshare.com/ESP32-S3-Touch-LCD-3.5/Resources-And-Documents)
(rev on file 2026-07-25). Everything in this section is
**schematic-verified**; a 5-minute continuity pass at bring-up (§9 step 0)
upgrades it to hardware-verified.

### 2.1 The 2×16 header (J8) pinout — previously undocumented

Waveshare's wiki says only "brings out available IO function pins". The
schematic gives the full map. Odd pins are one row, even pins the other:

| Pin | Net | | Pin | Net |
|---|---|---|---|---|
| 1 | **VBAT** (battery net, J7/AXP2101 BAT) | | 2 | **VBUS** (USB-C 5 V net → AXP2101 VBUS) |
| 3 | GND | | 4 | GND |
| 5 | **IO21** (CAM_D7) | | 6 | USB_N (GPIO19) |
| 7 | **IO38** (CAM_XCLK) | | 8 | USB_P (GPIO20) |
| 9 | **IO39** (CAM_D6) | | 10 | IO11 (SD_SCLK) |
| 11 | IO40 (CAM_D5) | | 12 | IO10 (SD_MOSI) |
| 13 | IO41 (CAM_PCLK) | | 14 | IO9 (SD_MISO) |
| 15 | IO42 (CAM_D4) | | 16 | IO17 (CAM_VSYNC) |
| 17 | IO45 (CAM_D0, strapping) | | 18 | IO18 (CAM_HREF) |
| 19 | IO46 (CAM_D3, strapping) | | 20 | IO0 (BOOT btn + IMU INT1) |
| 21 | IO47 (CAM_D1) | | 22 | ESP_EN (reset) |
| 23 | IO48 (CAM_D2) | | 24 | PWRON (AXP2101 power key) |
| 25 | **IO43** (ESP_TXD) | | 26 | IO7 (I²C SCL) |
| 27 | **IO44** (ESP_RXD) | | 28 | IO8 (I²C SDA) |
| 29 | GND | | 30 | GND |
| 31 | 3V3 | | 32 | 3V3 |

Consequences, in decreasing order of importance:

1. **GPIO43/44 reach the header** (pins 25/27), and their nets contain
   exactly two members each: the SoC pad and the header pin. **There is
   no CH343P or any other USB-UART bridge on this board** — USB data
   (GPIO19/20) runs straight to the USB-C connector through 22 Ω series
   resistors and an ESD array. Both §6 open items from the design doc
   are closed, in the direction the plan needs.
2. **All twelve ex-camera GPIOs reach the header** (17, 18, 21, 38–42,
   45–48), wired in parallel with the camera FPC. Using them costs the
   camera option — which for a POS terminal is the right trade (the
   scanner *is* the camera).
3. **VBUS is on the header** (pin 2, same net as USB-C VBUS into the
   AXP2101). This is the deployment power input: 5 V from a buck into
   pins 2 + 4 powers the board exactly as USB-C would, no connector
   surgery. Corollary warning in §4.
4. **GPIO4 is a correction to the design doc.** The integration doc's §6
   counted GPIO4 "free unconditionally". It is free at the SoC, but the
   schematic routes it to the unpopulated QSPI-panel pad
   (`LCD_QSPI_IO3`) and **not to the header** — treat it as unavailable
   without soldering. (Same for GPIO2/`LCD_QSPI_IO1`: nominally the
   panel-SDO/MISO line, not on the header.) This doesn't hurt: the
   header supplies more than enough pins.
5. Strapping pins **IO45/IO46 are exposed** on pins 17/19. Nothing that
   *drives* a line at reset may sit on them (a peripheral TX would
   qualify). This plan leaves them unconnected.
6. The even row carries **IO0 / EN / PWRON** on adjacent pins 20/22/24 —
   a slipped probe there resets or powers off the board. Both cable
   assemblies in this plan live on the **odd row**, partly for this
   reason.

### 2.2 Chosen pin assignment

| Function | GPIO | Header pin | Why this pin |
|---|---|---|---|
| Printer ← ESP TX | **21** | 5 (odd row) | camera-bus pin, free, non-strapping |
| Printer → ESP RX | **38** | 7 | ditto |
| Printer BUSY → CTS | **39** | 9 | ditto; CTS via GPIO matrix (§6.2) |
| Printer signal GND | — | 3 | completes a **contiguous odd-row block 3-5-7-9** — one 4-pin connector |
| Scanner ← ESP TX | **43** | 25 | boot-ROM log lands on the scanner, which ignores it (§5.4) — deliberately *not* on the printer |
| Scanner → ESP RX | **44** | 27 | UART0's ROM-default pin |
| Scanner signal GND | — | 29 | **contiguous block 25-27-29** — one 3-pin connector |

Reserved, not consumed: IO9/10/11 (TF card — future receipt journal),
IO12–16 (I²S — future scan beep, §10), IO40/41/42/47/48 + IO17/18
(spares for trigger/good-read/drawer lines), IO45/46 (strapping — leave
alone).

UART controller allocation: **UART1 = printer, UART0 = scanner**, UART2
spare. The ESP32-S3's GPIO matrix routes any UART to any pin, so the
choice is convention only: the scanner sits on UART0's ROM-default pins;
the printer's pins go through the matrix, which is free.

---

## 3. Bill of materials

| # | Item | Notes |
|---|---|---|
| 1 | ESP32-S3-Touch-LCD-3.5 | in hand, hardware-verified |
| 2 | EM5820-class 58 mm printer module, **TTL** variant | confirm TTL on the label, not RS232/USB |
| 3 | NETUM NT-EM61, **TTL** variant | the *default* variant — cheaper than the 5B's RS485 special-order. Confirm with seller; **never USB** (GPIO19/20 are the console) |
| 4 | 12 V / ≥3 A PSU | powers everything (§4) |
| 5 | Buck converter 12→8 V, ≥3 A | printer head supply; set before connecting |
| 6 | Buck converter 12→5 V, ≥1.5 A | board (via header VBUS) + scanner |
| 7 | ≥470 µF low-ESR electrolytic | at the printer VH terminals |
| 8 | Resistors 2× (1 kΩ + 2 kΩ) | level dividers, **only if** the printer talks 5 V (§5.2) — €0.10 contingency |
| 9 | Wire: 0.5 mm² (20 AWG) pair for printer power; 0.25 mm² signal | |
| 10 | 2.54 mm crimp housings: 1×4 and 1×3 (or a 2×16 shell) | the two odd-row blocks of §2.2 |
| 11 | Optional: inline fuse 2.5 A (printer branch) | |

Gone from the 5B BOM: SC16IS752 breakout, RS485 twisted pair, the
crystal-marking task. New Rust dependencies: **`escpos`** (same crate as
`local-server` and the 5B plan; §8.2) and **`embassy-sync`** (§8.6).

Single-supply alternative: a **5 V / 4 A** PSU with no bucks — printer VH
at 5 V (in the EM5820's 5–9 V window) prints lighter and slower but
works, board and scanner take 5 V natively. Fine for a first bench setup;
the 12 V tree is the recommendation for production print quality.

---

## 4. Power tree

This board has no 7–36 V VIN terminal (the 5B's luxury). It has something
almost as good once the header map is known: **VBUS on header pin 2**.

| Load | Source | Draw |
|---|---|---|
| Board (display + ESP32 + radio) | 5 V buck → header VBUS (pin 2) + GND (pin 4) | ≤0.5 A @ 5 V, Wi-Fi bursts included (computer-USB powered sessions are hardware-verified) |
| Printer head | 8 V buck | **1.5–2 A peak**, ~0.5 A average **(verify module rating)** |
| Scanner | 5 V buck | ≤0.35 A while illuminating **(verify — and confirm the TTL variant's VCC range; the 5–16 V spec on the listing may be the RS232/RS485 pigtail)** |

Rules:

- **Star ground at the PSU.** The printer's 2 A return must not share a
  conductor with any signal ground. Board GND via header pins 4/30;
  printer GND to the buck; buck and scanner grounds back to the PSU star
  point.
- One **signal** ground runs inside each data cable (header pin 3 ↔
  printer GND, pin 29 ↔ scanner GND) as the logic reference — mA, not
  head current.
- Set the 8 V buck **before** connecting the printer. Bulk capacitor at
  the printer end of the power pair.
- **Never feed header VBUS and USB-C at the same time.** They are the
  same net with no ORing diode — two hard 5 V sources in parallel.
  Bench workflow: flashing/console sessions run on USB-C with the
  external 5 V leg disconnected (printer/scanner PSU can stay up — the
  common signal ground is expected); deployed, the buck feeds pin 2 and
  USB-C stays empty.
- **AXP2101 VBUS input current limit (verify).** The PMU limits VBUS
  draw (register 0x16); the hardware default has been sufficient for
  every USB-powered dev session so far, but if the board browns out
  under Wi-Fi bursts on the bench supply, raise the limit in
  [../src/axp2101.rs](../src/axp2101.rs) `init()` — one register write,
  same pattern as the existing reg 0x30 write.
- Budget @ 12 V: 0.21 A (board, via buck) + 1.45 A (printer peak, via
  buck) + 0.16 A (scanner, via buck) ≈ **1.8 A peak → a 3 A supply has
  comfortable margin**.

Battery note: with a Li-ion cell on J7 the board itself rides through
mains dips (the AXP2101 switches over glitch-free); the printer and
scanner do not. Nothing in this plan depends on it — but it is a UPS
feature the 5B could not offer at any price, worth remembering for
deployment.

---

## 5. Physical build

### 5.1 Wiring tables

Printer cable — one 4-way housing on **header odd pins 3-5-7-9**, ≤1 m
to the module:

| Header | Signal | Printer |
|---|---|---|
| 3 | GND | GND |
| 5 (IO21) | ESP TX → | RX |
| 7 (IO38) | ESP RX ← | TX — direct **only if** the module is 3.3 V logic (§5.2) |
| 9 (IO39) | CTS ← | BUSY/DTR — same caveat; polarity §5.3 |

Printer power (from the 8 V buck, 0.5 mm² pair, bulk cap at the module):
VH+/VH− per the module label. **(verify pinout from the shipped label —
EM5820 boards ship TTL/RS232/USB variants on one PCB and the published
manual omits the pinout.)**

Scanner cable — one 3-way housing on **header odd pins 25-27-29**:

| Header | Signal | Scanner |
|---|---|---|
| 25 (IO43) | ESP TX → | RX (trigger/beep commands; carries boot noise, §5.4) |
| 27 (IO44) | ESP RX ← | TX — direct only if 3.3 V logic (§5.2) |
| 29 | GND | GND |

Scanner power: 5 V from the buck, not through the header (keeps the
AXP2101's VBUS budget for the board).

Seat the housings carefully: pin 1 (adjacent to the printer block) is
**VBAT** and pin 31 (adjacent to the scanner block) is **3V3** — a
one-position mis-seat puts a ground wire on a power net. Label the
housings; with a battery installed, wire with the battery disconnected.

### 5.2 Logic levels — the one place this board is stricter than the 5B

The 5B plan leaned on the SC16IS752's 5 V tolerant inputs. That cushion
is gone: **ESP32-S3 GPIOs are not 5 V tolerant** (absolute max ≈ VDD +
0.3 V = 3.6 V). Two module-dependent cases:

- **Module logic is 3.3 V** (common on recent EM5820-class boards and on
  NETUM engines, whose cores are 3.3 V): wire everything direct. **(verify
  per shipped module — a meter on TX idle level answers it in seconds.)**
- **Module logic is 5 V**: every *inbound* line needs a divider —
  printer TX → IO38, printer BUSY → IO39, scanner TX → IO44. 1 kΩ series
  / 2 kΩ to ground (5 V → 3.33 V) is fine at these baud rates over ≤1 m.

Outbound is the same story as the 5B (§4.3 there): the ESP drives TX at
3.3 V, which clears a ~2 V TTL threshold but not a true 5 V CMOS input
(0.7 × VDD = 3.5 V). Near-universally accepted on these modules;
**(verify printer RX threshold)** stays on the list.

While the printer boots ~100 ms behind the firmware, IO21 is high-Z
(pins come up as inputs); if the module's RX has no internal pull-up, a
floating line can clock in a noise byte. An optional 10 kΩ pull-up to
3.3 V at the connector retires it; most modules pull their own RX.

### 5.3 Printer BUSY → CTS

ESC/POS busy lines are conventionally **high = busy**. The ESP32-S3 UART
transmitter, with CTS flow control enabled, **pauses while the CTS input
is high** — the two conventions match, so a direct wire gives hardware
backpressure with zero firmware involvement, exactly like the 5B's
auto-CTS but without the bridge.

If the shipped module inverts BUSY **(verify on the bench, §9 step 2)**,
the fix is one line, not a rewire: the GPIO matrix inverts any input
signal (`InputSignal::with_input_inverter`, esp-hal
`gpio/interconnect.rs`). The fallback-of-the-fallback — polling BUSY as
a plain GPIO input and gating the spool drain in software — remains
available but should not be needed.

Paper-out mid-job needs no special handling: the printer stalls BUSY,
the TX FIFO fills, `write_async` pends, and everything resumes when
paper is reloaded (§8.1).

### 5.4 Scanner connection details

- **One-time configuration by setup barcodes** from the NETUM manual
  (not firmware): presentation/auto-sense mode, 9600 8N1, **CR+LF
  suffix**. Same procedure as the 5B plan.
- **Boot-ROM noise:** the ESP32-S3 ROM prints its boot log on GPIO43 at
  every reset, at 115200. The scanner's RX receives it as framing
  garbage and ignores it — the same conclusion the 5B plan reached for
  its RS485 bus, §4.4 there. This is *why* the scanner, not the printer,
  gets the 43/44 pair: a printer would render that garbage as smudged
  characters on paper at every reset. The `UART_PRINT_CONTROL` eFuse
  could silence the ROM permanently; it is irreversible — don't, without
  a separate decision.
- Unlike the 5B (SP3485 direction control unresolved), **the TX leg is
  actually usable here**: host → scanner commands (trigger, beep, LED)
  are a firmware feature away, not a hardware question (§10).
- Flashing is unaffected by scanner chatter: upload and console run over
  USB-Serial-JTAG (GPIO19/20), not UART0.

### 5.5 Mechanical notes (brief)

- Printer module needs a panel cutout, a straight paper path, and bay
  depth for a 58 mm × ⌀30–40 mm roll; mount so the paper exit clears the
  display bezel.
- Mount the scanner window angled a few degrees off the display glass to
  avoid its illumination reflecting back.
- Keep the printer power pair away from the display FFC **and the
  onboard 2.4 GHz antenna** (top edge of the board) — this board's Wi-Fi
  is load-bearing (SNTP → RTC), and a thermal head is a noisy neighbour.

---

## 6. The native UART path in detail

The 5B plan spent its longest chapter (§5 there) on the SC16IS752,
because the bridge was the one non-self-evident component. Here the
equivalent chapter is about what the ESP32-S3 already has — shorter,
because everything is on-chip and already in the HAL this repo pins.

### 6.1 What we're using

- **Two of the three UART controllers** (UART0 scanner, UART1 printer;
  UART2 spare). Console and flashing are USB-Serial-JTAG, so all three
  are architecturally free — the fact the whole plan stands on.
- **GPIO matrix**: any controller to any pin, with optional per-signal
  input inversion. IO21/38/39 are matrix-routed; IO43/44 are UART0's
  IO_MUX defaults.
- **128-byte TX and RX FIFOs per controller** (esp-hal
  `uart.ram_size` = 128 for the S3) — twice the bridge's 64, and
  serviced by interrupts instead of a 15 ms poll.
- **Hardware CTS flow control**: esp-hal 1.1.1 `Config::hw_flow_ctrl`
  (`HwFlowControl { cts: CtsConfig::Enabled, rts: RtsConfig::Disabled }`)
  plus `Uart::with_cts(pin)`. These APIs sit behind the `unstable`
  feature, which this project already enables. RTS and XON/XOFF stay
  off — nothing we receive needs throttling, and in-band flow control
  on a binary ESC/POS link is actively harmful (0x11/0x13 occur in
  data), same reasoning as the 5B plan.
- **Async mode** (`into_async()`): `write_async` refills the TX FIFO
  from an interrupt-driven waker, `read_async` returns on FIFO threshold
  or RX timeout. Baud comes from the 80 MHz APB clock with fractional
  division — no crystal to read, no divisor table, no §5.4-of-the-5B-doc.

### 6.2 Flow control semantics

With `CtsConfig::Enabled`, the transmitter checks CTS per byte and
pauses while the pin is high; a byte in flight always completes. Wired
to printer BUSY (high = busy), the printer throttles the SoC directly:

```
printer asserts BUSY ─► CTS high ─► TX FIFO stops draining
  ─► write_async stops being polled ─► printer task naturally parks
  ─► spool stops draining ─► UI watermark rises
```

One wire replaces the entire TXLVL-polling state machine of the 5B plan
(§5.6 there); the CPU cost of backpressure is zero, and the *evidence*
of backpressure (a `write_async` that hasn't resolved) is what the
status logic keys on (§8.1). If BUSY turns out inverted:
`with_input_inverter` at pin setup, one line (§5.3).

### 6.3 Tasks and executors

Both peripheral tasks run on the **thread executor**, beside the Wi-Fi
tasks — scan and print latencies are human-scale. The interrupt executor
(`software_interrupt1`) remains display-only: the LVGL flush protocol in
[../src/display.rs](../src/display.rs) spins on its ack, and nothing may
contend with it (standing rule from [../CLAUDE.md](../CLAUDE.md)).

Per-task memory: an `embassy_executor::task` static each (futures are a
few hundred bytes — the UART driver holds no buffers beyond the hardware
FIFO), plus the spool and channel statics of §8.

### 6.4 Boot-ROM noise, restated once

GPIO43 emits the ROM log at every reset — unavoidable without the
irreversible eFuse. The plan's answer is topological: the pin lands on
the peripheral that provably ignores garbage (scanner), and the printer
lives on matrix-routed pins that are high-Z until the firmware
configures them. Reset behavior is therefore: scanner sees ~1 KB of
framing errors and drops them; printer sees a floating-then-idle line
and prints nothing.

### 6.5 What this buys over the 5B's bridge, quantified

| | 5B (SC16IS752 over I²C) | here (native UART) |
|---|---|---|
| Added silicon | bridge breakout, address straps, crystal | none |
| Printer throughput ceiling | ~4.3 KB/s (64 B / 15 ms hub tick) | 960 B/s *at 9600*; **11.5 KB/s at 115200** — the wire is the only limit |
| Reaction latency | ≤15 ms (hub poll) | interrupt-driven, µs |
| CPU cost while printing | ~1.6 ms I²C copying per tick, thread priority | ~0 (FIFO + interrupts) |
| Touch coupling | printer shares the touch I²C bus | **none — different peripherals, different tasks** |
| Failure coupling | bridge NACK storm degrades the shared bus | UART failure is private to its task |
| Raster/logo printing | "the trap" — 5 s per logo, avoid | viable at 115200 (§8.8) |
| Flow control | auto-CTS in the bridge (good) | hardware CTS in the SoC (same quality, zero parts) |

The 5B plan's §5.7 lists what the bridge gives up versus a native UART;
this table is that section resolved. The trade was made at board level
instead: no isolated DI/DO, no CAN/RS485, no wide-range VIN (see the
integration doc's §6 table) — if any of those enter the product plan,
the answer changes boards, not wiring.

---

## 7. Firmware design — ground rules

Inherited constraints, all from [../CLAUDE.md](../CLAUDE.md):

1. **The sensor hub owns I²C exclusively**
   ([../src/sensors.rs](../src/sensors.rs)) — and this plan's headline
   is that it **stays byte-for-byte untouched**. Printer and scanner
   never touch the bus; touch/IMU/PMU/RTC polling cadence is unaffected
   by construction, not by careful budgeting.
2. **The interrupt executor is display-only** (§6.3). New tasks go on
   the thread executor via the main `Spawner`, same as
   [../src/wifi.rs](../src/wifi.rs).
3. **`alloc` is already on.** `.cargo/config.toml` builds
   `["core", "alloc"]` and `main.rs` installs the esp-alloc heap
   (73 744 B reclaimed dram2 + 64 KiB .bss; the Wi-Fi driver is the
   ~46 KiB anchor tenant). The `escpos` crate's `alloc` requirement —
   the one CLAUDE.md-level exception the 5B plan had to argue for — is
   already paid for here. The System tab's live heap stats are the
   watermark to watch; 8 MB of entirely-unused PSRAM is the escape
   valve (`esp-alloc` can take a second region) if it ever tightens.
4. **No steady-state logging** in the new paths — anomaly-only, matching
   the repo's 1 Hz debug heartbeat discipline. esp-println shares the
   USB-Serial-JTAG console; per-byte logging in a print loop would also
   wreck throughput measurements.
5. **Statics over heap for fixed buffers**, matching house style
   (`StaticCell`, atomics): the spool lives in `.bss` (§8.3). This
   board has the SRAM headroom the 5B lacked — no 29 KiB stack-guard
   drama here — but buffers still don't belong on task stacks.

New/changed files:

```
src/printer.rs           new  printer task: owns UART1, drains spool, polls status
src/spool.rs             new  print spool: SPSC ring + staging + escpos Driver impl
src/receipts.rs          new  receipt layouts via the escpos crate
src/scanner.rs           new  UART0 RX task, line assembly, channel
src/bin/uart-test.rs     new  diagnostic (§9)
src/bin/printer-test.rs  new  diagnostic (§9)
src/bin/scanner-test.rs  new  diagnostic (§9)
src/bin/main.rs          mod  UART construction + two task spawns
src/ui/system.rs         mod  printer status card, test-print button
src/ui/home.rs           mod  last-scan card
src/lib.rs               mod  module list
Cargo.toml               mod  escpos, embassy-sync
```

Untouched: `sensors.rs`, `display.rs`, `wifi.rs`, `sntp.rs`, `touch.rs`,
all existing drivers. The 5B plan's `sc16is752.rs` has no counterpart —
it is the module this board deletes.

---

## 8. Firmware design — module by module

### 8.1 `printer.rs` — printer task

Owns `Uart<'static, Async>` on UART1 (TX IO21, RX IO38, CTS IO39,
9600 8N1, CTS flow control enabled). Sketch:

```rust
#[embassy_executor::task]
pub async fn printer_task(mut uart: Uart<'static, Async>) {
  // init: ESC @ (reset), optionally GS a n (enable ASB), DLE EOT 4 (probe)
  // absent detection: 5 consecutive status-poll silences → PRINTER_ABSENT
  let mut ticker = Ticker::every(Duration::from_secs(1));
  let mut buf = [0u8; 256];
  loop {
    // 1. drain spool (chunked so status work interleaves)
    while let n @ 1.. = spool::take(&mut buf) {
      match with_timeout(Duration::from_secs(5), uart.write_async(&buf[..n])).await {
        Ok(Ok(_)) => flags::clear(STALLED),
        Ok(Err(e)) => { /* anomaly log, strike */ }
        Err(_timeout) => { flags::set(STALLED); /* CTS held — printer busy/out of paper.
             Bytes not accepted stay in `buf`/spool; retry next pass, job intact. */ }
      }
      poll_rx_nonblocking(&mut uart);          // ASB bytes may arrive any time
    }
    // 2. ~1 Hz: DLE EOT 4 → read reply → PRINTER_FLAGS atomic; LSR-equivalent
    //    error check via read result; strike/clear the health counter
    ticker.next().await;
  }
}
```

Design points:

- **Backpressure is hardware.** The task never checks a "busy" bit
  before writing; a stalled printer simply makes `write_async` slow. The
  5 s timeout converts "slow" into a UI-visible `STALLED` flag without
  aborting the job — paper reload resumes exactly where it stopped.
- **Status has two channels.** `DLE EOT 4` polls paper state at ~1 Hz
  while the link is idle. During a stall the poll bytes queue *behind*
  the stalled job (single wire, FIFO order), so the plan also enables
  **ASB (`GS a`)** where supported **(verify on the module)** — the
  printer then volunteers a status frame the moment paper runs out,
  on the RX line that is never blocked. Without ASB, paper-out during a
  job degrades to the `STALLED` flag plus last-known status — the same
  observable the 5B design settled for.
- **Absent detection** is response silence (5 strikes on the 1 Hz poll),
  not bus NACKs as on the 5B — a TTL line has no ack. Same
  `PRINTER_FLAGS` atomic either way: `PRESENT`, `PAPER_OUT`, `STALLED`,
  `ERROR` bits + a status-age field, decoded by the UI card.
- Boot recovery: init always starts with `ESC @`; a receipt torn by a
  mid-print reset is accepted as a truncated line on paper (5B §7.8
  semantics carry over).

### 8.2 ESC/POS generation — the `escpos` crate, decision carried over

The 5B plan's §7.2 evaluation (crate source review + cross-compile
check, 2026-07-25) **carries over in full**: `escpos = "=0.19.0"`,
`default-features = false`, `features = ["barcodes", "codes_2d"]`. The
target triple, toolchain channel, and `build-std` setup are identical in
this repo, and the two projects share the crate with
`~/dev_pos/local-server`, so receipt idioms stay uniform across the
estate. Re-run the one-command check (`cargo check --release` with the
dep added) as bring-up step 0-adjacent; no surprises expected.

Deltas from the 5B context, all favourable:

- The **`alloc` exception needs no arguing here** — this repo already
  runs a heap for esp-radio (ground rule 3).
- **Binary size is a non-issue twice over** (16 MB flash, and the LVGL
  binary already dwarfs the page-code tables).
- The `SpoolDriver` impl (~40 lines: `write` → `spool::stage`, `flush` →
  `spool::commit`, both `&self`-compatible with the atomics-based ring)
  transplants verbatim — see the 5B §7.2 listing.
- Residual risks unchanged: per-job `Vec`/`format!` churn on the heap
  (watch the System tab across the §9 soak), `name() -> String` (keep
  `DebugMode` off), pin the version exactly.
- The **`lib-printml` follow-up** (sharing receipt *layouts* with
  local-server by porting that no-IO templating layer to
  `no_std`+`alloc`) applies here identically — out of v1, decide before
  the first template is written twice.

### 8.3 `spool.rs` — print spool

Same design as 5B §7.3 — single-producer (LVGL/UI thread)
single-consumer (printer task) byte ring plus a staging buffer for
all-or-nothing job commits:

- Producer: `stage(&[u8])` accumulates, `commit()` publishes atomically
  (a torn half-receipt is worse than a rejected job); `Full` /
  `PrinterAbsent` surface to the UI. Consumer: `take(&mut [u8]) ->
  usize`, lock-free.
- **Storage: 8 KiB ring + 1 KiB staging as `.bss` statics** —
  the one deliberate divergence from the 5B file, which heap-leaked
  them to protect a 29 KiB stack budget this board doesn't have.
  Head/tail are `AtomicUsize`. 9 KiB of `.bss` is noise against this
  board's current map (the LVGL draw buffers alone are 50 KiB).
- 8 KiB ≈ 8 s of wire time at 9600 ≈ several queued receipts; the
  head/tail delta doubles as the UI watermark.

### 8.4 `scanner.rs` — UART0 RX task

- `Uart` (or just `UartRx`) on UART0: RX IO44, TX IO43, 9600 8N1, async;
  thread executor.
- Line assembly: accumulate to CR/LF, cap **128 bytes** (QR payloads),
  drop non-printables and empties — which also silently eats the
  boot-ROM garbage fragments (§5.4) and any truncated first line.
- Output: `embassy_sync::channel::Channel<CriticalSectionRawMutex,
  ScanEvent, 2>` with `ScanEvent { len: u8, data: [u8; 128] }` (~0.5 KiB
  static), drop-oldest on overflow — for presentation scanning the
  newest code wins. Plus a `SCAN_COUNT` atomic for the stats table.
  Identical contract to 5B §7.5.
- TX is initialized and idle in v1 (auto-sense mode). Unlike the 5B —
  where host-initiated triggering was hostage to unverified SP3485
  direction control — command-trigger mode here is purely additive
  firmware (§10).

### 8.5 UI changes

Extend existing tabs (no new tab; the LVGL pool sits at ~20 KiB used /
~25 KiB free of 48 KiB, and two cards cost ~2–3 KiB):

- **System tab — printer card:** state line (Ready / Busy / Stalled /
  Paper out / Absent from `PRINTER_FLAGS`), spool watermark, **Test
  print** button enqueueing a canned receipt (header, RTC date/time from
  the existing `RTC_HMS`/`RTC_DATE` atomics, counter, QR). Button
  disabled when absent/full — the backlight slider's atomics-in/
  atomics-out pattern, one more consumer.
- **Home tab — scan card:** last code (truncated), running count. Fed by
  draining the channel from the *frequent* update path (the cadence the
  IMU chart uses), not the 1 Hz fan-out — a scan should appear
  perceptibly instantly. If the pool tightens, bump `LV_MEM_SIZE` in
  [../lv-conf/lv_conf.h](../lv-conf/lv_conf.h) (full LVGL C rebuild,
  known cost).

### 8.6 Dependencies and constants

- `escpos = { version = "=0.19.0", default-features = false, features =
  ["barcodes", "codes_2d"] }` (§8.2).
- `embassy-sync` as a direct dependency for the channel — already in the
  tree transitively via embassy-net; align the version with `Cargo.lock`.
- Compile-time constants: `PRINTER_BAUD = 9600`, `SCANNER_BAUD = 9600`,
  `MAX_SCAN_LEN = 128`, `SPOOL_SIZE = 8192`, `SPOOL_STAGING = 1024`. Pin
  assignments live where all pins live: `main.rs` peripheral setup. No
  new env vars; if runtime flexibility is ever wanted, the
  `TZ_OFFSET_MINUTES` pattern is there to copy.

### 8.7 Failure matrix

| Condition | Detected by | Behavior | UI |
|---|---|---|---|
| Printer absent / powered off | 5 silent status polls | flags absent, spool rejects new jobs | "Printer: absent" |
| Paper out | ASB frame (if supported) else stall + last status | job parks in spool/FIFO, auto-resumes on reload | "Paper out" / "Stalled" |
| BUSY stall (buffer full, overtemp) | `write_async` timeout | `STALLED` flag, job intact, retries | "Busy" |
| Printer off **mid-job** | write timeout + status silence | strikes → absent; spool frozen | "Printer: absent" |
| Spool full | `commit()` fails | job rejected atomically | button disabled + toast |
| Scanner silent/unplugged | no signal exists (passive device) | nothing | count stops moving |
| Boot-ROM garbage → scanner | line validation | dropped | — |
| Reset mid-print | — | next boot `ESC @`; truncated line on paper accepted | — |

One row from the 5B table has no equivalent here: *"bridge absent —
printer collateral"*. There is no bridge, and no shared-bus fate: a
wedged printer link cannot touch the touch/IMU/RTC path even in theory.

### 8.8 Throughput notes

- Wire rate at 9600 8N1 = 960 B/s; a 400–700 B text receipt ≈ 0.5–0.8 s.
  The spool task's ceiling is the wire itself — no 15 ms-tick arithmetic
  anymore.
- **Raster is no longer the trap.** The 5B plan banished `GS v 0` logos
  (5 s each through the I²C straw). Here, if the module's self-test page
  advertises a higher baud, `PRINTER_BAUD = 115200` plus hardware CTS
  makes a 384×100 px logo ≈ 0.42 s — an after-verification config
  change, not a redesign. Text receipts still don't need it; QR/barcodes
  still render fastest via the printer's internal commands (`escpos`
  emits those already).
- Scanner: an EAN-13 + CRLF is 15 bytes ≈ 16 ms on the wire; scan→UI
  latency is dominated by the LVGL update cadence, well under 150 ms via
  the fast path (§8.5).

---

## 9. Bring-up sequence

Staged, one variable at a time, each step leaving a diagnostic binary
behind (house convention — `i2c-scan`, `lcd-test`, `touch-test`,
`sensor-test` already pay rent):

0. **Continuity pass (no firmware).** Beep header pins 3/5/7/9 and
   25/27/29 against the §2.1 table — upgrades the pinout from
   schematic-verified to hardware-verified in five minutes. Meter the
   peripheral modules' TX idle levels while at it (§5.2 divider
   decision).
1. **`uart-test`** — no peripherals. Jumper IO21→IO38 (header 5→7) and
   IO43→IO44 (header 25→27); the binary sends a pattern out each TX and
   verifies it back on the paired RX, then reports CTS pin state on
   IO39. Proves pin routing, matrix config, and both async drivers with
   zero external hardware.
2. **`printer-test`** — printer on the 8 V buck. `ESC @` + "HELLO 58MM"
   + feed; `DLE EOT 4` with and without paper; then the polarity check:
   start a long test print, open the head lever mid-job, confirm TX
   stalls (CTS) and resumes on close. Also prints the module's self-test
   page procedure (hold FEED at power-up) to capture actual baud, and
   probes `GS a` ASB support. Retires §11 items 3–6 in one session.
3. **`scanner-test`** — scanner configured via setup barcodes; dump
   assembled lines to the console. Present test codes; confirm suffix
   and framing; reset the board mid-session and confirm the boot-noise
   fragment is dropped, not displayed. (Monitor non-interactively:
   `espflash monitor --non-interactive --elf …`, per CLAUDE.md.)
4. **Integration** — full app: spool, printer task, scanner task, UI
   cards.
5. **Soak** — acceptance below.

Acceptance checklist:

- [ ] Continuity pass matches §2.1; TX idle levels metered, divider
      decision recorded
- [ ] `uart-test` loopback passes on both UARTs
- [ ] Test print correct at 9600; QR + EAN-13 render via printer commands
- [ ] Head-lever stall: TX pauses in hardware, resumes without byte loss
- [ ] Paper-out visible in UI within ~2 s (ASB) or as Stalled (no ASB);
      job completes after reload
- [ ] Boot with printer and/or scanner disconnected: UI shows absent,
      system otherwise healthy
- [ ] Reset mid-print: recovers to a working printer without power
      cycle; scanner shows no garbage line
- [ ] 2-minute continuous print soak: touch responsive, FPS overlay
      steady, hub cadence unaffected (expected by construction — verify
      anyway), heap watermark stable across ≥100 receipts (escpos churn)
- [ ] Scan while printing: no loss either direction
- [ ] Wi-Fi/SNTP unaffected with printer power pair routed per §5.5
      (antenna clearance)

---

## 10. Future extensions (explicitly out of v1)

- **Scan-confirmation beep.** Standard POS UX, and this board can do it
  natively: ES8311 codec + NS4150B amp are onboard, speaker header
  populated, I²S pins 12–16 deliberately left unconsumed by this plan.
  The schematic settles the missing control detail: the amp enable is
  **PA_CTRL on TCA9554 EXIO7** (the Waveshare demo's `pa_pin = NC`
  notwithstanding) — one expander write via the existing hub-owned
  driver. Needs an I²S bring-up that doesn't exist yet; that, not the
  beep, is the work.
- **Command-trigger scanning / host-controlled beep+LED** — scanner TX
  is already wired (IO43); this is protocol work in `scanner.rs` against
  the NETUM serial command set **(verify the TTL variant accepts serial
  commands — NETUM engines usually do)**.
- **115200 + logo raster** once the self-test page confirms the module
  supports it (§8.8).
- **Receipt journal on TF card** — SDMMC pins 9/10/11 reserved untouched;
  note the schematic routes the card's D3/CS to **EXIO3**, so SPI-mode
  fallback would involve the expander (SDMMC 1-bit doesn't need it
  beyond the pull-up already fitted).
- **Cash-drawer kick.** The honest gap versus the 5B: no isolated DO
  ports here. Options: a spare GPIO (40/41) + external MOSFET + flyback
  diode, or — with no camera fitted — **EXIO0 (CAM_PWDN)** through the
  same MOSFET, driven by the hub like every other expander pin. Drawer
  solenoids are amps-scale: dedicated 12 V branch, never through board
  nets.
- **Battery-backed operation** (AXP2101 + Li-ion on J7): the terminal
  rides through mains dips, printer pauses (BUSY/absent handles it),
  UI stays up. Free resilience if a cell is fitted; needs the charge
  parameters from CLAUDE.md's AXP2101 notes (TS-pin measure already
  disabled in `axp2101::init` — the known gotcha is retired).

---

## 11. Open verifications (blocking vs. non-blocking)

Blocking a purchase:

1. Scanner ordered as **TTL variant**; printer as **TTL** variant.
   (Never USB — GPIO19/20 are the console. The 5B doc's RS485
   requirement does **not** apply here; TTL is the default/cheapest
   option for both.)
2. Printer module's rated supply range (sets the buck; 5–9 V typical).

Blocking first power-up (bench, minutes):

3. Printer TX/BUSY logic level — 3.3 V direct or 5 V ⇒ dividers on
   IO38/IO39 lines (§5.2 — **stricter than the 5B**, the S3 is not 5 V
   tolerant).
4. Scanner TX logic level — same question for the IO44 line; plus the
   TTL variant's actual VCC requirement (§4).
5. Printer BUSY polarity (direct CTS vs `with_input_inverter`, §5.3).
6. Printer default baud, `DLE EOT` support, `GS a` (ASB) support —
   self-test page + `printer-test` (§9 step 2).
7. Header continuity per §2.1 (schematic-verified → hardware-verified,
   §9 step 0).

Non-blocking (v1 works without the answer):

8. AXP2101 VBUS input current-limit default vs. board peak draw on the
   bench 5 V feed (§4) — raise via reg 0x16 in `axp2101.rs` if needed.
9. Exact printer peak current (fuse rating margin).
10. NT-EM61 current at 5 V, idle and illuminating.
11. NETUM TTL serial-command support (gates the §10 trigger/beep
    extension only).

The 5B plan's list had six bench items *plus* two board-level unknowns
(SP3485 direction control, bridge crystal). Items of that second kind —
the ones that could have invalidated the architecture — no longer exist:
the schematic closed them all in §2.

---

## Appendix A — other schematic findings (beyond the header)

Read while extracting §2; recorded here because several complete or
correct [../CLAUDE.md](../CLAUDE.md)'s open questions. All
schematic-verified, none yet exercised on hardware. Worth folding into
CLAUDE.md's hardware tables on the next pass:

**TCA9554 expander — the full map** (CLAUDE.md documents only EXIO1):

| Pin | Net | Note |
|---|---|---|
| EXIO0 | CAM_PWDN | free for reuse when no camera is fitted (§10 drawer idea) |
| EXIO1 | LCD_RST | **also the touch controller's RST** — the touch-reset-shared-with-panel guess in CLAUDE.md is confirmed |
| EXIO2 | TP_INT | touch interrupt *is* wired after all — to the expander, not a GPIO; polling remains the right call |
| EXIO3 | SD_CS (TF card D3/CS) | matters only for SPI-mode SD; SDMMC 1-bit needs it high (pull-up fitted) |
| EXIO4 | RTC_INT (PCF85063) | answers CLAUDE.md's "check schematic" |
| EXIO5 | AXP_IRQ (AXP2101) | ditto — PMU IRQ exists, behind the expander |
| EXIO6 | SYS_OUT | feeds a FET into the PWRON node — looks like software power-off **(verify before use)** |
| EXIO7 | PA_CTRL | NS4150B speaker-amp enable (§10 beep) |

Interrupt-shaped consequence: every "INT" on this board (touch, RTC,
PMU) terminates at the expander, whose own INT output is not wired to
the ESP32 — so the hub's polling architecture isn't just a choice, it's
the only option. The one exception: **QMI8658 INT1 lands on GPIO0**,
shared with the BOOT button (and header pin 20).

Also confirmed: USB D± (GPIO19/20) run straight to the USB-C shell
(22 Ω + ESD, no bridge chip); GPIO4 routes only to the unpopulated
QSPI-panel pad (`LCD_QSPI_IO3`), reachable by soldering, not via the
header (the §2.1 item 4 correction). GPIO2 (`LCD_SPI_MISO`) appears
similarly stranded in the netlist — the panel's SDO sits on a separate
`LCD_MISO` net — but CLAUDE.md's demo-derived table lists it as panel
MISO, so treat GPIO2 as panel-owned unless a closer schematic read
shows otherwise. Battery (J7, VBAT) and backup cell (J6, VBACKUP) nets
surface on the header/AXP as CLAUDE.md describes.

## Sources

- **Board schematic** —
  [ESP32-S3-Touch-LCD-3.5-Schematic.pdf](https://files.waveshare.com/wiki/ESP32-S3-Touch-LCD-3.5/ESP32-S3-Touch-LCD-3.5-Schematic.pdf)
  (fetched 2026-07-25) — J8 pinout, expander map, USB path, GPIO4
  routing. The single source for everything marked schematic-verified.
- [../CLAUDE.md](../CLAUDE.md) — hardware-verified pin/address tables
  (2026-07-23), display/LVGL/hub architecture, toolchain.
- Design discussion:
  [printer-scanner-integration.md](../../ESP32-S3-5inch-Display/docs/printer-scanner-integration.md)
  (§6 this board, §7 board ranking); 5B implementation plan:
  [printer-scanner-implementation.md](../../ESP32-S3-5inch-Display/docs/printer-scanner-implementation.md)
  (SC16IS752 chapter, escpos evaluation §7.2, spool/scanner module
  designs carried over).
- **esp-hal 1.1.1** (local registry copy) — `uart/mod.rs`:
  `HwFlowControl`/`CtsConfig`, `with_cts`, `write_async`/`read_async`,
  128-byte FIFO (`uart.ram_size` in esp-metadata-generated 0.4.0);
  `gpio/interconnect.rs`: `with_input_inverter`. All present in the
  released version this repo pins (`~1.1.0`, `unstable` feature on).
- `escpos` 0.19.0 — evaluation and xtensa build check per the 5B doc
  (2026-07-25), local registry copy.
- Peripheral references (carried over from the design doc):
  [NETUM NT-EM61](https://us.amazon.com/NETUM-NT-EM61-Embedded-Barcode-Scanner/dp/B0FVM3BXGD),
  [EM5820 manual](https://manuals.plus/ae/1005004513800835),
  [DFRobot embedded thermal printer wiki](https://wiki.dfrobot.com/Embedded%20Thermal%20Printer%20-%20TTL%20Serial%20SKU:%20DFR0503-EN).
