//! # cw32l012-hal
//!
//! 武汉芯源半导体（CMSemicon）**CW32L012** 的 Rust `no_std` 硬件抽象层。
//!
//! 芯片概况：ARM Cortex-M0+ 内核（内部高频振荡器 HSIOSC 标称 96MHz）、
//! 64KB Flash、8KB SRAM（带奇偶校验）、UART ×3、SPI ×3、I2C ×2、
//! ATIM/GTIM/BTIM/LPTIM 定时器等外设的低功耗微控制器。
//!
//! 本 crate **零外部依赖、完全自包含**：
//!
//! | 模块 | 说明 |
//! |------|------|
//! | [`pac`] | 寄存器定义层（SYSCTRL / GPIO / UART + 全外设基地址表） |
//! | [`rcc`] | 时钟控制：HSI 分频、SysClk 源切换、HCLK/PCLK 分频、外设时钟门控 |
//! | [`gpio`] | GPIO：推挽输出 / 输入（上拉、下拉）/ 复用功能，原子置位/清零/翻转 |
//! | [`uart`] | UART1/2/3 阻塞式驱动（8-N-1，波特率自动计算） |
//! | [`delay`] | SysTick 忙等延时（毫秒 / 微秒） |
//! | [`runtime`] | （默认 feature）自包含启动运行时：中断向量表 + 复位初始化 |
//!
//! ## 最小示例
//!
//! ```ignore
//! #![no_std]
// 寄存器文档大量使用 `CR1[7:0]`、`SQRCH0[3:0]` 这类位域写法，rustdoc 会把
// 方括号当成 intra-doc 链接去解析，产生几百条"链接无法解析"的噪声警告。
// 本 crate 的文档以手册位域表为主，统一关掉这类警告（不影响 --show-type-layout）。
#![allow(rustdoc::broken_intra_doc_links)]
//! #![no_main]
//!
//! use cw32l012_hal::{gpio::{Level, Pin, PortId}, rcc::Rcc, Peripherals};
//!
//! #[no_mangle]
//! fn main() -> ! {
//!     let p = Peripherals::take().unwrap();
//!     let mut rcc = Rcc::new(p.sysctrl, p.flashctrl); // 复位默认：SysClk=HCLK=PCLK=4MHz
//!     let mut led = Pin::new(p.gpioa, PortId::A, 5, &mut rcc)
//!         .into_output(Level::Low);
//!     loop {
//!         led.toggle();
//!         // TODO: delay
//!     }
//! }
//!
//! #[panic_handler]
//! fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
//! ```
//!
//! ## 数据来源与验证状态
//!
//! 寄存器偏移、位段与复位值转录自《CW32L012 用户手册》V1.4
//! （武汉芯源半导体，2026-06；V1.4 相对 V1.2 新增 GPIOx_PDR 下拉寄存器，
//! 仅 PF3 引脚具备实际下拉）。
//!
//! - 已通过 `thumbv6m-none-eabi` 交叉编译验证（debug + release 零警告）；
//! - **GPIO 输出路径已于 2026-10-17 在 CW32L012 实板验证**（PC13 闪烁：
//!   AHBEN KEY 门控 → ANALOG 清零 → DIR 配置 → BSRR/TOG，寄存器回读
//!   全部符合手册）；
//! - 其余模块（UART / SysTick 延时 / 时钟变频）尚未全量上板；
//!   量产前请对照手册复核关键寄存器位。

#![no_std]

pub mod adc;
pub mod atim;
pub mod btim;
pub mod dac;
pub mod delay;
pub mod dma;
#[cfg(feature = "ehal-1")]
pub mod ehal;
pub mod flash;
pub mod gpio;
pub mod gtim;
pub mod halltim;
pub mod i2c;
pub mod lptim;
pub mod opa;
pub mod pac;
pub mod pwr;
pub mod rcc;
pub mod rtc;
pub mod spi;
pub mod uart;
pub mod vc;
pub mod wdt;
pub mod wwdt;

#[cfg(all(feature = "runtime", target_arch = "arm", target_os = "none"))]
pub mod runtime;

