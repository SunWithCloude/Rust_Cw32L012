//! 生态对接验证（v0.9 新增）：critical-section / embedded-io / PWM trait
//!
//! 一次上电验证三件事，全部落在 SRAM 里供 `probe-rs read` 读取：
//!
//! 1. **critical-section**（feature `critical-section-impl`）：进出临界区前后的
//!    `PRIMASK` 必须是 0 → 1 → 0，嵌套时内层仍为 1（不能重复开中断）。
//!    这直接验证 `cpsid i` / `mrs PRIMASK` 内联汇编的实际效果。
//! 2. **`embedded_hal::pwm::SetDutyCycle`**：满量程 10000，设 25% 读回 ≈2500，
//!    设 `u16::MAX` 被夹到 100%（10000）。
//! 3. **`embedded_io::Write`**：向 UART1（PA02/PA03，115200）写一串字节，
//!    返回的字节数必须等于长度。TX 没接东西也无所谓——验证的是 trait 通路。
//!
//! 观测地址见 `nm` 的 `[bB]` 段符号；数值语义见 TESTING.md 的 `ecosystem` 节。

#![no_std]
#![no_main]

use core::arch::asm;

use cw32l012_hal::{
    gpio::{Af, Pin, PortId},
    gtim::{Gtim, PwmChannel},
    rcc::Rcc,
    uart::Uart,
    Peripherals,
};
use embedded_hal::pwm::SetDutyCycle;
use embedded_io::Write;

/// 进临界区前的 PRIMASK（预期 0）
static mut CS_BEFORE: u32 = 0xDEAD_BEEF;
/// 第一层临界区内（预期 1）
static mut CS_INSIDE: u32 = 0xDEAD_BEEF;
/// 第二层（嵌套）临界区内（预期仍是 1）
static mut CS_NESTED: u32 = 0xDEAD_BEEF;
/// 出临界区后（预期 0）
static mut CS_AFTER: u32 = 0xDEAD_BEEF;
/// `max_duty_cycle()` 返回值（预期 10000）
static mut PWM_MAX: u16 = 0xDEAD;
/// 设 25% 后读回的占空比（预期 ≈2500）
static mut PWM_25PCT: u16 = 0xDEAD;
/// 设 `u16::MAX` 后读回的占空比（预期 10000，被夹紧）
static mut PWM_CLAMP: u16 = 0xDEAD;
/// `embedded_io::Write::write` 返回的字节数（预期 16）
static mut IO_WRITTEN: u16 = 0xDEAD;
/// 里程碑：每完成一项 +1，用于判断卡在哪一步
static mut STAGE: u32 = 0;

/// 读 PRIMASK（0 = 中断开启）
#[inline]
fn primask() -> u32 {
    let v: u32;
    unsafe {
        asm!("mrs {}, PRIMASK", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}

macro_rules! record {
    ($slot:ident, $ty:ty, $val:expr) => {{
        let v: $ty = $val;
        unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!($slot), v) };
    }};
}

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

    // ---- 1. critical-section：PRIMASK 必须被真正屏蔽并正确恢复 ----
    record!(CS_BEFORE, u32, primask());
    critical_section::with(|_| {
        record!(CS_INSIDE, u32, primask());
        critical_section::with(|_| {
            record!(CS_NESTED, u32, primask());
        });
    });
    record!(CS_AFTER, u32, primask());
    record!(STAGE, u32, 1);

    // ---- 2. embedded_hal::pwm::SetDutyCycle ----
    let _pwm_pin = Pin::new(p.gpiob, PortId::B, 4, &mut rcc).into_af_output(Af::Af6);
    let mut tim = Gtim::new(p.gtim1, &mut rcc);
    tim.start_pwm(PwmChannel::Ch1, clocks.pclk_hz, 1_000, 0)
        .ok();
    {
        let mut pin = tim.pwm_pin(PwmChannel::Ch1);
        record!(PWM_MAX, u16, pin.max_duty_cycle());
        pin.set_duty_cycle(2_500).ok();
        record!(PWM_25PCT, u16, pin.duty());
        pin.set_duty_cycle(u16::MAX).ok();
        record!(PWM_CLAMP, u16, pin.duty());
    }
    record!(STAGE, u32, 2);

    // ---- 3. embedded_io::Write（PA02=TX，115200）----
    let tx = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_af_output(Af::Af1);
    let rx = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_af_input(Af::Af1);
    let mut uart = Uart::new(p.uart1, tx, rx, &mut rcc, clocks.pclk_hz, 115_200);
    let payload = b"cw32 eio ok\r\n";
    let n = uart.write(payload).unwrap_or(0);
    // 显式走 trait（本体也有同名 flush，返回 ()）
    Write::flush(&mut uart).ok();
    record!(IO_WRITTEN, u16, n as u16);
    record!(STAGE, u32, 3);

    // 收尾自旋（禁止 wfi：会让 SWD 掉线）
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
