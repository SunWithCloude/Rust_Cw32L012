//! TS 排查第二轮：BGR 完整重启 + ADC2/0xE + 极限采样 + 60s 预热
//!
//! 背景：第一轮（ts_probe）证明 TS 节点 ≈0V 但呈高阻负载（S6 残留 83
//! counts）。手册 §25.12.19 注："BGREN、ADCEN、TSEN、VC1/2/3/4、OPA1/2
//! 使能都会自动启动 BGR"、"BGR 只能软件失能或 POR reset 才能失能"。
//! 若 TS 偏置挂在 BGR 启动状态机上，完整的 BGR 软件断电→按序重启是
//! 最后一个未尝试的软件手段。
//!
//! 相位：
//! - `P1[0..3]`  — 完整重启：EN=0 → BGR_CR=0（全关，2ms）→ TSEN=1
//!   （触发自动启动）→ BGREN=1 → 100ms → EN=1 → TS×3（S518）
//! - `P2[0..3]`  — BGR 单独反弹：BGREN=0（TSEN 保持，2ms）→ BGREN=1
//!   → 100ms → TS×3
//! - `P3_ADC2`   — ADC2 通道 0xE（手册称内部通道仅 ADC1，纯对照）
//! - `P4_DIV8`   — ADCCLK=500kHz + S518（采样 1.04ms，极限长）
//! - `WARM[12]`  — 每 5s 一次的 60s 预热曲线
//! - `BLOG[6]`   — BGR_CR 轨迹（重启各节点）
//! - `ADC1_CR` / `ADC2_CR` — CR 快照（验证 bit8 保留位保持）

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

/// P1：BGR 完整重启后 TS×3
static mut P1: [u16; 3] = [0; 3];
/// P2：BGR 单独反弹后 TS×3
static mut P2: [u16; 3] = [0; 3];
/// P3：ADC2 通道 0xE
static mut P3_ADC2: u16 = 0;
/// P3 后 ADC2_CR 快照
static mut ADC2_CR: u32 = 0;
/// P4：Div8 + S518 的 TS
static mut P4_DIV8: u16 = 0;
/// P5：60s 预热曲线（每 5s）
static mut WARM: [u16; 12] = [0; 12];
/// BGR_CR 轨迹
static mut BLOG: [u32; 6] = [0; 6];
/// ADC1_CR 快照
static mut ADC1_CR: u32 = 0;
/// P4 恢复 Div2 后的 TS 对照
static mut TS_BACK: u16 = 0;

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
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);
    let mut adc = Adc::new(p.adc1, &mut rcc, AdcClk::Div2); // ADCCLK=2MHz

    let cr = pac::BGR_BASE as *mut u32;
    // SAFETY: 单核独占，按相位顺序执行
    unsafe {
        ADC1_CR = adc.regs().cr.read();

        // ---- P1：BGR 完整重启 ----
        adc.regs().cr.modify(|v| v & !ac::CR_EN); // ADC1 先关
        cr.write_volatile(0x0); // BGREN=0、TSEN=0（软件失能 BGR）
        delay.delay_ms(2);
        BLOG[0] = cr.read_volatile(); // 看是否被自动拉回
        cr.write_volatile(0x2); // TSEN=1（应触发 BGR 自动启动）
        delay.delay_ms(1);
        BLOG[1] = cr.read_volatile();
        cr.write_volatile(cr.read_volatile() | 0x1); // BGREN=1 显式
        delay.delay_ms(100);
        BLOG[2] = cr.read_volatile();
        adc.regs().cr.modify(|v| v | ac::CR_EN);
        delay.delay_us(10);
        for i in 0..3 {
            P1[i] = raw(&mut adc, 0xE, 15);
        }
        BLOG[3] = cr.read_volatile();

        // ---- P2：BGR 单独反弹（TSEN 保持 1）----
        cr.write_volatile(cr.read_volatile() & !0x1);
        delay.delay_ms(2);
        cr.write_volatile(cr.read_volatile() | 0x1);
        delay.delay_ms(100);
        for i in 0..3 {
            P2[i] = raw(&mut adc, 0xE, 15);
        }
        BLOG[4] = cr.read_volatile();

        // ---- P3：ADC2 通道 0xE（对照）----
        let a2 = &*(pac::ADC2_BASE as *const pac::Adc);
        a2.cr
            .modify(|v| (v & !ac::CR_CLK_MASK) | ac::CR_RESET | (0 << ac::CR_CLK_POS));
        a2.cr.modify(|v| v | ac::CR_EN);
        a2.start.write(0);
        a2.sqrcfr.write(0xE);
        a2.sample.write(15);
        a2.icr.write(ac::ICR_CLEAR_EOC_EOS);
        a2.start.write(ac::START);
        let mut n = 1 << 22;
        while a2.isr.read() & ac::ISR_EOC == 0 {
            if n == 0 {
                break;
            }
            n -= 1;
            core::hint::spin_loop();
        }
        P3_ADC2 = (a2.result0.read() & 0xFFF) as u16;
        ADC2_CR = a2.cr.read();
        a2.cr.modify(|v| v & !ac::CR_EN);

        // ---- P4：Div8 极限长采样（S518 = 1.04ms）----
        adc.regs()
            .cr
            .modify(|v| (v & !ac::CR_CLK_MASK) | (3 << ac::CR_CLK_POS));
        delay.delay_us(10);
        P4_DIV8 = raw(&mut adc, 0xE, 15);

        // ---- P5：60s 预热曲线（Div8 保持）----
        for i in 0..12 {
            WARM[i] = raw(&mut adc, 0xE, 15);
            led.toggle();
            delay.delay_ms(5000);
        }
        BLOG[5] = cr.read_volatile();

        // ---- 恢复 Div2 对照 ----
        adc.regs()
            .cr
            .modify(|v| (v & !ac::CR_CLK_MASK) | (1 << ac::CR_CLK_POS));
        delay.delay_us(10);
        TS_BACK = raw(&mut adc, 0xE, 15);
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
