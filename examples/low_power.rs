//! 低功耗模式（v0.5 新增 `pwr` 模块验证例）：Sleep + DeepSleep
//!
//! 阶段 1（Sleep）：BTIM1 2Hz 更新中断作唤醒源。主循环
//! `sleep()` → 中断唤醒（ISR 只清标志）→ 主循环翻转 PC13。
//! 共 10 轮 → PC13 0.5s 方波 10 下 = 休眠/唤醒循环工作。
//!
//! 阶段 2（DeepSleep）：IWDT 2s 溢出中断作唤醒源（RC10K 域，
//! DeepSleep 下继续计数）。`deep_sleep(true)` 置 WAKEUPCLK → 唤醒后
//! 系统时钟自动切 HSI 4MHz。ISR 里清 OV 标志 + 喂狗防复位；主循环
//! 记录唤醒次数与唤醒后的 SYSCLK 选择。共 6 轮 → PC13 2s 慢闪。
//!
//! 验证方式（读静态变量 / 电流计）：
//! - `SLEEPS`=10（阶段 1 唤醒次数）、`DS_WAKES`=6（阶段 2）
//! - `WAKE_CLK`——预期 0x0（WAKEUPCLK=1 → 唤醒后 SYSCLK=HSI）
//! - `WDT_FEEDS`=6 ——IWDT ISR 喂狗次数 = DS_WAKES
//! - 万用表/电流计：DeepSleep 阶段平均电流显著低于 Sleep 阶段
//!   （高速时钟关闭的实测证据）
//! - 若把 ISR 里的喂狗注释掉：IWDT 2s 后 ACTION=中断不复位……
//!   注意中断模式下不会复位，重启后改回 ACTION=复位模式可验证复位

#![no_std]
#![no_main]

use cw32l012_hal::{
    btim::Btim,
    delay::Delay,
    gpio::{Level, Pin, PortId},
    pac, pwr,
    rcc::Rcc,
    runtime::Interrupt,
    wdt::TimeoutMs,
    Peripherals,
};

/// 阶段 1 Sleep 唤醒次数
static mut SLEEPS: u32 = 0;
/// 阶段 2 DeepSleep 唤醒次数
static mut DS_WAKES: u32 = 0;
/// 唤醒后的 SYSCLK 时钟源选择（CR0[2:0]，WAKEUPCLK=1 → 0=HSI）
static mut WAKE_CLK: u32 = 0xFF;
/// IWDT ISR 喂狗次数
static mut WDT_FEEDS: u32 = 0;

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

    // ---- 阶段 1：Sleep + BTIM1 2Hz（每 500ms 唤醒一次）----
    rcc.enable_btim();
    let mut btim = Btim::new(p.btim1, &mut rcc);
    if btim.start_periodic(clocks.pclk_hz, 2).is_err() {
        loop {
            core::hint::spin_loop();
        }
    }
    btim.enable_update_irq(true);
    Interrupt::Btim1.enable();
    for _ in 0..10 {
        pwr::sleep(); // 唤醒源：BTIM1 更新中断（ISR 清标志）
        unsafe {
            SLEEPS += 1;
            led.toggle();
        }
    }

    // ---- 阶段 2：DeepSleep + IWDT 2s 溢出中断 ----
    Interrupt::Wdt.enable();
    let mut iwdt = cw32l012_hal::wdt::Iwdt::new(p.iwdt, &mut rcc);
    // 溢出=中断模式（不复位），2s 超时；RC10K 域 DeepSleep 下继续跑
    if iwdt.start_interrupt(TimeoutMs(2000)).is_err() {
        loop {
            core::hint::spin_loop();
        }
    }
    for _ in 0..6 {
        if pwr::deep_sleep(true).is_ok() {
            unsafe {
                DS_WAKES += 1;
                // WAKEUPCLK=1 → 唤醒后 SYSCLK 应已切 HSI（CR0[2:0]=0b000）
                WAKE_CLK = p.sysctrl.cr0.read() & 0x7;
                led.toggle();
            }
        }
        delay.delay_ms(10);
    }

    // ---- 收尾：停 IWDT（调试双键），快速闪烁表示流程完成 ----
    iwdt.stop();
    loop {
        led.toggle();
        delay.delay_ms(100);
    }
}

// BTIM1 更新中断（阶段 1 唤醒源）：清标志
cw32l012_hal::interrupt!("BTIM1_irq", btim1_isr, {
    unsafe {
        let p = Peripherals::steal();
        p.btim1.icr.write(pac::btim::ICR_CLEAR_UIF);
    }
});

// WDT 中断（阶段 2 唤醒源，IRQ0）：IWDT 溢出（ACTION=中断）→ 清 OV + 喂狗
cw32l012_hal::interrupt!("WDT_irq", wdt_isr, {
    unsafe {
        let p = Peripherals::steal();
        // SR.OV（bit3）RW0：写 0 清除，其余位保持 1
        p.iwdt.sr.modify(|v| v & !(1 << 3));
        p.iwdt.kr.write(0xAAAA); // 喂狗（KR_FEED）
        WDT_FEEDS += 1;
    }
});

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
