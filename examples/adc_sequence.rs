//! ADC 序列扫描 + DAC 联动（v0.4 新增序列模式验证例）
//!
//! 一次启动转换 3 个通道（ENS=2 → SQRCH0..2）：
//! 1. DAC 通道 2 步进 → 序列扫描 [DAC_OUT2(IN12), TS, BGR]
//! 2. 结果落 RESULT0..2，与 DAC 设定/温度/基准公式交叉核对
//!
//! 实测结论（2026-10-19，本板）：
//! - IN12 回环精确跟随 DAC 设定（±1 LSB）✓
//! - BGR ≈1488（1200/3300×4095=1489）✓
//! - TS 读 0（内置温度传感器个体异常，厂商例程复刻同现象，
//!   见 `AdcError::SensorDead` 保护）；TS 恢复后本例即全绿
//!
//! `RESULTS` = [in12, ts, bgr]，`RUNS` 持续递增 = 固件存活。

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

/// 最近一轮序列结果 [IN12, TS, BGR]
static mut RESULTS: [u16; 3] = [0; 3];
/// 序列转换轮数
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

    let mut _led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);
    let mut dac = Dac::new(p.dac, &mut rcc);
    let mut adc = Adc::new(p.adc1, &mut rcc, AdcClk::Div2);

    let chs = [
        AdcChannel::In(12),
        AdcChannel::TempSensor,
        AdcChannel::Bgr1v2,
    ];
    let samples = [SampleCycles::S24, SampleCycles::S262, SampleCycles::S166];

    loop {
        // DAC 从 0 向 4095 步进（每轮 +512），回环值随之变化
        static mut DAC_SET: u16 = 0;
        let set = unsafe {
            DAC_SET = (DAC_SET + 512) & 0x0FFF;
            DAC_SET
        };
        dac.set_ch2(set).ok();
        delay.delay_us(50);

        let mut out = [0u16; 3];
        if adc.convert_sequence(&chs, &samples, &mut out).is_ok() {
            unsafe {
                RESULTS = out;
                RUNS += 1;
            }
        }
        _led.toggle();
        delay.delay_ms(300);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
