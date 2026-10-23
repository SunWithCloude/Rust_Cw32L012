//! GTIM1 PWM 输出（v0.4 新增 `gtim` 模块验证例）
//!
//! GTIM1_CH1 @ PB04（AF6），1kHz、占空比 10%→90% 呼吸扫动。
//! 验证方式（无示波器也可）：
//! - `REG` 静态变量记录 (ARR, PSC, CCR) 回读值——预期 ARR=3999、
//!   PSC=0、CCR 随扫动变化
//! - `CNT_MIN/MAX` 采样计数器滚动范围——预期铺满 0..ARR
//! - 接示波器/逻辑分析仪到 PB04 可看 1kHz 方波，占空比缓变

#![no_std]
#![no_main]

use cw32l012_hal::{
    gpio::{Af, Pin, PortId},
    gtim::{Gtim, PwmChannel},
    rcc::Rcc,
    Peripherals,
};

/// (ARR<<16 | PSC) 配置回读
static mut REG: u32 = 0;
/// 最近一次 CCR 回读
static mut CCR: u16 = 0;
/// CNT 采样最小值
static mut CNT_MIN: u16 = 0xFFFF;
/// CNT 采样最大值
static mut CNT_MAX: u16 = 0;

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

    // PWM 引脚：PB04 = GTIM1_CH1（AF6，手册表 9-2）
    let _pwm_pin = Pin::new(p.gpiob, PortId::B, 4, &mut rcc).into_af_output(Af::Af6);

    let mut tim = Gtim::new(p.gtim1, &mut rcc);
    // 1kHz @ PCLK 4MHz：PSC=0、ARR=3999
    match tim.start_pwm(PwmChannel::Ch1, clocks.pclk_hz, 1_000, 1_000) {
        Ok(_) => {}
        Err(_) => loop {
            core::hint::spin_loop();
        },
    }

    unsafe {
        let (arr, psc) = tim.period_config();
        REG = (arr as u32) << 16 | psc as u32;
    }

    // 占空比 10%→90%→10% 循环扫动（每步 5%）
    let mut duty: u16 = 1_000;
    let mut dir_up = true;
    loop {
        tim.set_duty(PwmChannel::Ch1, duty).ok();
        unsafe {
            CCR = tim.ccr(PwmChannel::Ch1);
            let c = tim.count();
            if c < CNT_MIN {
                CNT_MIN = c;
            }
            if c > CNT_MAX {
                CNT_MAX = c;
            }
        }
        if dir_up {
            duty += 500;
            if duty >= 9_000 {
                dir_up = false;
            }
        } else {
            duty = duty.saturating_sub(500);
            if duty <= 1_000 {
                dir_up = true;
            }
        }
        // 无延时：占空比高速扫动，示波器上呈呼吸效果
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
