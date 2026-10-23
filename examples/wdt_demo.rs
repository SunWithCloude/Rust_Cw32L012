//! IWDT 看门狗演示（v0.4 新增 `wdt` 模块验证例）
//!
//! 行为设计成**可观测的复位循环**：
//! 1. IWDT 超时 ≈ 1.5s（RC10K ÷4、ARR 按公式自动取）
//! 2. `FEEDS` 计数喂狗（PC13 每次喂狗翻转，肉眼可见快闪 5 次）
//! 3. 喂满 5 次后**故意停喂** → 1.5s 后看门狗复位 → 重启
//! 4. `BOOT_ID` 记录本次开机以来喂狗次数；复位后清零重新爬升
//!
//! 预期观测（调试器内存窗口）：
//! - `BOOT_ID` 从 0 爬到 5 → 停喂 → 芯片复位 → 回到 0 再爬
//! - 周期 ≈ 5×300ms + 1.5s ≈ 3s 一轮，PC13 快闪 5 下、灭 1.5s
//!
//! 这就是"看门狗真的会咬人"的硬证据。

#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    wdt::{Iwdt, TimeoutMs},
    Peripherals,
};

/// 本次开机喂狗次数（看门狗复位后归零重新计数）
static mut BOOT_ID: u32 = 0;

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
    let mut wdt = Iwdt::new(p.iwdt, &mut rcc);

    // 1.5s 超时（RC10K 有 ±% 级漂移，喂狗间隔留足裕量）
    match wdt.start(TimeoutMs(1500)) {
        Ok(_) => {}
        Err(_) => loop {
            core::hint::spin_loop();
        },
    }

    loop {
        unsafe {
            BOOT_ID += 1;
            // 只喂前 5 次；之后故意停喂 → 1.5s 后看门狗复位
            if BOOT_ID <= 5 {
                led.toggle();
                wdt.feed();
            }
        }
        delay.delay_ms(600); // 喂狗间隔 600ms < 1.5s ✓
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
