//! TS POR 实验：验证「模拟块偏置状态跨系统复位保持」假说
//!
//! 手册依据：
//! - 表 4-1：SYSRESETREQ 只复位"内核+外设"，POR/BOR 才复位"整个 MCU"；
//! - §25.12.19 注："BGR 只能软件失能或 POR reset 才能失能"——暗示模拟
//!   偏置状态跨系统复位保持。probe-rs download/reset 走的是 SYSRESETREQ。
//!
//! 流程：上电即快照 RESETFLAG（bit0=POR/BOR、bit9=SYSRESETREQ）→ 清全部
//! 标志 → 采 TS×3 + BGR（相位 A）→ BGR 软件完整重启后采 TS×3（相位 B）
//! → 快照 BGR_CR 与结束时的 RESETFLAG，进 wfi 等待读取。
//!
//! 判定：断电重启后 RF_BOOT 应为 0x1（POR 置位、无软复位残留）；
//! 若 TS_A/B >750 counts → 模拟状态挂死假说成立（需 POR 恢复）；
//! 若仍 ≈0 → 软件侧手段穷尽，TS 输出级无输出为硬件结论。

#![no_std]
#![no_main]

use cw32l012_hal::{
    adc::{Adc, AdcClk},
    delay::Delay,
    gpio::{Level, Pin, PortId},
    pac,
    rcc::Rcc,
    Peripherals,
};

/// RESETFLAG 复位后原值（bit0=POR、bit9=SYSRESETREQ）
static mut RF_BOOT: u32 = 0;
/// 相位 A：POR 后直接采 TS×3（S518）
static mut TS_A: [u16; 3] = [0; 3];
/// 相位 A 的 BGR 对照
static mut BGR_A: u16 = 0;
/// 相位 B：BGR 软件重启后 TS×3
static mut TS_B: [u16; 3] = [0; 3];
/// BGR_CR 轨迹（A 后 / 重启中 / B 后）
static mut BLOG: [u32; 3] = [0; 3];
/// 结束时 RESETFLAG（应只有本 Boot 的标志）
static mut RF_END: u32 = 0;

/// 绕过驱动前置条件的原始单次转换
#[inline]
fn raw(adc: &mut Adc, ch: u32, sample: u32) -> u16 {
    use cw32l012_hal::pac::adc as ac;
    let r = adc.regs();
    r.start.write(0);
    r.sqrcfr.write(ch);
    r.sample.write(sample);
    r.icr.write(ac::ICR_CLEAR_EOC_EOS);
    r.start.write(ac::START);
    let mut n = 1 << 22;
    while r.isr.read() & ac::ISR_EOC == 0 {
        if n == 0 {
            return 0xFFFF;
        }
        n -= 1;
        core::hint::spin_loop();
    }
    let v = (r.result0.read() & 0xFFF) as u16;
    r.icr.write(ac::ICR_CLEAR_EOC_EOS);
    v
}

#[no_mangle]
fn main() -> ! {
    use cw32l012_hal::pac::adc as ac;
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let mut delay = Delay::new(&rcc.clocks());
    let _led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);
    let mut adc = Adc::new(p.adc1, &mut rcc, AdcClk::Div2);

    let cr = pac::BGR_BASE as *mut u32;
    let rf = (pac::SYSCTRL_BASE + 0x4C) as *mut u32; // SYSCTRL_RESETFLAG
                                                     // SAFETY: 单核独占，顺序执行
    unsafe {
        RF_BOOT = rf.read_volatile();
        rf.write_volatile(0x0); // 清全部复位标志（RW0）

        // ---- 相位 A：当前（POR 后默认）BGR 状态直接采 ----
        for i in 0..3 {
            TS_A[i] = raw(&mut adc, 0xE, 15);
        }
        BGR_A = raw(&mut adc, 0xF, 12);
        BLOG[0] = cr.read_volatile();

        // ---- 相位 B：BGR 软件完整重启 ----
        adc.regs().cr.modify(|v| v & !ac::CR_EN);
        cr.write_volatile(0x0); // BGREN=0、TSEN=0
        delay.delay_ms(2);
        cr.write_volatile(0x2); // TSEN=1（触发自动启动）
        delay.delay_ms(1);
        cr.write_volatile(cr.read_volatile() | 0x1); // BGREN=1
        delay.delay_ms(100);
        BLOG[1] = cr.read_volatile();
        adc.regs().cr.modify(|v| v | ac::CR_EN);
        delay.delay_us(10);
        for i in 0..3 {
            TS_B[i] = raw(&mut adc, 0xE, 15);
        }
        BLOG[2] = cr.read_volatile();

        RF_END = rf.read_volatile();
    }

    loop {
        // SAFETY: wfi 无副作用
        unsafe { core::arch::asm!("wfi") };
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