/// 芯片外设单例。[`Peripherals::take`] 只能成功一次，保证外设所有权唯一。
pub struct Peripherals {
    /// 系统控制（复位与时钟）
    pub sysctrl: &'static pac::Sysctrl,
    /// FLASH 控制器（读等待周期 / 缓存 / 擦写控制；交给 [`rcc::Rcc`]）
    pub flashctrl: &'static pac::Flashctrl,
    /// GPIO 端口 A（PA0..PA8，视封装）
    pub gpioa: &'static pac::Gpio,
    /// GPIO 端口 B（PB0..PB7，视封装）
    pub gpiob: &'static pac::Gpio,
    /// GPIO 端口 C（PC13..PC15，视封装）
    pub gpioc: &'static pac::Gpio,
    /// GPIO 端口 F（PF0/PF1/PF3/PF6/PF7，视封装）
    pub gpiof: &'static pac::Gpio,
    /// UART1（PA02=TXD, PA03=RXD，AF1；另有 PA09/PA10 组合）
    pub uart1: &'static pac::Uart,
    /// UART2（PA02=TXD, PA03=RXD，AF2；另有 PA07/PA06、PB00/PB01 组合）
    pub uart2: &'static pac::Uart,
    /// UART3（PA04=TXD/PA05=RXD AF1，或 PA01=TXD/PA00=RXD AF2；另有 PA08/09、PA13/14、PB02/PB05 组合）
    pub uart3: &'static pac::Uart,
    /// SPI1（SCK=PA05/PA10、MOSI=PA07/PA12、MISO=PA06/PA11，均 AF3）
    pub spi1: &'static pac::Spi,
    /// SPI2（PB03/PB04/PB05，视封装可用性）
    pub spi2: &'static pac::Spi,
    /// SPI3（视封装可用性）
    pub spi3: &'static pac::Spi,
    /// I2C1（SCL=PA04/PA09、SDA=PA05/PA10，均 AF2，需开漏）
    pub i2c1: &'static pac::I2c,
    /// I2C2（视封装可用性）
    pub i2c2: &'static pac::I2c,
    /// BTIM1 基础定时器
    pub btim1: &'static pac::Btim,
    /// BTIM2 基础定时器
    pub btim2: &'static pac::Btim,
    /// BTIM3 基础定时器
    pub btim3: &'static pac::Btim,
    /// ADC1（内部通道：温度传感器 IN14、1.2V 基准 IN15）
    pub adc1: &'static pac::Adc,
    /// ADC2（仅外部通道，映射与 ADC1 不同，见表 25-4）
    pub adc2: &'static pac::Adc,
    /// DAC 双通道（CH2 内部互联 ADC1_IN12，可无接线回环自测）
    pub dac: &'static pac::Dac,
    /// IWDT 独立看门狗（RC10K 独立时钟域）
    pub iwdt: &'static pac::Iwdt,
    /// LPTIM 低功耗定时器（LSI/LSE 域，DeepSleep 可唤醒，IRQ19）
    pub lptim: &'static pac::Lptim,
    /// HALLTIM 霍尔接口定时器（3 路捕获 + PWM，IRQ22 与 BTIM3 共享）
    pub halltim: &'static pac::Halltim,
    /// VC1 模拟电压比较器（与 VC2 共用 vc12ref 分压源，IRQ14）
    pub vc1: &'static pac::Vc,
    /// VC2 模拟电压比较器（IRQ15）
    pub vc2: &'static pac::Vc,
    /// VC3 模拟电压比较器（与 VC4 共用 vc34ref，IRQ14）
    pub vc3: &'static pac::Vc,
    /// VC4 模拟电压比较器（IRQ15）
    pub vc4: &'static pac::Vc,
    /// VC1/VC2 公共电阻分压参考源
    pub vc12ref: &'static pac::VcRef,
    /// VC3/VC4 公共电阻分压参考源
    pub vc34ref: &'static pac::VcRef,
    /// OPA1 运算放大器（正端 INP4 = DAC_OUT1）
    pub opa1: &'static pac::Opa,
    /// OPA2 运算放大器（正端 INP4 = DAC_OUT2）
    pub opa2: &'static pac::Opa,
    /// WWDT 窗口看门狗（PCLK 时钟域，窗口内喂狗）
    pub wwdt: &'static pac::Wwdt,
    /// ATIM 高级定时器（互补 PWM + 死区 + 刹车）
    pub atim: &'static pac::Atim,
    /// GTIM1 通用定时器（PWM ×4 通道）
    pub gtim1: &'static pac::Gtim,
    /// GTIM2 通用定时器
    pub gtim2: &'static pac::Gtim,
    /// GTIM3 通用定时器
    pub gtim3: &'static pac::Gtim,
    /// GTIM4 通用定时器
    pub gtim4: &'static pac::Gtim,
    /// DMA 控制器（4 通道，AHB 总线，§8）
    pub dma: &'static pac::Dma,
    /// RTC 实时时钟（BCD 日历 + 闹钟 + 唤醒定时器，§13）
    pub rtc: &'static pac::Rtc,
}

