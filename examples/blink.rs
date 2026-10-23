//! LED 闪烁示例（默认复位时钟：HSI 4MHz）
//!
//! LED 引脚按 PA05 配置，请根据实际开发板原理图修改
//! `PortId` 与引脚号。

#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
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

    // 复位默认时钟：SysClk = HSI = 4MHz，HCLK = PCLK = 4MHz
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    // PA05 推挽输出（按板卡调整）
    let mut led = Pin::new(p.gpioa, PortId::A, 5, &mut rcc).into_output(Level::Low);

    let mut delay = Delay::new(&clocks);

    loop {
        led.toggle();
        delay.delay_ms(500);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
