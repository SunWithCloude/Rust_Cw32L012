//! ADC 内部通道自测（v0.3 新增 [`adc`] 模块实机验证例）
//!
//! 不接任何外部信号即可验证：
//! 1. `vdda_mv()`     — 经 1.2V 基准 + 出厂校准反推 VDDA（预期 ≈3.3V 上下）
//! 2. `temperature_mc()` — 内置温度传感器（预期室温 ±10℃）
//!
//! 结果写入 `VDDA_MV` / `TEMP_MC` 静态变量（符号可用
//! `llvm-nm target/.../examples/adc_internal | grep -E "VDDA|TEMP"` 定位），
//! 供调试器/探针读取核对。PC13 每 2 秒心跳翻转一次表示主循环存活。

#![no_std]
#![no_main]

use cw32l012_hal::{
    adc::{Adc, AdcClk},
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

/// 最近一次 VDDA 测量（mV）
static mut VDDA_MV: u32 = 0;
/// 最近一次温度测量（毫摄氏度）
static mut TEMP_MC: i32 = 0;
/// 测量轮数
static mut SAMPLES: u32 = 0;

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
    // ADC1 @ ADCCLK=PCLK/2（保守；TS/BGR 通道用长采样周期补偿源阻抗）
    let mut adc = Adc::new(p.adc1, &mut rcc, AdcClk::Div2);

    loop {
        // SAFETY: main 独占这些静态变量（单核、ISR 不访问）
        unsafe {
            if let Ok(mv) = adc.vdda_mv() {
                VDDA_MV = mv;
            }
            if let Some(t) = adc.temperature_mc().ok() {
                TEMP_MC = t;
            }
            SAMPLES += 1;
        }
        led.toggle();
        delay.delay_ms(2000);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
