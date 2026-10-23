# cw32l012-hal

[![crates.io](https://img.shields.io/crates/v/cw32l012-hal.svg)](https://crates.io/crates/cw32l012-hal)
[![docs.rs](https://docs.rs/cw32l012-hal/badge.svg)](https://docs.rs/cw32l012-hal)
[![license](https://img.shields.io/crates/l/cw32l012-hal.svg)](https://crates.io/crates/cw32l012-hal)
[![CI](https://github.com/SunWithCloude/Rust_Cw32L012/actions/workflows/ci.yml/badge.svg)](https://github.com/SunWithCloude/Rust_Cw32L012/actions/workflows/ci.yml)

**English** | [简体中文](README.zh-CN.md)

A `no_std` hardware abstraction layer for the **CW32L012** (ARM Cortex-M0+) from
Wuhan Xinyuan Semiconductor (CMSemicon), written from scratch against the
vendor's user manual — no C SDK, no `svd2rust`, zero dependencies by default.

```toml
[dependencies]
cw32l012-hal = "0.8"
```

- Source: https://github.com/SunWithCloude/Rust_Cw32L012
- API docs: https://docs.rs/cw32l012-hal
- Per-example test log (measured values, pass criteria, pitfalls): [TESTING.md](TESTING.md)

## Why this exists

The CW32L012 is a cheap, quite capable M0+ part (64 KB flash, 8 KB SRAM,
96 MHz capable). It ships with a Keil pack and a C standard-peripheral library,
but there is no Rust support at all — no PAC, no HAL, and no probe-rs target.

This crate covers all three: a hand-written register layer (`pac`), a typed HAL
on top of it, and a self-contained runtime (vector table + `.data`/`.bss` init),
so building for `thumbv6m-none-eabi` produces a flashable ELF with **no external
crates**. `embedded-hal 1.0` traits are available behind the default `ehal-1`
feature; turn default features off and you are back to zero dependencies.

Everything below has been run on real silicon — the numbers in
[TESTING.md](TESTING.md) are measured, not estimated.

## Chip overview

| Item | Value |
|------|-------|
| Core | ARM Cortex-M0+ (24 / 48 / 96 MHz depending on supply voltage) |
| Internal RC | HSIOSC nominal 96 MHz (reset default HSI = 96/24 = 4 MHz) |
| Flash | 64 KB @ `0x0000_0000` |
| SRAM | 8 KB @ `0x2000_0000` (with parity) |
| Low power | Sleep / DeepSleep, wakeup from LPTIM, RTC, WWDT, VC, GPIO |
| Peripherals | UART×3, SPI×3, I2C×2, ATIM, GTIM×4, BTIM×3, LPTIM, HALLTIM, ADC×2 (12-bit), DAC, VC×4 (comparators), OPA×2, DMA×4, CORDIC, CRC/EAU |

## Features

- **Register layer (`pac`)** — typed register blocks, the full base-address map,
  bit-field constants, and **compile-time layout assertions**
  (`size_of::<Gpio>() == 0x64` and friends). Raw access stays available as an
  escape hatch.
- **Clocks (`rcc`)** — HSI divider (16-entry lookup table), sysclk source switch,
  HCLK/PCLK prescalers, peripheral gating with the `0x5A5A` write-protect key
  handled for you. Frequency changes automatically resync the flash wait states
  (UM §4.5.2 safety sequence), so `run_hsi_at_96mhz()` is one call. Factory flash
  TRIM is loaded on `Rcc::new` (§4.4.2).
- **GPIO (`gpio`)** — push-pull / open-drain outputs, floating / pull-up /
  pull-down inputs, alternate functions, atomic set / clear / toggle (BSRR/TOG),
  bus-level readback for open-drain. External interrupts with edge selection,
  R1W0 flag clearing and a hardware digital filter. `Pin::into_analog()` clears
  the digital driver — **required** for ADC/VC/OPA/DAC pins, otherwise the output
  driver fights the analog node (we hit this on PB00 with OPA1_OUT).
- **UART / SPI / I2C** — blocking drivers. UART is 8-N-1 with integer+fractional
  baud division and self-healing overrun handling. SPI does all four CPOL/CPHA
  modes, MSB/LSB first, and picks the baud divider automatically. I2C derives
  100 kHz / 400 kHz timings from the datasheet formula (regression-tested against
  the official table 23-4 values), reports NACK/arbitration/timeout as structured
  errors, and offers `scan()`.
- **Timers** — BTIM (periodic/one-shot, PSC/ARR auto-paired), GTIM (PWM, edge
  aligned mode 1, duty in 0.01 % units), ATIM (complementary PWM with dead time
  and break input), LPTIM (PCLK/LSE/LSI, deep-sleep wakeup), HALLTIM (24-bit,
  3 capture channels, PWM, `SOFTCAP`).
- **Analog** — ADC single conversion and sequence scan (2–8 channels), internal
  temperature sensor and 1.2 V reference channels (`vdda_mv()`,
  `temperature_mc()`, integer math straight from the manual), DAC dual channel,
  VC comparators (hysteresis, window mode, filtering, interrupts), OPA
  (external gain / internal PGA ×2…×32 / follower).
- **DMA (`dma`)** — 4 channels (channel index = priority), 8/16/32-bit widths,
  software trigger plus **all 40 peripheral hardware triggers** from table 8-8.
  BLOCK/BULK modes, address increment and reload, auto-restart for ring buffers.
  `Dma::mem_to_mem()` is a one-liner; `Config::periph_to_mem()` /
  `mem_to_periph()` configure streams.
- **RTC (`rtc`)** — BCD calendar with automatic leap years, LSE/LSI/HSE/HSI
  source selection with oscillation-ready polling, validated `DateTime`,
  glitch-free `datetime()` (double read), two alarms with independently maskable
  hour/minute/second, periodic interrupt, wakeup timer, `KEY` unlock handled
  automatically.
- **Flash self-programming (`flash`)** — 512-byte page erase, 32-bit word
  programming, `write_page()`, `PAGELOCK` group handling, structured decoding of
  all five `ISR` error flags, and a linker-symbol (`_etext`) guard that keeps
  erase/program out of the code region.
- **Watchdogs** — IWDT (key-value sequence, `TimeoutMs` auto-fits PRS/ARR,
  always rounding *up* so the timeout is never shorter than requested) and WWDT
  (window solved for you, feeding too early is rejected).
- **Delay** — ms/µs/ns. Short delays use a 3-cycle/iteration busy loop (avoids
  SysTick start/stop overhead), long ones use SysTick with 64-bit math; conversion
  factors are precomputed in `Delay::new`, so the common clock rates need no
  64-bit division at all. Measured error < 1 % at 4 MHz.
- **Runtime (`runtime`, default feature)** — 48-entry vector table with weak
  symbols, `interrupt!` macro, NVIC control, PRIMASK critical sections. No
  `cortex-m-rt` required.
- **Ecosystem traits** — `embedded-hal 1.0` (delay, digital, SPI bus + device,
  I2C, PWM `SetDutyCycle`), `embedded-io` `Read` / `Write` for UART,
  `defmt::Format` on every error type and the enums worth logging, and a
  `critical-section` implementation backed by `PRIMASK` (nesting-safe: the
  previous mask is read back and restored, so an inner critical section never
  re-enables interrupts early).

### Feature flags

| Feature | Default | Adds |
|---|---|---|
| `runtime` | ✅ | Vector table, reset init (.data/.bss), `interrupt!` macro, NVIC control |
| `ehal-1` | ✅ | `embedded-hal 1.0`: delay / digital / SPI (bus + device) / I2C / PWM |
| `critical-section-impl` | — | `PRIMASK`-based `critical-section` implementation (single-core M0+) |
| `embedded-io` | — | `embedded_io::Read` / `Write` for `Uart` (requires Rust 1.81+) |
| `defmt` | — | `defmt::Format` derives for error types and key enums (requires Rust 1.81+) |

Turn everything off (`default-features = false`) and the crate is back to **zero
external dependencies**. Only one `critical-section` implementation may exist per
binary — if you also use `cortex-m` with `critical-section-single-core`, drop ours.

Note: the `defmt` feature only links when compiling for the target; defmt's
sections are not usable in a host `cargo test` binary.

## Getting started

### 1. Toolchain

```bash
rustup target add thumbv6m-none-eabi
```

### 2. Build

```bash
cargo build --target thumbv6m-none-eabi --release --example blink
```

The repository ships `.cargo/config.toml` with the linker arguments
(`-Tmemory.x -nmagic`), so no extra setup is needed.

### 3. Flash

The CW32L012 is **not yet in probe-rs's built-in target list**, so point it at
the target description bundled in this repo:

```bash
probe-rs download \
  --chip CW32L012x8 \
  --chip-description-path tools/CW32L0_Series.yaml \
  target/thumbv6m-none-eabi/release/examples/blink
probe-rs reset --chip CW32L012x8 --chip-description-path tools/CW32L0_Series.yaml
```

The same arguments are already wired into the `probe-rs run` runner in
`.cargo/config.toml`, so `cargo run --target thumbv6m-none-eabi --release --example blink`
works too. `tools/CW32L0_Series.yaml` was generated with `target-gen` from the
vendor DFP pack 1.0.2; an upstream submission to probe-rs is planned (see
[Roadmap](#roadmap)).

### 4. Hello, blink

```rust
#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

#[no_mangle]
fn main() -> ! {
    let p = match Peripherals::take() { Some(p) => p, None => loop {} };

    // Reset default: SysClk = HCLK = PCLK = 4 MHz (HSI = HSIOSC / 24)
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    let mut led = Pin::new(p.gpioa, PortId::A, 5, &mut rcc).into_output(Level::Low);
    let mut delay = Delay::new(&clocks);

    loop {
        led.toggle();
        delay.delay_ms(500);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}
```

### 5. Interrupts

```rust
cw32l012_hal::interrupt!("GPIOA_irq", gpioa_isr, {
    // handle the interrupt
});
```

## Debugging in VS Code

Install the **probe-rs debugger** extension (`probe-rs.probe-rs-debugger`).
probe-rs **0.32 already ships CW32L0 target support** (`probe-rs chip list` shows
`CW32L0 Series` with the `x8` variants), so nothing extra has to be configured.
`.vscode/launch.json` in this repo has two configurations:

```jsonc
{
  "type": "probe-rs-debug",
  "request": "launch",
  "name": "probe-rs · blink (CW32L012)",
  "chip": "CW32L012x8",
  "probe": "0483:3748",
  "wireProtocol": "Swd",
  "flashingConfig": { "flashingEnabled": true, "haltAfterReset": true },
  "coreConfigs": [
    { "coreIndex": 0, "programBinary": "${workspaceFolder}/target/thumbv6m-none-eabi/release/examples/blink" }
  ]
}
```

Press **F5**: it builds, flashes and halts at the entry point. Breakpoints, call
stack, variables and the peripheral register view all work.

Two things that cost us time:

- **`wireProtocol` must be `Swd`.** Without it probe-rs falls back to JTAG and
  fails with `JtagGetIdcodeError`.
- **On probe-rs < 0.32** the chip is unknown; add
  `"chipDescriptionPath": "${workspaceFolder}/tools/CW32L0_Series.yaml"`. The
  generated target file is still shipped for that case (and for the CLI flag
  `--chip-description-path`).
- **Cortex-Debug will not work at all.** It is GDB-based and needs
  `arm-none-eabi-gdb` plus a GDB server (OpenOCD / ST-Link GDB server / J-Link);
  none of them ship a CW32L0 target definition. The probe-rs extension speaks
  DAP directly and needs no GDB.

`tools/dap_check.py` is a headless smoke test of the debug path (spawns
`probe-rs dap-server`, sends a real DAP `initialize`/`launch`/`configurationDone`
sequence, prints the resolved stack frame) — handy when the IDE misbehaves.

## Crate layout

```text
src/
├── lib.rs        # Peripherals singleton, module wiring
├── pac.rs        # register blocks + bit constants + layout assertions
├── rcc.rs        # reset & clock control
├── gpio.rs       # typed pins: Output / Input / AfPin
├── uart.rs spi.rs i2c.rs
├── btim.rs gtim.rs atim.rs lptim.rs halltim.rs
├── adc.rs dac.rs vc.rs opa.rs
├── dma.rs rtc.rs flash.rs wdt.rs wwdt.rs
├── delay.rs
├── ehal.rs       # embedded-hal 1.0 impls
└── runtime.rs    # vector table + reset init (built only for `arm` + `none`)
examples/         # 32 examples, all run on hardware
tools/            # elf2bin.py, CW32L0_Series.yaml (probe-rs target)
```

## Memory map

| Constant | Address | Note |
|---|---|---|
| `FLASH_BASE` | `0x0000_0000` | 64 KB main flash |
| `SRAM_BASE` | `0x2000_0000` | 8 KB SRAM |
| `BOOT_BASE` | `0x0010_0000` | 2 KB bootloader (UART ISP) |
| `SYSCTRL_BASE` | `0x4000_4000` | system control (clock/reset) |
| `GPIOA/B/C/F_BASE` | `0x4800_0000/0100/0200/0300` | GPIO (AHB) |

The complete table lives in `src/pac.rs`.

## Interrupt vector table (48 entries)

| IRQ | Source | IRQ | Source |
|---|---|---|---|
| 0 | WDT | 16 | GTIM1 |
| 1 | LVD | 17 | GTIM2 |
| 2 | RTC | 18 | GTIM3/4 |
| 3 | FLASHRAM | 19 | LPTIM |
| 4 | RCC | 20 | BTIM1 |
| 5–8 | GPIOA/B/C/F | 21–22 | BTIM2, BTIM3/HALLTIM |
| 9–10 | DMA1/2, DMA3/4 | 23–24 | I2C1, I2C2 |
| 11 | CORDIC | 25–26 | SPI1, SPI2/3 |
| 12 | ADC1 | 27–29 | UART1/2/3 |
| 13 | ATIM | 30 | ADC2/DAC |
| 14–15 | VC1/3, VC2/4 | 31 | FAULT |

## Register-level confidence

The register layer was cross-checked against the community
[`cw32l012_pac` 0.1.1](https://crates.io/crates/cw32l012_pac) crate (generated by
`svd2rust` 0.37.1) and against UM V1.4 directly. Three places where the official
SVD is wrong or incomplete, and this crate follows the manual instead:

- **`GPIOx_PDR @ 0x0C`** — missing from the official PAC entirely (the whole
  `0x08–0x0F` range is marked reserved). It exists (UM V1.4 §9.6.3), but only
  **PF3** actually has a resistor; reset value `0x0000_0008` = PF3 pull-down on.
  `Pull::Down` is therefore a no-op on every other pin.
- **`IRQ31 = FAULT @ 0xBC`** — the HSE/LSE runtime-failure interrupt is missing
  from the official PAC's 31-entry `__INTERRUPTS` array.
- **`SYSCTRL.PLL @ 0x28`** — a leftover from a CW32F SVD; the L012 clock tree has
  no PLL. Treated as reserved here.

Every peripheral block also has a `const` assertion on its `size_of`, so a wrong
offset is a compile error rather than a silent misread.

## Verification status

All of the following ran on a CW32L012 start-kit with an ST-LINK clone and
`probe-rs`. Measured values and pass criteria are in [TESTING.md](TESTING.md).

| Peripheral | What was verified |
|---|---|
| GPIO | full path (AHBEN key → ANALOG clear → DIR → BSRR/TOG), 500 ms square wave on PC13 |
| SPI + 96 MHz | ST7789 over hardware SPI at 24 MHz: init + clear + full-screen colour bars in < 3 s (software SPI took ~90 s) |
| BTIM | 2 Hz update interrupt toggling PC13, `wfi` wakeup |
| ADC (internal) | VDDA = 3282 mV from the 1.2 V reference, die temperature 26.9 °C |
| GTIM | 1 kHz PWM, ARR/PSC/CNT read back exactly |
| DAC → ADC | on-chip loopback, max error **3 LSB** over 15 sweeps |
| IWDT | real reset loop after ~1.5 s without feeding (observed via a boot counter) |
| ATIM | complementary PWM, 250 µs dead time, break input; BDTR = 0x94FF read back |
| WWDT | 51 early feeds rejected, 50 in-window feeds accepted, zero resets |
| Low power | Sleep×10 (BTIM wakeup), DeepSleep×6 (IWDT wakeup), sysclk back on HSI after wakeup |
| LPTIM | deep-sleep wakeup, 3 wakeups, LSI at 32768 Hz (0.1 % error) |
| HALLTIM | capture width 4097 counts mid-run, ARR/CCR readback |
| VC / OPA | comparator cross count 2035 of 2048 expected, low/high flags toggling |
| DMA | mem→mem 64 words bit-exact; ADC→DMA 256 samples with zero CPU involvement, MIN/MAX/MEAN = 2045/2050/2047 for a DAC code of 2048 |
| RTC | set 23:59:50, read back next day 00:00:01 (Saturday), alarm fired at the programmed second |
| Flash | page 127 counter survives two resets (1 → 2 → 3) |

**76 unit tests** (`cargo test --lib`) cover baud/timing/period/calibration/
dead-time/window math, including regressions against the official values printed
in the manual.

## Known limitations

- **UART TX/RX over a real wire has not been exercised yet** (the driver is
  implemented and the loopback example is ready; a scope capture is pending).
- I2C timings follow table 23-4 but have not been validated against a physical
  slave — use `scan()` first on a new board.
- ADC external pin channels share the (verified) internal conversion path; not
  separately measured.
- On **this particular board** the internal temperature sensor reads ≈ 0 counts
  while the 1.2 V reference on the same path works fine. The vendor's own
  example reproduces it, so it looks like a dead TS node on the die, not a
  driver bug. `adc` guards against it with `AdcError::SensorDead`.
- Factory TRIM loading assumes the calibration area is programmed; out-of-range
  values are skipped.
- `CNT.REPEAT` is cleared by hardware after each transfer, so multi-channel
  sequence + DMA needs a re-`configure()` per round (single-channel EOC-triggered
  DMA is verified).

## Roadmap

- [x] PAC, clock tree, GPIO, UART, SPI, I2C, timers, ADC/DAC, DMA, RTC, flash
- [x] `embedded-hal 1.0` traits, self-contained runtime
- [x] Ecosystem: `critical-section`, `embedded-io`, `defmt` (v0.9)
- [x] 32 examples verified on hardware
- [x] probe-rs target support (upstreamed — `probe-rs 0.32+` ships `CW32L0 Series`)
- [ ] `.bin` / `.hex` packaging script (`cargo-binutils`)
- [ ] UART cross-wire validation
- [ ] GitHub Actions hardware-in-the-loop job

## Data sources

- *CW32L012 User Manual* V1.4 (Wuhan Xinyuan Semiconductor, 2026-06) — the single
  source of truth for every register definition
- *CW32L012 Datasheet* — electrical characteristics and pinout
- `WHXY.CW32L012_DFP` 1.0.2 — device family pack, used only to generate the
  probe-rs target YAML

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.

Not affiliated with or endorsed by Wuhan Xinyuan Semiconductor / CMSemicon.
CW32 is their trademark; the register definitions here were transcribed from the
publicly available user manual.
