//! LPTIM@LSI：频率自校准 + Sleep/DeepSleep 中断唤醒（v0.6 验证例）
//!
//! 三阶段：
//! 1. **校准**：LPTIM 以 LSI 计数（DIV1、ARR=4095），5 秒内轮询 ARRM
//!    次数 → `LSI_HZ = ticks × 4096 / 5`（HCLK 为 HSI 4MHz 已知基准）；
//! 2. **Sleep 唤醒**：ARR=0x7FFF（≈1s），ARRM 中断经 `wfi` 唤醒 ×3；
//! 3. **DeepSleep 唤醒**：同一中断在深度休眠下唤醒 CPU ×3（LSI 时钟域
//!    持续计数，表 3-3 深度休眠列 Y）。
//!
//! 静态量：`LSI_HZ` / `CAL_TICKS` / `SLEEP_WAKES` / `DS_WAKES`，
//! PC13 每次中断翻转（DeepSleep 阶段凭 LED 可见节奏）。

#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    lptim::{flags, Lptim, LptimClk},
    pac, pwr,
    rcc::Rcc,
    runtime, Peripherals,
};

/// 实测 LSI 频率（Hz）
static mut LSI_HZ: u32 = 0;
/// 校准窗口内 ARRM 次数
static mut CAL_TICKS: u32 = 0;
/// Sleep 模式中断唤醒次数
static mut SLEEP_WAKES: u32 = 0;
/// DeepSleep 模式中断唤醒次数
static mut DS_WAKES: u32 = 0;
/// 相位：0=校准 1=Sleep 2=DeepSleep（ISR 分流计数）
static mut PHASE: u8 = 0;

#[no_mangle]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let mut delay = Delay::new(&rcc.clocks());
    let _led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // LPTIM @ LSI，DIV1
    let mut lpt = Lptim::new(p.lptim, &mut rcc, LptimClk::Lsi, 0);
    lpt.set_arr(0x0FFF); // 校准期 ≈125ms/周期 @32.8kHz
    lpt.enable();
    lpt.start_continuous();

    // ---- 阶段 1：5 秒校准窗口（轮询，CPU 不睡）----
    let mut ticks = 0u32;
    for _ in 0..2500 {
        // 2500 × 2ms = 5s
        if lpt.arrm_flag() {
            lpt.clear_irq(flags::ARRM);
            ticks += 1;
        }
        delay.delay_ms(2);
    }
    let lsi_hz = ticks * 4096 / 5;
    // 用 volatile 写，避免优化构建把这些"只写不读"的静态量整段删掉
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(CAL_TICKS), ticks);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(LSI_HZ), lsi_hz);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(PHASE), 1);
    }

    // ---- 阶段 2/3 准备：≈1s 周期 + ARRM 中断 ----
    lpt.disable(); // EN=0 才能改 IER/CFGR
    lpt.clear_irq(flags::ARRM);
    lpt.enable_irq(flags::ARRM);
    lpt.set_arr(0x7FFF);
    lpt.enable();
    lpt.start_continuous();
    unsafe {
        runtime::Interrupt::Lptim.enable();
        core::arch::asm!("cpsie i");
    }

    // ---- 阶段 2：Sleep ×3 ----
    for _ in 0..3 {
        pwr::sleep();
    }

    // ---- 阶段 3：DeepSleep ×3 ----
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(PHASE), 2) };
    for _ in 0..3 {
        let _ = pwr::deep_sleep(true); // 唤醒后 SYSCLK 切 HSI 4MHz
    }

    loop {
        // SAFETY: wfi 无副作用
        unsafe { core::arch::asm!("wfi") };
    }
}

// LPTIM ARRM 中断：清标志 → 按相位计数 → 翻转 PC13
cw32l012_hal::interrupt!("LPTIM_irq", lptim_isr, {
    unsafe {
        let p = Peripherals::steal();
        p.lptim
            .icr
            .write(pac::lptim::FLAGS_MASK & !pac::lptim::ARRM);
        p.gpioc.tog.write(1 << 13);
        let ph = core::ptr::read_volatile(core::ptr::addr_of!(PHASE));
        let cell = if ph == 1 {
            core::ptr::addr_of_mut!(SLEEP_WAKES)
        } else if ph == 2 {
            core::ptr::addr_of_mut!(DS_WAKES)
        } else {
            core::ptr::null_mut()
        };
        if !cell.is_null() {
            let v = core::ptr::read_volatile(cell as *const u32);
            core::ptr::write_volatile(cell, v + 1);
        }
    }
});

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
