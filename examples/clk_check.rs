//! 时钟与延时精度实测（v0.6 验证例）
//!
//! 三条互相独立的量，用来定位"延时不准"的根源：
//! - `DELAY_TICKS`：`delay_ms(1000)` 期间 BTIM1(1kHz) 走过的节拍数；
//!   理想 1000，偏小说明 SysTick 延时偏短。
//! - `US_TICKS`：1000 × `delay_us(1000)` 期间的节拍数；理想 1000，
//!   与 `DELAY_TICKS` 对比可区分毫秒/微秒两条路径。
//! - `SECS` / `TICKS`：每秒自增（SysTick）与 1kHz 硬件中断计数，
//!   用外部墙钟对照可反推 PCLK/HSI 实际频率。

#![no_std]
#![no_main]

use cw32l012_hal::{
    btim::Btim,
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

/// `delay_ms(1000)` 期间 BTIM 1kHz 节拍数（理想 1000）
static mut DELAY_TICKS: u32 = 0;
/// 1000 次 `delay_us(1000)` 期间 BTIM 节拍数（理想 1000）
static mut US_TICKS: u32 = 0;
/// 5000 次 `delay_us(100)` 期间节拍数（理想 500）
static mut US100_TICKS: u32 = 0;
/// 500 次 `delay_us(10000)` 期间节拍数（理想 5000）
static mut US10000_TICKS: u32 = 0;
/// `delay_ms(1000)` 次数
static mut SECS: u32 = 0;
/// BTIM1 1kHz 中断累计次数（PCLK 基准）
static mut TICKS: u32 = 0;

#[no_mangle]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();
    let mut delay = Delay::new(&clocks);
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // BTIM1 @ 1kHz（PCLK 直接分频，硬件基准）
    let mut btim = Btim::new(p.btim1, &mut rcc);
    btim.start_periodic(clocks.pclk_hz, 1000).ok();
    btim.enable_update_irq(true);
    unsafe {
        cw32l012_hal::runtime::Interrupt::Btim1.enable();
        core::arch::asm!("cpsie i");
    }
    delay.delay_ms(10); // 等定时器起跑

    // ---- 单次测量：延时 1s，看硬件走了多少节拍 ----
    let a = unsafe { TICKS };
    delay.delay_ms(1000);
    let b = unsafe { TICKS };
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(DELAY_TICKS), b - a) };

    let a = unsafe { TICKS };
    for _ in 0..1000 {
        delay.delay_us(1000);
    }
    let b = unsafe { TICKS };
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(US_TICKS), b - a) };

    // 不同时长对比：区分"每次调用固定开销"与"比例误差"
    let a = unsafe { TICKS }; // 5000 × 100µs = 0.5s
    for _ in 0..5000 {
        delay.delay_us(100);
    }
    let b = unsafe { TICKS };
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(US100_TICKS), b - a) };

    let a = unsafe { TICKS }; // 500 × 10ms = 5s
    for _ in 0..500 {
        delay.delay_us(10_000);
    }
    let b = unsafe { TICKS };
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(US10000_TICKS), b - a) };

    loop {
        delay.delay_ms(1000);
        led.toggle();
        // SAFETY: main 独占（ISR 只写 TICKS）
        unsafe {
            let v = core::ptr::read_volatile(core::ptr::addr_of!(SECS));
            core::ptr::write_volatile(core::ptr::addr_of_mut!(SECS), v + 1);
        }
    }
}

// BTIM1 更新中断：清标志 + 计数
cw32l012_hal::interrupt!("BTIM1_irq", btim1_isr, {
    unsafe {
        let p = Peripherals::steal();
        p.btim1.icr.write(cw32l012_hal::pac::btim::ICR_CLEAR_UIF);
        let v = core::ptr::read_volatile(core::ptr::addr_of!(TICKS));
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TICKS), v + 1);
    }
});

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
