//! 延时精度实测（v0.6 验证例）：SysTick 延时 vs 外部墙钟
//!
//! 固件每秒（`delay_ms(1000)`）自增一次 `SECS` 并翻转 PC13。
//! 用外部计时（shell 的 `date`/`sleep`）对照即可算出误差：
//!
//! ```text
//! t0=$(date +%s%N); probe-rs reset ...; sleep 20
//! # 读 SECS（nm 定位符号），t1=$(date +%s%N)
//! # 误差 = SECS / ((t1-t0)/1e9) - 1
//! ```
//!
//! 说明：`Rcc::new` 后默认 SysClk = HSI = 4MHz，SysTick 重装载值按
//! `hclk_hz` 计算；本例用来量化"1 秒到底有多长"。

#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

/// 已完成的 1 秒延时次数（对外部墙钟）
static mut SECS: u32 = 0;

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

    loop {
        delay.delay_ms(1000);
        led.toggle();
        // SAFETY: main 独占该静态变量（单核、无 ISR 访问）
        unsafe {
            SECS += 1;
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
