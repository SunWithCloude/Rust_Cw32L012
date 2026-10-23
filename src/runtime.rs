//! 自包含 Cortex-M0+ 运行时（feature `runtime`，仅交叉编译时编译）
//!
//! 提供：
//! - 中断向量表（`memory.x` 中生成，弱符号默认指向 `DefaultHandler`）
//! - 复位入口：`.data` 段复制、`.bss` 段清零后跳转用户 `main`
//! - [`Interrupt`] 中断号枚举与 NVIC 操作
//! - [`interrupt!`] 宏：定义可被向量表引用的中断服务程序
//! - [`CriticalSection`]：PRIMASK 临界区
//!
//! 用户程序约定：
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! #[no_mangle]
//! fn main() -> ! { /* ... */ }
//!
//! #[panic_handler]
//! fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
//! ```

// ---------------------------------------------------------------------------
// 复位入口与默认中断处理
// ---------------------------------------------------------------------------

/// 默认中断处理符号（`memory.x` 中 DefaultHandler 指向此函数）
#[no_mangle]
extern "C" fn _DefaultHandler_default() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// 复位向量（`.isr_vector.reset` 段，由链接脚本 KEEP）
#[link_section = ".isr_vector.reset"]
#[used]
static RESET_VECTOR: fn() -> ! = Reset;

/// 复位入口（导出符号 `Reset`，供链接脚本 `ENTRY` 引用）：
/// 初始化 `.data`/`.bss` 后调用用户 `main`
#[no_mangle]
#[allow(non_snake_case)]
fn Reset() -> ! {
    extern "C" {
        static _sidata: u32;
        static mut _sdata: u32;
        static mut _edata: u32;
        static mut _sbss: u32;
        static mut _ebss: u32;
    }

    unsafe {
        // 复制 .data（Flash → SRAM）
        let mut src = core::ptr::addr_of!(_sidata);
        let mut dst = core::ptr::addr_of_mut!(_sdata);
        let end = core::ptr::addr_of_mut!(_edata);
        while dst < end {
            dst.write(src.read());
            src = src.add(1);
            dst = dst.add(1);
        }

        // 清零 .bss
        let mut bss = core::ptr::addr_of_mut!(_sbss);
        let bss_end = core::ptr::addr_of_mut!(_ebss);
        while bss < bss_end {
            bss.write(0);
            bss = bss.add(1);
        }
    }

    extern "Rust" {
        fn main() -> !;
    }
    unsafe { main() }
}

// ---------------------------------------------------------------------------
// 中断号与 NVIC
// ---------------------------------------------------------------------------

/// CW32L012 外部中断号（手册表 5-1，IRQ0..31）
///
/// 符号名与 `memory.x` 中 `XXX_irq` 一一对应，
/// 可用 [`interrupt!`] 宏定义服务程序。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    /// IRQ0：窗口/独立看门狗（符号 `WDT_irq`）
    Wdt = 0,
    /// IRQ1：低压检测（`LVD_irq`）
    Lvd = 1,
    /// IRQ2：RTC（`RTC_irq`）
    Rtc = 2,
    /// IRQ3：FLASH/RAM（`FLASHRAM_irq`）
    FlashRam = 3,
    /// IRQ4：RCC（`RCC_irq`）
    Rcc = 4,
    /// IRQ5：GPIOA（`GPIOA_irq`）
    GpioA = 5,
    /// IRQ6：GPIOB（`GPIOB_irq`）
    GpioB = 6,
    /// IRQ7：GPIOC（`GPIOC_irq`）
    GpioC = 7,
    /// IRQ8：GPIOF（`GPIOF_irq`）
    GpioF = 8,
    /// IRQ9：DMA 通道 1/2（`DMA12_irq`）
    Dma12 = 9,
    /// IRQ10：DMA 通道 3/4（`DMA34_irq`）
    Dma34 = 10,
    /// IRQ11：CORDIC（`CORDIC_irq`）
    Cordic = 11,
    /// IRQ12：ADC1（`ADC1_irq`）
    Adc1 = 12,
    /// IRQ13：ATIM（`ATIM_irq`）
    Atim = 13,
    /// IRQ14：VC1/3（`VC13_irq`）
    Vc13 = 14,
    /// IRQ15：VC2/4（`VC24_irq`）
    Vc24 = 15,
    /// IRQ16：GTIM1（`GTIM1_irq`）
    Gtim1 = 16,
    /// IRQ17：GTIM2（`GTIM2_irq`）
    Gtim2 = 17,
    /// IRQ18：GTIM3/4（`GTIM34_irq`）
    Gtim34 = 18,
    /// IRQ19：LPTIM（`LPTIM_irq`）
    Lptim = 19,
    /// IRQ20：BTIM1（`BTIM1_irq`）
    Btim1 = 20,
    /// IRQ21：BTIM2（`BTIM2_irq`）
    Btim2 = 21,
    /// IRQ22：BTIM3/HALLTIM（`BTIM3_HALLTIM_irq`）
    Btim3Halltim = 22,
    /// IRQ23：I2C1（`I2C1_irq`）
    I2c1 = 23,
    /// IRQ24：I2C2（`I2C2_irq`）
    I2c2 = 24,
    /// IRQ25：SPI1（`SPI1_irq`）
    Spi1 = 25,
    /// IRQ26：SPI2/3（`SPI23_irq`）
    Spi23 = 26,
    /// IRQ27：UART1（`UART1_irq`）
    Uart1 = 27,
    /// IRQ28：UART2（`UART2_irq`）
    Uart2 = 28,
    /// IRQ29：UART3（`UART3_irq`）
    Uart3 = 29,
    /// IRQ30：ADC2/DAC（`ADC2_DAC_irq`）
    Adc2Dac = 30,
    /// IRQ31：FAULT（`FAULT_irq`）
    Fault = 31,
}

