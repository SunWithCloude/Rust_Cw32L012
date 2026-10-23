//! DAC → OPA 跟随器 → PB00 → VC1 比较器模拟回环（v0.6 验证例，零外部接线）
//!
//! 链路（全部片内/引脚，无需跳线）：
//! 1. `DAC_OUT1` 内部信号 → OPA1 正端 INP4（手册 §29.6.1 CR.INP4EN）；
//! 2. OPA1 设为**跟随器**，输出脚 `OPA1_OUT = PB00`（表 29-1）；
//! 3. `VC1_CH3 = PB00`（表 27-2）——同一个焊盘，配置为模拟；
//! 4. VC1 负端 = 内部电阻分压 `VDDA×(1+3)/8 = VDDA/2`。
//!
//! 于是 DAC 电压升到 VDDA/2 时 VC1 输出翻转。固件扫描 DAC 找翻转点：
//! 理论 `CROSS ≈ 2048`（VDDA 3.3V 时 1.65V）。这同时验证了 DAC、OPA、
//! VC 三个模块与它们的引脚复用关系。
//!
//! 静态量：`CROSS`（翻转码值）、`LOW_FLV`（DAC=0 时比较器输出）、
//! `HIGH_FLV`（DAC=4095 时输出）、`RUNS`（扫描轮数）

#![no_std]
#![no_main]

use cw32l012_hal::{
    dac::Dac,
    delay::Delay,
    gpio::{Level, Pin, PortId},
    opa::{Opa, OpaMode},
    rcc::Rcc,
    vc::{NnInput, NpInput, Speed, Vc},
    Peripherals,
};

/// DAC 扫描找到的比较器翻转点（0xFFFF = 未找到）
static mut CROSS: u16 = 0xFFFF;
/// DAC=0 时 VC1 输出电平
static mut LOW_FLV: u8 = 0;
/// DAC=4095 时 VC1 输出电平
static mut HIGH_FLV: u8 = 0;
/// 扫描轮数
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

    let _led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // PB00 = OPA1_OUT 也是 VC1_CH3：必须配成**模拟**（ANALOG=1）。
    // 实测坑：不配模拟时 DIR=0（输出）+ODR=0 的数字驱动器把节点压在 0V，
    // OPA 输出被短路，比较器恒为低（HIGH_FLV=0、CROSS=0xFFFF）。
    let _aout = Pin::new(p.gpiob, PortId::B, 0, &mut rcc).into_analog();

    let mut dac = Dac::new(p.dac, &mut rcc);
    dac.set_ch1(0).ok();

    // OPA1 跟随器：正端 INP4 = DAC_OUT1 内部信号，输出脚 PB00
    let mut opa = Opa::new(p.opa1, &mut rcc);
    opa.enable_inputs(cw32l012_hal::pac::opa::CR_INP4EN, 0);
    opa.set_mode(OpaMode::Follower);
    opa.enable();
    delay.delay_us(100);

    // VC1：正端 PB00（CH3），负端内部 VDDA×(1+3)/8
    let mut vc = Vc::new(p.vc1, p.vc12ref, &mut rcc, 1);
    vc.ref_enable(3, false); // VDDA/2
    vc.select_inputs(NpInput::Ch3, NnInput::RefDiv);
    vc.set_speed(Speed::High); // 高速（自动开 BGR）
    vc.set_hysteresis(false);
    vc.enable();
    delay.delay_ms(1);

    loop {
        // 两端端点电平
        dac.set_ch1(0).ok();
        delay.delay_us(200);
        let low = vc.output_high() as u8;

        dac.set_ch1(4095).ok();
        delay.delay_us(200);
        let high = vc.output_high() as u8;

        // 二分查找翻转点（12 步收敛到 1 LSB）
        let (mut lo, mut hi) = (0u16, 4095u16);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            dac.set_ch1(mid).ok();
            delay.delay_us(100);
            if vc.output_high() {
                hi = mid;
            } else {
                lo = mid;
            }
        }

        // SAFETY: main 独占这些静态变量（单核、无 ISR 访问）
        // volatile 写：防止优化构建把"只写不读"的结果静态量删掉
        unsafe {
            let cross = if high == 1 && low == 0 { hi } else { 0xFFFF };
            core::ptr::write_volatile(core::ptr::addr_of_mut!(CROSS), cross);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(LOW_FLV), low);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(HIGH_FLV), high);
            let v = core::ptr::read_volatile(core::ptr::addr_of!(RUNS));
            core::ptr::write_volatile(core::ptr::addr_of_mut!(RUNS), v + 1);
        }

        delay.delay_ms(500);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
