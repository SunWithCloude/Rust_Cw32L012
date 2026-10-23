//! BTIM1 周期中断驱动 PC13 闪烁（v0.3 新增 [`btim`] 模块实机验证例）
//!
//! 溢出频率 2Hz（500ms 半周期），BTIM1 更新中断里 TOG 翻转 PC13。
//! 与 `blink_pc13`（忙等延时）形成对照：LED 闪烁节奏完全相同，
//! 但 CPU 在 `wfi` 里睡觉，由硬件定时器唤醒。
//!
//! `ISR_COUNT` 静态变量记录中断次数，可用调试器观察递增。

#![no_std]
#![no_main]

use cw32l012_hal::{
    btim::Btim,
    gpio::{Level, Pin, PortId},
    pac,
    rcc::Rcc,
    runtime, Peripherals,
};

/// 中断次数计数（调试器可观察）
static mut ISR_COUNT: u32 = 0;

#[no_mangle]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    // 复位默认 PCLK=4MHz
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    let _led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // BTIM1 @2Hz：PSC/ARR 自动选取，CK_CNT 与溢出频率见 counter_hz()
    let mut tim = Btim::new(p.btim1, &mut rcc);
    match tim.start_periodic(clocks.pclk_hz, 2) {
        Ok(_) => {}
        Err(_) => loop {
            core::hint::spin_loop();
        },
    }
    tim.enable_update_irq(true);

    // NVIC 层使能 BTIM1（IRQ20），然后全局开中断
    unsafe {
        runtime::Interrupt::Btim1.enable();
        core::arch::asm!("cpsie i");
    }

    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

// BTIM1 更新中断：清标志 → 翻转 PC13（TOG 原子写）→ 计数
cw32l012_hal::interrupt!("BTIM1_irq", btim1_isr, {
    unsafe {
        let p = Peripherals::steal();
        p.btim1.icr.write(pac::btim::ICR_CLEAR_UIF);
        p.gpioc.tog.write(1 << 13);
        ISR_COUNT += 1;
    }
});

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
