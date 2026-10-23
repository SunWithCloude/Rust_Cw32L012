//! FLASH 自编程自测：掉电保持的启动计数器（零接线）
//!
//! 在第 127 页（`0x0000_FE00`，页大小 512B）存一条 12 字节记录：
//! `[magic, count, ~count]`。每次复位读出来 +1 再写回去，**复位不清零**，
//! 这就是最朴素的参数存储/NVM 用法。
//!
//! 相位：
//! 1. 读旧值（首次上电应为 `0xFFFF_FFFF`）；
//! 2. 擦除整页，读回首字验证**确实变成 0xFF**；
//! 3. 编程 3 个字；
//! 4. 回读校验（magic / count / 反码）；
//! 5. 记录 `_etext`（代码区上界，`erase_page` 用它挡误擦）。
//!
//! 观察量：`OLD_MAGIC`、`ERASED`、`COUNT`、`RB_*`、`OK`、`ERR`、`ETEXT`。

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};

use cw32l012_hal::{
    delay::Delay,
    flash::{self, Flash},
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

macro_rules! put {
    ($p:expr, $v:expr) => {
        unsafe { write_volatile($p, $v) }
    };
}

/// 记录所在页
const PAGE: u8 = 127;
/// 记录地址
const ADDR: u32 = (PAGE as u32) * 512;
/// 魔数 "FLHC"
const MAGIC: u32 = 0x4348_4C46;

static mut OLD_MAGIC: u32 = 0;
static mut OLD_COUNT: u32 = 0;
static mut ERASED: u32 = 0;
static mut NEW_COUNT: u32 = 0;
static mut RB_MAGIC: u32 = 0;
static mut RB_COUNT: u32 = 0;
static mut RB_CHECK: u32 = 0;
static mut OK: u32 = 0;
static mut ERR: u32 = 0;
static mut ETEXT: u32 = 0;
static mut SECURITY: u32 = 0;

#[no_mangle]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let mut delay = Delay::new(&rcc.clocks());
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);
    let mut nvm = Flash::new(p.flashctrl, &mut rcc);

    put!(addr_of_mut!(ETEXT), flash::code_end());
    put!(addr_of_mut!(SECURITY), nvm.security_level() as u32);

    // ---- 1. 读旧记录 ----
    let old_magic = Flash::read_u32(ADDR);
    let old_count = Flash::read_u32(ADDR + 4);
    let old_check = Flash::read_u32(ADDR + 8);
    put!(addr_of_mut!(OLD_MAGIC), old_magic);
    put!(addr_of_mut!(OLD_COUNT), old_count);

    let valid = old_magic == MAGIC && old_check == !old_count;
    let count = if valid && old_count < 0xFFFF_0000 {
        old_count + 1
    } else {
        1
    };
    put!(addr_of_mut!(NEW_COUNT), count);

    // ---- 2~4. 擦除 → 编程 → 回读 ----
    let mut err = 0u32;
    let mut buf = [0u8; 12];
    buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    buf[4..8].copy_from_slice(&count.to_le_bytes());
    buf[8..12].copy_from_slice(&(!count).to_le_bytes());

    // SAFETY: 第 127 页（0xFE00）在代码区之外（见 ETEXT），且本程序独占该页
    let erased = unsafe {
        if nvm.erase_page(PAGE).is_err() {
            err = 1;
        }
        let e = Flash::read_u32(ADDR);
        if err == 0 && nvm.program(ADDR, &buf).is_err() {
            err = 2;
        }
        e
    };
    put!(addr_of_mut!(ERASED), erased);

    let rb_magic = Flash::read_u32(ADDR);
    let rb_count = Flash::read_u32(ADDR + 4);
    let rb_check = Flash::read_u32(ADDR + 8);
    put!(addr_of_mut!(RB_MAGIC), rb_magic);
    put!(addr_of_mut!(RB_COUNT), rb_count);
    put!(addr_of_mut!(RB_CHECK), rb_check);

    let ok = err == 0 && rb_magic == MAGIC && rb_count == count && rb_check == !count;
    put!(addr_of_mut!(OK), ok as u32);
    put!(addr_of_mut!(ERR), err);

    // 写成功亮灯 3 下，失败常亮
    if ok {
        for _ in 0..3 {
            led.toggle();
            delay.delay_ms(120);
            led.toggle();
            delay.delay_ms(120);
        }
    } else {
        led.set_high();
    }

    loop {
        delay.delay_ms(1000);
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
