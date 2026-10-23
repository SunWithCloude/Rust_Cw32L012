# cw32l012-hal

[English](README.md) | **简体中文**

[![crates.io](https://img.shields.io/crates/v/cw32l012-hal.svg)](https://crates.io/crates/cw32l012-hal)
[![docs.rs](https://docs.rs/cw32l012-hal/badge.svg)](https://docs.rs/cw32l012-hal)
[![license](https://img.shields.io/crates/l/cw32l012-hal.svg)](https://crates.io/crates/cw32l012-hal)
[![CI](https://github.com/SunWithCloude/Rust_Cw32L012/actions/workflows/ci.yml/badge.svg)](https://github.com/SunWithCloude/Rust_Cw32L012/actions/workflows/ci.yml)

> 源码仓库：https://github.com/SunWithCloude/Rust_Cw32L012
> ｜ crates.io：`cw32l012-hal = "0.8"` ｜ 测试记录见 [TESTING.md](TESTING.md)


[武汉芯源半导体（CMSemicon）CW32L012](https://www.whxy.com) 的 Rust `no_std` 硬件抽象层（HAL）。

## 芯片概况

| 项目 | 参数 |
|------|------|
| 内核 | ARM Cortex-M0+（24MHz/48MHz/96MHz 电压范围相关） |
| 内部高频振荡器 | HSIOSC 标称 96MHz（复位默认 HSI = 96/24 = 4MHz） |
| Flash | 64KB @ `0x0000_0000` |
| SRAM | 8KB @ `0x2000_0000`（带奇偶校验） |
| 低功耗 | 多种低功耗模式，LPTIM / RTC / WWDT 唤醒源 |
| 外设 | UART×3、SPI×3、I2C×2、ATIM/GTIM×4/BTIM×3/LPTIM、12 位 ADC×2、DAC、比较器×4、DMA×4、CORDIC、CRC/EAU |

## 特性（v0.8.0）

- **零负担生态兼容**：默认 feature `ehal-1` 提供 **embedded-hal 1.0** 全套
  trait（DelayNs / digital / SpiBus+SpiDevice / I2c），可直接对接生态驱动；
  关闭后 crate 回到零外部依赖
- **PAC 寄存器层**（`pac`）：SYSCTRL / GPIO / UART / SPI / I2C / BTIM / ADC
  寄存器块类型化访问 + 全部外设基地址表 + 位段常量 + 编译期布局静态断言
- **时钟控制**（`rcc`）：HSI 分频（16 档查找表编码）、SysClk 源切换、HCLK/PCLK 分频、外设时钟门控（自动处理 `0x5A5A` 写保护键）；变频自动同步 FLASH 读等待周期（手册 §4.5.2 安全流程，96MHz 满速一键直达）；`Rcc::new` 自动加载 FLASH 工厂 TRIM 校准（§4.4.2，HSIOSC 精度 ±5% → 出厂校准级）
- **GPIO**（`gpio`）：推挽 / 开漏输出、输入（浮空、上拉、下拉）、复用功能，原子置位 / 清零 / 翻转（BSRR / TOG），开漏总线电平回读；自动清 ANALOG 模拟位；**外部中断 API**（沿选择 + R1W0 清标志 + 硬件数字滤波）；**下拉仅 PF3 具备实际电阻**（V1.4 §9.6.3）
- **UART**（`uart`）：UART1/2/3 阻塞式收发，8-N-1，波特率整数+小数分频自动计算；接收路径自动清除 ORE/NE/PE/FE 错误标志（防溢出假死）
- **SPI**（`spi`，v0.3）：SPI1/2/3 阻塞式主机，8-bit、四种 CPOL/CPHA、MSB/LSB、单工单发/单收；`SCK=PCLK/(2×(N+1))` 波特自动选档
- **I2C**（`i2c`，v0.3）：I2C1/2 阻塞式主机，100k/400k 时序按规范推导（表 23-4 官方值 ±1 TCLKD 回归单测）；NACK/仲裁/总线卡死结构化错误 + 超时；`scan()` 总线扫描
- **BTIM**（`btim`，v0.3）：BTIM1-3 周期/单次定时，PSC/ARR 自动配对（ARR 最大化量化精度），更新中断联动 `runtime::Interrupt`
- **ADC**（`adc`，v0.3）：ADC1/2 单次阻塞转换；**内置温度传感器与 1.2V 基准通道**（TSEN 30µs 稳定等待、出厂校准、手册公式整数实现：`vdda_mv()` / `temperature_mc()`）；**序列扫描**（`convert_sequence`，2..=8 通道一次启动，v0.4）
- **DAC**（`dac`，v0.4）：双通道 12-bit 直写模式 + DOR 回读；内部互联 DAC_OUT2→ADC1_IN12 支持**零接线回环自测**
- **DMA**（`dma`，v0.7）：4 通道（通道号即优先级），8/16/32 位位宽，
  软件触发 + **40 种外设硬件触发全表**（`pac::dma::hs`，表 8-8）；BLOCK/BULK、
  地址自增与完成重载、自动重传（环形缓冲）；`mem_to_mem()` 一行搬存储器，
  外设流用 `Config::periph_to_mem / mem_to_periph`。
  **实测**：ADC 连续转换 → EOC 硬件触发搬 256 点，CPU 全程不参与
  （MIN/MAX/MEAN=2045/2050/2047，DAC 码 2048）；存储器 64 字搬运逐字一致
- **RTC**（`rtc`，v0.8）：BCD 日历（年月日周 + 时分秒，自动闰年、24 小时制）、
  时钟源 LSE/LSI/HSE/HSI（`Rtc::new` 自动起振并等稳定，LSE 不起振可退 LSI）、
  `DateTime` 合法性校验 + 双读稳定的 `datetime()`、两个闹钟（时/分/秒可独立
  屏蔽，按表 13-4 编码成寄存器值）、周期中断 0.5s~1 月、自动唤醒定时器、
  `KEY` 解锁/上锁自动配对。**实测**：23:59:50 起跑 12s → 次日 00:00:01、
  星期六，闹钟命中时刻与设定值一致
- **FLASH 自编程**（`flash`，v0.8）：512B 页擦除 + 32 位字编程 + `write_page`，
  `PAGELOCK` 按 8 页分组解锁/回锁，5 种 `ISR` 错误结构化返回，
  用链接期 `_etext` 把代码区挡在擦写范围外。**实测**：页 127 存计数器，
  连续两次复位后 1→2→3（掉电保持）
- **IWDT**（`wdt`，v0.4）：独立看门狗，键值机完整封装，`TimeoutMs` 自动配 PRS/ARR（向上取整永不短于请求值），RC10K 独立时钟域
- **GTIM**（`gtim`，v0.4）：GTIM1-4 PWM 输出（边沿对齐模式 1），频率 + 万分比占空比 API，CCR/ARR/PSC/CNT 全可回读
- **LPTIM**（`lptim`，v0.6）：PCLK/LSE/LSI 计数 + DIV1..128，ARRM/CMPM 中断，
  **深度休眠可唤醒**（IRQ19）；`Lptim::new(Lsi)` 自动使能 LSI 振荡器并等稳定
  （LSI 复位默认关闭，§4.5.5），`clk_ready()` 可查
- **HALLTIM**（`halltim`，v0.6）：24bit 霍尔定时器，3 路捕获（捕获沿将 CNT 存
  WIDTH 并清零）、PWM 输出、两级滤波、TRGO 触发源、`SOFTCAP` 软件捕获
- **VC 比较器**（`vc`，v0.6）：VC1–VC4，正端 4 路 / 负端 2 路 + 内部 DAC/分压，
  迟滞 20mV、极性、窗口比较、16 档滤波、高/上升沿/下降沿中断（IRQ14/15，
  深度休眠可唤醒）；`ref_voltage_mv()` 计算分压参考
- **OPA 运放**（`opa`，v0.6）：OPA1/2，外部放大 / 内部 PGA（×2..×32）/ 跟随器，
  偏置 1..8µA，电平校准与触发校准
- **延时**（`delay`）：毫秒 / 微秒 / 纳秒；**短延时走 3 周期/轮的周期忙等**
  （v0.6，避开 SysTick 启停开销），长延时 u64 分段防溢出；换算系数在
  `Delay::new` 预计算，常规频率（4/48/96MHz 等）零 64 位除法
  （实测 4MHz：100µs/1000µs/10ms 误差均 <5%）
- **模拟引脚**（`gpio`）：`Pin::into_analog()` —— ADC / VC / OPA / DAC 引脚
  必须置 `GPIOx_ANALOG.PINy=1`，否则数字输出驱动器会把模拟节点拉死（v0.6
  实机踩坑：PB00 未配模拟导致 OPA 输出被短路）
- **运行时**（`runtime`，默认 feature）：48 项中断向量表（弱符号可覆盖）、`interrupt!` 宏、NVIC 门控、PRIMASK 临界区

## 目录结构

```text
cw32l012-hal/
├── Cargo.toml          # 包定义（零依赖）
├── memory.x            # 链接脚本（向量表 + 存储器布局）
├── build.rs            # 链接脚本搜索路径
├── .cargo/config.toml  # thumbv6m 交叉编译 rustflags
├── src/
│   ├── lib.rs          # Peripherals 单例 + 模块组织
│   ├── pac.rs          # 寄存器定义层（Rw/Ro/Wo 原语 + 寄存器块 + 位常量）
│   ├── rcc.rs          # 复位与时钟控制
│   ├── gpio.rs         # GPIO（类型状态 Pin → Output/Input/AfPin）
│   ├── uart.rs         # UART 阻塞驱动
│   ├── delay.rs        # SysTick 延时
│   └── runtime.rs      # 自包含启动运行时（仅交叉编译时编译）
└── examples/
    ├── blink.rs             # LED 闪烁（PA05）
    ├── blink_pb08_pb09.rs   # 双 LED 闪烁（PB08/PB09）
    ├── uart_echo.rs         # UART1 回显（PA02/PA03，115200-8N1）
    └── tft_*.rs             # ST7789 TFT 屏幕系列例程（彩条/文字/诊断等 7 个）
```

## 快速上手

### 0. ST-LINK / SWD 4 线接法（最常见，足够烧录和调试）

如果你用的是 ST-LINK / ST-LINK V2 / 开发板自带调试头，通常 4 根杜邦线就够：

- SWDIO  -> 目标板 SWDIO / PA13 / SWDIO（按开发板丝印）
- SWCLK  -> 目标板 SWCLK / PA14 / SWCLK（按开发板丝印）
- GND    -> 目标板 GND
- 3V3    -> 目标板 VCC / 3V3

如果目标板带 `NRST`，再额外加 1 根 `NRST` 更稳定；但不接 `NRST` 也常能正常烧录和调试。

> 说明：这是一条 SWD 调试线，不是 UART 下载线。要做 UART ISP 下载，需要 TX/RX/GND，再配合 BOOT 引脚；它和 ST-LINK 不是一回事。

### 1. 安装工具链

```bash
rustup target add thumbv6m-none-eabi
```

### 2. 构建示例

```bash
cd cw32l012-hal
cargo build --target thumbv6m-none-eabi --release --example blink
cargo build --target thumbv6m-none-eabi --release --example uart_echo
```

产物位于 `target/thumbv6m-none-eabi/release/examples/`（ELF 格式）。
本仓库 `firmware/` 目录已附带预先构建好的两个示例固件
（`blink.bin/.hex`、`uart_echo.bin/.hex`，见下文），可直接烧录。

> 如需自行转换格式：`cargo install cargo-binutils` 后使用
> `cargo objcopy -- -O binary` / `-O ihex`，或使用
> `python3 tools/elf2bin.py in.elf out.bin out.hex`（随本项目附带）。

### 3. 烧录

最稳妥的方式是使用 ST-LINK 进行 SWD 烧录和调试；4 根杜邦线即可完成：

- SWDIO
- SWCLK
- GND
- 3V3

如果你使用的是官方 CW32 Programmer 或串口 ISP，则需要 UART TX/RX/GND 和 BOOT 引脚；但 ST-LINK 作为调试器通常更简单、更稳定。

CW32L012 内置 BootLoader 支持 UART ISP 下载（BOOT 引脚拉低后复位进入，
地址 `0x0010_0000` 起 2KB），也可使用官方 CW32 Programmer 工具或支持
Cortex-M 的第三方烧录器（J-Link / DAP-Link + OpenOCD 等）。

### LED 闪烁示例（examples/blink.rs）

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

    // 复位默认时钟：SysClk = HCLK = PCLK = 4MHz（HSI = HSIOSC/24）
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    let mut led = Pin::new(p.gpioa, PortId::A, 5, &mut rcc)
        .into_output(Level::Low);

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

### UART 回显示例（examples/uart_echo.rs）

```rust
let tx = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_af_output(Af::Af1); // PA02=UART1_TXD
let rx = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_af_input(Af::Af1);  // PA03=UART1_RXD
let mut uart = Uart::new(p.uart1, tx, rx, &mut rcc, clocks.pclk_hz, 115_200);
uart.write_str("hello\r\n");
loop {
    if let Some(b) = uart.try_read_byte() {
        uart.write_byte(b);
    }
}
```

### 用户程序模板（自建工程）

```rust
#![no_std]
#![no_main]

#[no_mangle]
fn main() -> ! {
    // ... 使用 cw32l012_hal ...
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}
```

无需 cortex-m-rt：`runtime` feature 已提供向量表与复位初始化；
定义中断服务程序使用：

```rust
cw32l012_hal::interrupt!("GPIOA_irq", gpioa_isr, {
    // 处理 GPIOA 中断（引脚中断需先经 pac 原始寄存器配置 RISEIE/FALLIE）
});
```

## API 速查

| 类型 / 函数 | 说明 |
|-------------|------|
| `Peripherals::take()` | 外设单例（一次成功） |
| `Rcc::new(sysctrl)` | RCC 句柄（复位默认 4MHz） |
| `rcc.set_hsi_div(HsiDiv::Div1)` | SysClk → 96MHz（注意 Flash 等待周期，见手册第 7 章） |
| `rcc.set_hclk_div/set_pclk_div` | 总线分频 |
| `rcc.clocks()` | 取当前 `Clocks { sysclk_hz, hclk_hz, pclk_hz }` |
| `rcc.enable_gpio/enable_uartN` | 外设时钟门控 |
| `Pin::new(regs, port, pin, &mut rcc)` | 创建引脚（自动开时钟） |
| `.into_output(Level) / .into_output_open_drain(Level) / .into_input(Pull) / .into_af_output(Af) / .into_af_input(Af)` | 类型状态转换 |
| `Output::{set_high, set_low, toggle, is_set_high, pin_is_high}` | 原子输出操作 / 总线电平回读 |
| `Output::{enable_open_drain, disable_open_drain}` | 推挽 ↔ 开漏运行时切换 |
| `Uart::new(regs, tx, rx, rcc, pclk_hz, baud)` | 初始化 UART（8N1） |
| `Uart::{write_byte, read_byte, try_read_byte, write_str, flush}` | 阻塞 / 非阻塞 IO（read 自动清错） |
| `Uart::{error_flags, clear_errors}` | 接收错误查询 / 清除 |
| `Delay::{delay_ms, delay_us}` | SysTick 延时 |
| `runtime::Interrupt::*` + `.enable()/.disable()` | NVIC 控制 |
| `runtime::CriticalSection::new()` | PRIMASK 临界区守卫 |
| `pac::*` | 全部寄存器原始访问（逃生舱） |

## 时钟树

```text
HSIOSC 96MHz ──/24（默认）──> HSI 4MHz ──┐
HSE 晶体 ────────────────────────────────┤
LSI ~32.8kHz ────────────────────────────┼─> SysClk ──/HCLKPRS──> HCLK ──/PCLKPRS──> PCLK
LSE 32.768kHz ───────────────────────────┘            (1..128)          (1..8)
```

变频 API（`set_hsi_div` / `set_hclk_div` / `run_hsi_at_96mhz`）已自动同步
FLASH 读等待周期（表 7-1：≤24MHz→0、≤48MHz→1、≤72MHz→2、≤96MHz→3），
并在 WAIT≠0 时使能 CACHE/FETCH（手册建议）。满速前仍需确认电压范围
支持对应主频（见数据手册）。

## 存储器与外设基地址（pac 模块常量）

| 常量 | 地址 | 说明 |
|------|------|------|
| `FLASH_BASE` | `0x0000_0000` | 主 Flash 64KB |
| `SRAM_BASE` | `0x2000_0000` | SRAM 8KB |
| `BOOT_BASE` | `0x0010_0000` | BootLoader 2KB（UART ISP） |
| `SYSCTRL_BASE` | `0x4000_4000` | 系统控制（RCC） |
| `UART1/2/3_BASE` | `0x4000_0C00/1000/2000` | UART（APB1） |
| `GPIOA/B/C/F_BASE` | `0x4800_0000/0100/0200/0300` | GPIO（AHB） |
| SPI1/2/3、I2C1/2、ATIM、GTIM1-4、BTIM123、LPTIM、HALLTIM、ADC12、RTC、IWDT、WWDT、DMA、FLASHCTRL、RAMCTRL、CRC、EAU、CORDIC | 见 `src/pac.rs` | 全部外设 |

## 中断向量表（runtime::Interrupt）

| IRQ | 名称 | IRQ | 名称 |
|-----|------|-----|------|
| 0 | WDT | 16 | GTIM1 |
| 1 | LVD | 17 | GTIM2 |
| 2 | RTC | 18 | GTIM3/4 |
| 3 | FLASHRAM | 19 | LPTIM |
| 4 | RCC | 20 | BTIM1 |
| 5 | GPIOA | 21 | BTIM2 |
| 6 | GPIOB | 22 | BTIM3/HALLTIM |
| 7 | GPIOC | 23 | I2C1 |
| 8 | GPIOF | 24 | I2C2 |
| 9 | DMA1/2 | 25 | SPI1 |
| 10 | DMA3/4 | 26 | SPI2/3 |
| 11 | CORDIC | 27 | UART1 |
| 12 | ADC1 | 28 | UART2 |
| 13 | ATIM | 29 | UART3 |
| 14 | VC1/3 | 30 | ADC2/DAC |
| 15 | VC2/4 | 31 | FAULT |

## 与官方 cw32l012_pac 的交叉验证

本 crate 自带 `pac` 模块（不依赖外部 PAC）；v0.2.2 已与
[crates.io 上的 cw32l012_pac 0.1.1](https://crates.io/crates/cw32l012_pac)
（svd2rust v0.37.1 生成）逐项对照：

- **全部一致**：25 个外设基地址、GPIO 寄存器偏移（含 AFRH@0x14 /
  AFRL@0x18 前后关系、BRR@0x58、BSRR@0x5C、TOG@0x60）、SYSCTRL /
  UART / FLASH 位段、HSI（TRIM[10:0] / DIV[11:14] / STABLE[15]）、
  UART CR1（TXEN=0 / RXEN=1）、FLASH_CR2（WAIT[2:0] / FETCH=3 /
  CACHE=4）
- **本 crate 独有修正**（官方 PAC 的 SVD 缺陷，已按用户手册裁决）：
  - `GPIOx_PDR @ 0x0C`（官方 PAC 漏建模，0x08-0x0F 全标 reserved）。
    v0.2.2 曾建模、v0.2.3 按 SVD 删除，**v0.2.4 依据 UM V1.4 §9.6.3 重新
    建模并落地**：PDR 真实存在，但硬件上**仅 PF3 引脚具备实际下拉电阻**
    （复位值 0x0000_0008 = PF3 默认下拉使能）；`Pull::Down` 仅在 PF3 生效，
    其余引脚按浮空处理
  - `IRQ31 = FAULT @ 0xBC`（HSE/LSE 运行中失效中断；官方 PAC 的
    `__INTERRUPTS` 数组仅 31 项，漏最后一项）
  - 官方 PAC 建模的 `SYSCTRL.PLL @ 0x28` 为 CW32F 系列 SVD 残留——
    CW32L012 手册时钟树无 PLL，本 crate 按 reserved 处理
- **命名差异**：IRQ4 官方 PAC 称 `SYSCTRL`，本 crate 按手册表 5-1
  称 `RCC`（同一中断源；手册注：时钟起振失败/稳定对应 RCC 全局中断）

## 验证状态与已知限制

**已验证（实板，2026-10-17/18/19，CW32L012 + ST-LINK + probe-rs；全部流程与判定标准见 [TESTING.md](TESTING.md) 测试备忘录）**：
- **GPIO 全路径**：AHBEN KEY 门控 → ANALOG 清零 → DIR → BSRR/TOG；
  PC13 500ms 方波实测，寄存器回读与手册一致
- **SPI + 96MHz 时钟变频**：ST7789 @ SPI 24MHz，复位后 <3s 完成 init +
  清屏 + 全屏彩条（`examples/tft_hw_spi.rs`）；软 SPI 时代需 ~90s
- **BTIM 中断**：2Hz 更新中断翻转 PC13，`wfi` 睡眠硬件唤醒
  （`examples/timer_blink.rs`）
- **ADC 内部通道**：VDDA=3282mV（1.2V 基准 + 出厂校准反推）、
  温度 26.9℃（手册公式 + 出厂 Trim/T0）（`examples/adc_internal.rs`）
- **GTIM PWM**：1kHz 呼吸占空比，ARR/PSC/CNT 回读精确（v0.4）
- **DAC→ADC 片上回环**：DAC_OUT2→IN12，15 轮扫描最大误差 **3 LSB**（v0.4）
- **IWDT 复位**：停喂 ~1.5s 后真实复位循环（BOOT_ID 观测）（v0.4）
- **ATIM 互补 PWM + 死区 + 刹车**：2kHz/30%/250µs，BDTR=0x94FF 回读精确；
  刹车输入必须 AF 复用输入（踩坑记录在 TESTING.md）（v0.5）
- **WWDT 窗口语义**：过早喂狗 51 次全部拦截、窗口内喂狗 50 次全部成功、
  零复位（v0.5）
- **低功耗**：Sleep×10（BTIM 唤醒）+ DeepSleep×6（IWDT 唤醒）+
  WAKEUPCLK 唤醒后 SYSCLK=HSI（v0.5）
- 46 项主机侧单元测试（波特/时序/周期/校准/死区/窗口公式，含手册官方数值回归）

**寄存器级（编译期+文档级）**：
- 偏移 / 位段 / 复位值逐项核对《CW32L012 用户手册》V1.4（2026-06）；
  关键项于 2026-10-17 与 V1.4 原文逐行复核
- 寄存器块编译期布局静态断言（Gpio 0x64 / Uart 0x40 / Spi 0x20 /
  I2c 0x74 / Btim 0x30 / Adc 0x7C）
- 向量表二进制复核：MSP 初值 = `0x20002000`（SRAM 顶端），48 项向量就位

**未验证（上板前请注意）**：
- UART 收发数据路径、`delay_us` 低频校准忙循环精度（示波器量测）
- I2C 主机时序参数（公式已对齐表 23-4，但板上有真实从机器件前
  建议先用 `scan()` 验证 ACK 行为）
- ADC 外部引脚通道（内部通道已验证；外部通道走同一转换路径，风险低）
- 工厂 TRIM 校准自动加载假设校准区已编程；异常值自动跳过
- 序列扫描 + DMA 的 `REPEAT>1` 组合（`CNT.REPEAT` 每轮传输后被硬件清 0，
  多通道序列需每轮重配；单通道 EOC 触发已实机验证）
- TS 温度传感器（本板个体异常：读数 ≈0，厂商例程复刻同现象；驱动已加
  SensorDead 失效保护，BGR/DAC-IN12 同路径正常——详见 TESTING.md）
- LPTIM/HALLTIM/VC/OPA 已实现（v0.6），实机验证待跑
  （`lptim_deepsleep` / `halltim_capture` / `analog_loop`，见 TESTING.md）

## 路线图

- [x] GPIO 输出路径实板验证（v0.2.4：PC13 闪烁 + 寄存器回读）
- [x] SPI 阻塞驱动（v0.3：实板 ST7789 @24MHz，<3s 全屏绘制）
- [x] I2C 阻塞驱动（v0.3：时序回归表 23-4，待上板接从机器件）
- [x] BTIM 定时中断（v0.3：实板 2Hz 中断闪灯，wfi 唤醒）
- [x] ADC 单次转换 + 内部温度/基准通道（v0.3：实板 VDDA/温度）
- [x] GPIO 外部中断高层 API（v0.3：沿选择 + 数字滤波）
- [x] embedded-hal 1.0 trait 实现（v0.3：默认 feature，可关回零依赖）
- [x] FLASH 等待周期自动配置（v0.2：>24MHz 时钟安全切换，§4.5.2 流程）
- [x] UART 溢出错误自愈 + 开漏输出（v0.2）
- [x] GPIOx_PDR 下拉建模（v0.2.4：UM V1.4 §9.6.3，仅 PF3 实效）
- [x] I2C 上板验证（scan + 真实从机器件读写）
- [x] DAC 双通道直写 + ADC 内部回环（v0.4：dac_adc_loop 零接线自测）
- [x] IWDT 看门狗（v0.4：键值机 + 超时自动配对，停喂复位实机验证待跑）
- [x] GTIM PWM 输出（v0.4：4 实例 × 4 通道，回读可验证）
- [x] ADC 序列扫描（v0.4：ENS 2..=8 通道 + 独立采样周期）
- [x] ATIM 高级定时器（v0.5：死区 + 刹车 + 互补，实机验证）
- [ ] UART 收发上板验证（uart_echo 对夹）
- [x] IWDT 停喂复位实机确认（v0.5：BOOT_ID 复位循环观测）
- [x] DAC 回环精度实机确认（v0.5：MAX_ERR=3 LSB）
- [x] 低功耗模式（v0.5：Sleep/DeepSleep，实机双阶段唤醒验证）
- [x] WWDT 窗口看门狗（v0.5：窗口自动求解 + 喂狗保护，实机验证）
- [x] LPTIM 低功耗定时器（v0.6：LSI/LSE/PCLK + 深度休眠唤醒）
- [x] HALLTIM 霍尔接口定时器（v0.6：3 路捕获 + PWM + 软件捕获）
- [x] VC 模拟比较器 ×4（v0.6：迟滞/窗口/滤波/中断）
- [x] OPA 运算放大器 ×2（v0.6：PGA / 跟随器 / 校准）
- [x] DMA 控制器（v0.7：mem→mem + ADC 硬件触发，实机 256 点零 CPU）
- [x] RTC 实时时钟（v0.8：跨日走时 + 闹钟，实机验证）
- [x] FLASH 自编程（v0.8：页擦除/编程 + 掉电保持计数）
- [ ] cargo-binutils 打包 .bin/.hex 产物脚本

## 数据来源

- 《CW32L012 用户手册》V1.4，武汉芯源半导体，2026-06（寄存器定义的唯一依据）
- 《CW32L012 数据手册》（电气特性与封装引脚）

## 许可

MIT OR Apache-2.0（见 LICENSE）。