impl Interrupt {
    /// 在 NVIC 中使能该中断（ISER 写 1 置位）
    pub fn enable(self) {
        let irq = self as u32;
        unsafe {
            core::ptr::write_volatile(0xE000_E100 as *mut u32, 1 << irq);
        }
    }

    /// 在 NVIC 中禁用该中断（ICER 写 1 清除）
    pub fn disable(self) {
        let irq = self as u32;
        unsafe {
            core::ptr::write_volatile(0xE000_E180 as *mut u32, 1 << irq);
        }
    }

    /// 将该中断置为挂起（ISPR）
    pub fn pend(self) {
        let irq = self as u32;
        unsafe {
            core::ptr::write_volatile(0xE000_E200 as *mut u32, 1 << irq);
        }
    }

    /// 清除挂起状态（ICPR）
    pub fn unpend(self) {
        let irq = self as u32;
        unsafe {
            core::ptr::write_volatile(0xE000_E280 as *mut u32, 1 << irq);
        }
    }
}

// ---------------------------------------------------------------------------
// interrupt! 宏
// ---------------------------------------------------------------------------

/// 定义中断服务程序。
///
/// 第一个参数必须是与 [`Interrupt`] 枚举对应文档中列出的向量符号名
/// （如 `"UART1_irq"`），第二个参数为函数名（需在当前作用域唯一）。
///
/// ```ignore
/// cw32l012_hal::interrupt!("GPIOA_irq", gpioa_isr, {
///     // 处理 GPIOA 中断...
/// });
/// ```
#[macro_export]
macro_rules! interrupt {
    ($name:literal, $func:ident, $body:block) => {
        #[export_name = $name]
        extern "C" fn $func() $body
    };
}

// ---------------------------------------------------------------------------
// 临界区（PRIMASK）
// ---------------------------------------------------------------------------

/// 临界区守卫：构造时关中断（`cpsid i`），析构时按需恢复。
///
/// ```ignore
/// {
///     let _cs = cw32l012_hal::runtime::CriticalSection::new();
///     // 关键操作...
/// } // 自动恢复中断
/// ```
pub struct CriticalSection {
    primask: u32,
}

impl CriticalSection {
    /// 进入临界区（关闭可屏蔽中断）
    pub fn new() -> Self {
        let primask: u32;
        unsafe {
            core::arch::asm!(
                "mrs {0}, PRIMASK",
                "cpsid i",
                out(reg) primask,
            );
        }
        CriticalSection { primask }
    }
}

impl Default for CriticalSection {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CriticalSection {
    fn drop(&mut self) {
        // 仅在进入前中断开启时才重新开启
        if self.primask & 1 == 0 {
            unsafe {
                core::arch::asm!("cpsie i");
            }
        }
    }
}
