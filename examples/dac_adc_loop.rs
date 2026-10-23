//! DAC → ADC 片内回环自测（v0.4 新增 `dac` 模块验证例，零外部接线）
//!
//! 原理：DAC 通道 2 的输出在芯片内部直连 ADC1 的 IN12 通道（手册
//! 表 25-4）——写 DAC 一个值，立刻用 ADC 读回来，比对误差。
//!
//! `MAX_ERR` 记录全程最大 |DAC 设定 − ADC 读回|（LSB）。
//! 预期：≤ 30 LSB（DAC/ADC 增益失调 + VDDA 噪声；典型 <15）。
//! `RUNS` 记录完成的 ramp 轮数（持续递增 = 固件活着）。

#![no_std]
#![no_main]

use cw32l012_hal::{
    adc::{Adc, AdcChannel, AdcClk, SampleCycles},
    dac::Dac,
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

/// 全程最大回环误差（LSB）
static mut MAX_ERR: u32 = 0;
/// 完成 ramp 轮数
static mut RUNS: u32 = 0;

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
    let mut dac = Dac::new(p.dac, &mut rcc);
    let mut adc = Adc::new(p.adc1, &mut rcc, AdcClk::Div2);

    loop {
        // 0 → 4095 斜坡，每 256 LSB 一个测试点
        for step in 0..=16u16 {
            let set = step * 256;
            let set = if set > 4095 { 4095 } else { set };
            dac.set_ch2(set).ok();
            // 等待 DHR→DOR 传输 + 模拟建立（TEN=0 下 1 APCLK 转移，
            // 建立 RC 时间常数在 µs 级；给足裕量）
            delay.delay_us(50);
            if let Ok(raw) = adc.convert(AdcChannel::In(12), SampleCycles::S24) {
                let err = if raw > set { raw - set } else { set - raw };
                unsafe {
                    if err as u32 > MAX_ERR {
                        MAX_ERR = err as u32;
                    }
                }
            }
        }
        unsafe {
            RUNS += 1;
        }
        led.toggle();
        delay.delay_ms(500);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
