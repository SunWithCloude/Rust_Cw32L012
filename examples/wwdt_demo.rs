//! WWDT 窗口看门狗（v0.5 新增 `wwdt` 模块验证例）
//!
//! 喂狗窗口 [98.3ms, 200.7ms]（`WindowConfig::for_feed_interval` 自动求解，
//! PCLK 4MHz → PRS=2、WCNT=0x70、WINR=0x58）。主循环 150ms 一轮：
//!
//! 1. 先 delay 50ms → `feed()`：当前计数仍在窗口值之上 → **必须**返回
//!    `TooEarly`（HAL 拒绝写，芯片不复位）→ `EARLY` 计数；
//! 2. 再 delay 100ms → `feed()`：落在窗口内 → 刷新成功 → `FEEDS` 计数。
//!
//! 验证方式（读静态变量）：
//! - `FEEDS` 持续 +1（150ms/轮）且程序永不复位（PC13 0.15s 闪烁不中断）
//! - `EARLY` 同步 +1（每轮一次过早喂狗被安全拦截）
//! - `WW_CFG` = (PRS<<16 | WCNT<<8 | WINR) 回读——预期 0x0002_7058
//! - 若注释掉 `feed()` 的窗口检查（改用 `feed_forced`）并保持过早喂狗，
//!    芯片将复位循环——PC13 常灭，即窗口保护生效的对照证据

#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    wwdt::{FeedError, WindowConfig, Wwdt},
    Peripherals,
};

/// (PRS<<16 | WCNT<<8 | WINR) 配置回读
static mut WW_CFG: u32 = 0;
/// 成功喂狗次数
static mut FEEDS: u32 = 0;
/// 被拦截的过早喂狗次数
static mut EARLY: u32 = 0;
/// 最近一次喂狗时的计数值
static mut LAST_CNT: u8 = 0;

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
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::High);
    let mut delay = Delay::new(&clocks);

    let mut wwdt = Wwdt::new(p.wwdt, &mut rcc);
    let cfg = match WindowConfig::for_feed_interval(clocks.pclk_hz, 200) {
        Some(c) => c,
        None => loop {
            core::hint::spin_loop();
        },
    };
    wwdt.start(cfg);
    unsafe {
        WW_CFG = (cfg.prs as u32) << 16 | (cfg.wcnt as u32) << 8 | cfg.winr as u32;
    }

    loop {
        // 50ms 时尝试过早喂狗 → HAL 检查窗口并拒绝（不写寄存器、不复位）
        delay.delay_ms(50);
        unsafe {
            LAST_CNT = wwdt.count();
            if wwdt.feed() == Err(FeedError::TooEarly) {
                EARLY += 1;
            }
        }
        // 累计到 150ms（50+100）→ 落在 [98, 201]ms 窗口内 → 喂狗成功
        delay.delay_ms(100);
        unsafe {
            if wwdt.feed().is_ok() {
                FEEDS += 1;
            }
        }
        led.toggle();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
