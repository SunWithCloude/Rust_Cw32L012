//! UART1 回显示例（PA02=TXD、PA03=RXD，AF1，115200-8-N-1）
//!
//! 接线：MCU PA02 -> USB 串口适配器 RXD，
//!       MCU PA03 <- USB 串口适配器 TXD，共地。
//! 复位后发送欢迎消息，随后将收到的每个字节原样回显。

#![no_std]
#![no_main]

use cw32l012_hal::{
    gpio::{Af, Pin, PortId},
    rcc::Rcc,
    uart::Uart,
    Peripherals,
};

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

    // UART1 引脚：PA02 = UART1_TXD，PA03 = UART1_RXD（AF1）
    let tx = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_af_output(Af::Af1);
    let rx = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_af_input(Af::Af1);

    let mut uart = Uart::new(p.uart1, tx, rx, &mut rcc, clocks.pclk_hz, 115_200);

    uart.write_str("CW32L012 Rust HAL ready\r\n");

    loop {
        if let Some(b) = uart.try_read_byte() {
            uart.write_byte(b);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
