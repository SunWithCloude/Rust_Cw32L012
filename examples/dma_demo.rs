//! DMA 自测：存储器→存储器 + ADC 硬件触发搬运（零接线）
//!
//! 两个相位，全部结果写进静态量，跑完用 probe-rs 直读 SRAM：
//!
//! - **P1 存储器→存储器**：64 个 u32 从 `SRC` 搬到 `DST`（32 位宽、BULK、
//!   软件触发），逐字比对，记录首个不匹配下标。
//! - **P2 ADC 硬件触发**：DAC2 输出恒定码 → 内部互联 ADC1_IN12，ADC 连续
//!   转换，每次 EOC 由 DMA 通道 1 搬一个 16 位结果进 `BUF[256]`，
//!   **CPU 全程不参与**。搬运完统计 min/max/mean，应与 DAC 码一致（±3 LSB）。
//!
//! SRAM 布局（`nm` 实测后再读）：
//! `SRC[64×u32]`、`DST[64×u32]`、`BUF[256×u16]`、以及若干 u32/u16 统计量。

#![no_std]
#![no_main]

use core::ptr::{addr_of, addr_of_mut, write_volatile};

use cw32l012_hal::{
    adc::{Adc, AdcChannel, AdcClk, SampleCycles},
    dac::Dac,
    delay::Delay,
    dma::{Config, Dma, Width},
    gpio::{Level, Pin, PortId},
    pac,
    rcc::Rcc,
    Peripherals,
};

/// 写入观察量（避免被优化掉）
macro_rules! put {
    ($p:expr, $v:expr) => {
        write_volatile($p, $v)
    };
}

/// P1 源数据（常量表，编译期生成）
static SRC: [u32; 64] = {
    let mut a = [0u32; 64];
    let mut i = 0;
    while i < 64 {
        a[i] = 0xA5A5_0000 ^ ((i as u32) * 0x0001_0101);
        i += 1;
    }
    a
};
/// P1 目的缓冲
static mut DST: [u32; 64] = [0; 64];
/// P2 ADC 采样缓冲
static mut BUF: [u16; 256] = [0; 256];

/// P1：比对通过的元素数
static mut P1_OK: u32 = 0;
/// P1：首个不匹配下标（0xFFFF_FFFF = 全部一致）
static mut P1_BAD: u32 = 0;
/// P1：DMA 错误码（0 = 无错）
static mut P1_ERR: u16 = 0;
/// P2：DMA 错误码
static mut P2_ERR: u16 = 0;
/// P2：搬运后剩余数量（应为 0）
static mut P2_REMAIN: u16 = 0;
/// P2：通道状态（5 = 传输完成）
static mut P2_STATUS: u8 = 0;
/// P2：min / max / mean / 首样本
static mut P2_MIN: u16 = 0;
static mut P2_MAX: u16 = 0;
static mut P2_MEAN: u16 = 0;
static mut P2_FIRST: u16 = 0;
/// P2：与 DAC 码偏差超过 16 LSB 的样本数
static mut P2_OUT: u32 = 0;

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
    let mut dma = Dma::new(p.dma, &mut rcc);

    // ---------------- P1：存储器→存储器 ----------------
    let mut p1_err = 0u16;
    unsafe {
        let src = core::slice::from_raw_parts(addr_of!(SRC) as *const u32, 64);
        let dst = core::slice::from_raw_parts_mut(addr_of_mut!(DST) as *mut u32, 64);
        if let Err(e) = dma.mem_to_mem(src, dst) {
            let _ = e;
            p1_err = 1; // 0 保留给"成功"
        }
    }
    // SAFETY: 单核；DMA 已完成
    unsafe {
        let mut ok = 0u32;
        let mut bad = 0xFFFF_FFFFu32;
        for i in 0..64 {
            if *addr_of!(DST).cast::<u32>().add(i) == SRC[i] {
                ok += 1;
            } else if bad == 0xFFFF_FFFF {
                bad = i as u32;
            }
        }
        put!(addr_of_mut!(P1_OK), ok);
        put!(addr_of_mut!(P1_BAD), bad);
        put!(addr_of_mut!(P1_ERR), p1_err);
    }

    // ---------------- P2：ADC 硬件触发 → 内存 ----------------
    let mut dac = Dac::new(p.dac, &mut rcc);
    let _ = dac.set_ch2(2048); // DAC_OUT2 → ADC1_IN12
    let mut adc = Adc::new(p.adc1, &mut rcc, AdcClk::Div4); // ADCCLK = 1MHz，S166 = 166µs
                                                            // 先单次转换一次：配好 SQRCH0/SAMPLE，同时确认回环通路
    let _ = adc.convert(AdcChannel::In(12), SampleCycles::S166);

    let mut p2_err = 0u16;
    let remain: u16;
    let status: u8;
    unsafe {
        let buf = core::slice::from_raw_parts_mut(addr_of_mut!(BUF) as *mut u16, 256);
        let ch = dma.channel(1);
        ch.configure(&Config::periph_to_mem(
            pac::ADC1_BASE as u32 + 0x30, // RESULT0
            buf.as_mut_ptr() as u32,
            buf.len() as u16,
            Width::B16,
            pac::dma::hs::ADC1_SINGLE,
        ));
        // 打开 ADC 的 EOC→DMA 请求，再进连续转换
        adc.regs().ier.modify(|v| v | ac::IER_DMAEOC);
        adc.regs().cr.modify(|v| v | ac::CR_CONT);
        adc.regs().start.write(ac::START);

        match ch.wait() {
            Ok(()) => {}
            Err(e) => {
                let _ = e;
                p2_err = 1;
            }
        }
        remain = ch.remaining();
        status = ch.status();

        // 收尾：停 ADC、关 DMA 通道
        adc.regs().cr.modify(|v| v & !ac::CR_CONT);
        adc.regs().start.write(0);
        adc.regs().ier.modify(|v| v & !ac::IER_DMAEOC);
        ch.disable();

        // 统计
        let mut min = u16::MAX;
        let mut max = 0u16;
        let mut sum = 0u32;
        let mut out = 0u32;
        for i in 0..256 {
            let v = *buf.as_ptr().add(i);
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
            sum += v as u32;
            if v.abs_diff(2048) > 16 {
                out += 1;
            }
        }
        put!(addr_of_mut!(P2_ERR), p2_err);
        put!(addr_of_mut!(P2_REMAIN), remain);
        put!(addr_of_mut!(P2_STATUS), status);
        put!(addr_of_mut!(P2_MIN), min);
        put!(addr_of_mut!(P2_MAX), max);
        put!(addr_of_mut!(P2_MEAN), (sum / 256) as u16);
        put!(addr_of_mut!(P2_FIRST), *buf.as_ptr());
        put!(addr_of_mut!(P2_OUT), out);
    }

    // 收尾后闪灯表示跑完
    loop {
        delay.delay_ms(200);
        // SAFETY: 单核
        unsafe { core::arch::asm!("nop") };
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
