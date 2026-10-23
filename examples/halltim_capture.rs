//! HALLTIM 自测：24bit 计数 + PWM 输出 + 双次软件捕获（v0.6 验证例）
//!
//! 零外部接线。计数时钟 PCLK/2 = 2MHz，ARR=19999 → **100Hz PWM、10ms 周期**，
//! 占空比 30%（CCR=6000）。
//!
//! 自检序列（每次都从"捕获后 CNT=0"这个硬件行为出发）：
//! 1. `SOFTCAP` 软件捕获 → 硬件把 CNT 存入 WIDTH 并**清零 CNT**；
//! 2. 立即读 CNT ≈ 0（只有捕获后几微秒的增量）；
//! 3. 延时 2ms → CNT ≈ 2ms × 2MHz = **4000**（绝对参照）；
//! 4. 再捕获一次 → WIDTH ≈ 步骤 3 的 CNT，CNT 再次归零。
//!
//! 判定：`W2 ≈ 4000`、`W1` 小、`CNT_AFTER` 小。若计数/捕获/WIDTH 任一环节
//! 失效，这组关系都无法同时成立。
//!
//! 静态量：`W1` / `CNT_MID` / `W2` / `CNT_AFTER` / `ARR` / `CCR` / `ROUNDS`

#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Af, Level, Pin, PortId},
    halltim::{HallDiv, Halltim},
    rcc::Rcc,
    Peripherals,
};

/// 第一次捕获（捕获前 CNT 未知，只要求它是小值：捕获后立即清零）
static mut W1: u32 = 0;
/// 延时 2ms 后的计数值（参照量，应 ≈4000）
static mut CNT_MID: u32 = 0;
/// 第二次捕获得到的脉宽（应 ≈CNT_MID）
static mut W2: u32 = 0;
/// 第二次捕获后的计数值（应 ≈0）
static mut CNT_AFTER: u32 = 0;
/// ARR 回读（应恒为 19999）
static mut ARR: u32 = 0;
/// CCR 回读（应恒为 6000）
static mut CCR: u32 = 0;
/// 完成的捕获轮数（持续递增 = 固件活着）
static mut ROUNDS: u32 = 0;

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
    // PB00 = HALLTIM_OUT（表 9-2，AF9）
    let _out = Pin::new(p.gpiob, PortId::B, 0, &mut rcc).into_af_output(Af::Af9);

    let mut hall = Halltim::new(p.halltim, &mut rcc, HallDiv::Div2); // 2MHz
    hall.set_period(20_000 - 1).ok(); // 2MHz / 20000 = 100Hz
    hall.set_duty_permille(300); // CCR = 6000
    hall.enable();
    delay.delay_ms(5);

    loop {
        // 1) 首次捕获：把当前 CNT 存入 WIDTH 并清零
        hall.software_capture();
        let w1 = hall.pulse_width();

        // 2) 跑满 2ms（2MHz × 2ms = 4000 计数，远小于 ARR 不溢出）
        delay.delay_ms(2);
        let mid = hall.count();

        // 3) 再次捕获 → WIDTH 应等于刚才的 CNT
        hall.software_capture();
        let w2 = hall.pulse_width();
        let after = hall.count();
        hall.clear_irq(cw32l012_hal::pac::halltim::CAPF);

        // 4) 寄存器回读
        let regs = hall.regs();
        let arr = regs.arr.read() & cw32l012_hal::pac::halltim::DATA_MASK;
        let ccr = regs.ccr.read() & cw32l012_hal::pac::halltim::DATA_MASK;

        // SAFETY: main 独占这些静态变量（单核、无 ISR 访问）
        // volatile 写：防止优化构建把"只写不读"的测量静态量整段删掉
        unsafe {
            core::ptr::write_volatile(core::ptr::addr_of_mut!(W1), w1);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(CNT_MID), mid);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(W2), w2);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(CNT_AFTER), after);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(ARR), arr);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(CCR), ccr);
            let v = core::ptr::read_volatile(core::ptr::addr_of!(ROUNDS));
            core::ptr::write_volatile(core::ptr::addr_of_mut!(ROUNDS), v + 1);
        }

        delay.delay_ms(200);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