impl Peripherals {
    /// 获取外设单例；第二次及以后调用返回 `None`。
    ///
    /// 注：Cortex-M0+ 不支持核心库原子交换指令，此处在单核裸机语义下
    /// 使用 `static mut` 实现一次性标记（请在开启中断前完成首次调用）。
    pub fn take() -> Option<Self> {
        static mut TAKEN: bool = false;
        // SAFETY: 单核裸机环境；约定首次调用发生在使能中断之前。
        unsafe {
            if TAKEN {
                None
            } else {
                TAKEN = true;
                Some(Self::steal())
            }
        }
    }

    /// 无视单例直接取得外设引用（例如中断上下文中再次取用）。
    ///
    /// # Safety
    /// 调用方需自行保证不出现别名所有权冲突（例如重复初始化同一外设）。
    pub unsafe fn steal() -> Self {
        Peripherals {
            sysctrl: &*(pac::SYSCTRL_BASE as *const pac::Sysctrl),
            flashctrl: &*(pac::FLASHCTRL_BASE as *const pac::Flashctrl),
            gpioa: &*(pac::GPIOA_BASE as *const pac::Gpio),
            gpiob: &*(pac::GPIOB_BASE as *const pac::Gpio),
            gpioc: &*(pac::GPIOC_BASE as *const pac::Gpio),
            gpiof: &*(pac::GPIOF_BASE as *const pac::Gpio),
            uart1: &*(pac::UART1_BASE as *const pac::Uart),
            uart2: &*(pac::UART2_BASE as *const pac::Uart),
            uart3: &*(pac::UART3_BASE as *const pac::Uart),
            spi1: &*(pac::SPI1_BASE as *const pac::Spi),
            spi2: &*(pac::SPI2_BASE as *const pac::Spi),
            spi3: &*(pac::SPI3_BASE as *const pac::Spi),
            i2c1: &*(pac::I2C1_BASE as *const pac::I2c),
            i2c2: &*(pac::I2C2_BASE as *const pac::I2c),
            btim1: &*(pac::BTIM1_BASE as *const pac::Btim),
            btim2: &*(pac::BTIM2_BASE as *const pac::Btim),
            btim3: &*(pac::BTIM3_BASE as *const pac::Btim),
            adc1: &*(pac::ADC1_BASE as *const pac::Adc),
            adc2: &*(pac::ADC2_BASE as *const pac::Adc),
            dac: &*(pac::DAC_BASE as *const pac::Dac),
            iwdt: &*(pac::IWDT_BASE as *const pac::Iwdt),
            lptim: &*(pac::LPTIM_BASE as *const pac::Lptim),
            halltim: &*(pac::HALLTIM_BASE as *const pac::Halltim),
            vc1: &*(pac::VC1_BASE as *const pac::Vc),
            vc2: &*(pac::VC2_BASE as *const pac::Vc),
            vc3: &*(pac::VC3_BASE as *const pac::Vc),
            vc4: &*(pac::VC4_BASE as *const pac::Vc),
            vc12ref: &*(pac::VC12REF_BASE as *const pac::VcRef),
            vc34ref: &*(pac::VC34REF_BASE as *const pac::VcRef),
            opa1: &*(pac::OPA1_BASE as *const pac::Opa),
            opa2: &*(pac::OPA2_BASE as *const pac::Opa),
            wwdt: &*(pac::WWDT_BASE as *const pac::Wwdt),
            atim: &*(pac::ATIM_BASE as *const pac::Atim),
            gtim1: &*(pac::GTIM1_BASE as *const pac::Gtim),
            gtim2: &*(pac::GTIM2_BASE as *const pac::Gtim),
            gtim3: &*(pac::GTIM3_BASE as *const pac::Gtim),
            gtim4: &*(pac::GTIM4_BASE as *const pac::Gtim),
            dma: &*(pac::DMA_BASE as *const pac::Dma),
            rtc: &*(pac::RTC_BASE as *const pac::Rtc),
        }
    }
}
