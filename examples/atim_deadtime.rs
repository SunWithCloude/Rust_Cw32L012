//! ATIM 互补 PWM + 死区 + 刹车（v0.5 新增 `atim` 模块验证例）
//!
//! ATIM_CH1 @ PA08（AF7）/ ATIM_CH1N @ PA15（AF7），2kHz、30% 占空比、
//! 死区 250µs（4MHz PCLK 下 1000 t_DTS，落在 DTG 111 编码档）。
//!
//! 验证方式（无示波器也可）：
//! - `REG` 回读 (ARR, PSC)——预期 ARR=1999、PSC=0（4M/2k=2000 ticks）
//! - `CCR` 回读——预期 2000×30% = 600
//! - `BDTR` 回读——预期 0x8000_0000 | 0xE0 | 0x1F... 即 MOE=1、
//!   DTG=0xFF（111 档满格：32+31=63 → 63×16=1008 ticks ≈ 252µs）
//! - `CNT_MIN/MAX`——预期铺满 0..1999
//! - 接示波器：PA08/PA15 互补方波，翻转沿间隔 250µs 互不重叠；
//!   `enable_brake` 后拉低 PA06（ATIM_BK）两路同时熄灭

#![no_std]
#![no_main]

use cw32l012_hal::{
    atim::{Atim, AtimChannel},
    gpio::{Af, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

/// (ARR<<16 | PSC) 配置回读
static mut REG: u32 = 0;
/// 最近一次 CCR 回读
static mut CCR: u16 = 0;
/// BDTR 回读（MOE/DTG/刹车位）
static mut BDTR: u32 = 0;
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

    // 互补输出引脚：PA08 = ATIM_CH1、PA15 = ATIM_CH1N（均 AF7，手册表 9-2）
    let _ch1 = Pin::new(p.gpioa, PortId::A, 8, &mut rcc).into_af_output(Af::Af7);
    let _ch1n = Pin::new(p.gpioa, PortId::A, 15, &mut rcc).into_af_output(Af::Af7);
    // 刹车输入 PA06 = ATIM_BK：必须走 AF7 复用输入 + 上拉（低有效），
    // 悬空或普通 GPIO 输入模式都会让 BRK 采样为低 → 刹车锁存 → MOE=0
    // （实机抓到的坑：普通输入模式下 MOE 被清，输出无波形）
    let _bk = Pin::new(p.gpioa, PortId::A, 6, &mut rcc).into_af_input(Af::Af7);

    let mut tim = Atim::new(p.atim, &mut rcc);
    // 2kHz @ PCLK 4MHz：PSC=0、ARR=1999；30% 占空比；死区 250µs
    match tim.start_pwm_complementary(AtimChannel::Ch1, clocks.pclk_hz, 2_000, 3_000, 250_000) {
        Ok(_) => {}
        Err(_) => loop {
            core::hint::spin_loop();
        },
    }
    // 刹车：BRK 低电平有效（外部拉低 → MOE 硬件清零 → 输出安全态）
    tim.enable_brake(false, 0);
    tim.re_enable_output(); // 若上电瞬间有毛刺已锁存刹车，此处恢复主输出

    unsafe {
        let (arr, psc) = tim.period_config();
        REG = (arr as u32) << 16 | psc as u32;
        CCR = tim.ccr(AtimChannel::Ch1);
        BDTR = tim.bdtr();
    }

    // 运行观测：CNT 滚动范围（也可在此周期性 set_duty 做呼吸）
    loop {
        unsafe {
            let c = tim.count();
            if c < CNT_MIN {
                CNT_MIN = c;
            }
            if c > CNT_MAX {
                CNT_MAX = c;
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
