//! UART 对夹验证（PA02 = TXD、PA03 = RXD ↔ CH340X USB-TTL）
//!
//! 接线（3 根线，务必共地）：
//!
//! ```text
//! CH340X TXD  ──►  MCU PA03 (RXD)
//! CH340X RXD  ◄──  MCU PA02 (TXD)
//! CH340X GND  ───  MCU GND
//! ```
//!
//! 行为：上电发一行 banner，随后**逐字节回显**，同时把统计量写进 SRAM：
//! 波特率分频寄存器实测值、收/发字节数、接收错误次数、最后一字节。
//!
//! 上位机侧脚本：`tools/uart_link_test.py`（发固定 pattern、逐字节比对回显、
//! 再用 probe-rs 读回统计量）。

#![no_std]
#![no_main]

use cw32l012_hal::{
    gpio::{Af, Pin, PortId},
    rcc::Rcc,
    uart::Uart,
    Peripherals,
};

/// 观测块（`repr(C)` 保证布局固定，上位机按 6 个 u32 解码）
#[repr(C)]
struct Obs {
    /// 魔数，确认读到的是这个块而不是残留内存
    magic: u32,
    /// BRRI 回读（波特率整数分频）
    brri: u32,
    /// BRRF 回读（波特率小数分频）
    brrf: u32,
    /// 累计收到的字节数
    rx: u32,
    /// 累计回显的字节数
    tx: u32,
    /// 接收错误标志命中次数
    err: u32,
    /// 最后一字节（u8 放低 8 位）
    last: u32,
}

static mut OBS: Obs = Obs {
    magic: 0,
    brri: 0,
    brrf: 0,
    rx: 0,
    tx: 0,
    err: 0,
    last: 0,
};

const MAGIC: u32 = 0x5550_5254; // "UPRT"

/// 并发更新观测块（单线程主循环，直接写即可）
fn obs(f: impl FnOnce(&mut Obs)) {
    // SAFETY: 只有主循环写这个静态量，没有中断上下文访问
    unsafe {
        let p = core::ptr::addr_of_mut!(OBS);
        let mut v = core::ptr::read_volatile(p);
        f(&mut v);
        core::ptr::write_volatile(p, v);
    }
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
    let clocks = rcc.clocks(); // PCLK = 4MHz

    // UART1：PA02 = TXD、PA03 = RXD（AF1）
    let tx = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_af_output(Af::Af1);
    let rx = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_af_input(Af::Af1);
    let mut uart = Uart::new(p.uart1, tx, rx, &mut rcc, clocks.pclk_hz, 115_200);

    // 记录实际写进寄存器的波特率分频（4MHz / 115200 → BRRI=34、BRRF≈9）
    let (brri, brrf) = {
        let r = uart.regs();
        (r.brri.read() & 0xFFFF, r.brrf.read() & 0xF)
    };
    obs(|o| {
        o.magic = MAGIC;
        o.brri = brri;
        o.brrf = brrf;
    });

    uart.write_str("CW32L012 UART LINK OK\r\n");

    loop {
        // 先看错误标志（try_read_byte 内部会静默清掉它们）
        if uart.error_flags() != 0 {
            obs(|o| o.err += 1);
            uart.clear_errors();
        }
        if let Some(b) = uart.try_read_byte() {
            obs(|o| {
                o.rx += 1;
                o.last = b as u32;
            });
            uart.write_byte(b);
            obs(|o| o.tx += 1);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
