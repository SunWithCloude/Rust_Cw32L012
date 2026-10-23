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

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    let mut led1 = Pin::new(p.gpiob, PortId::B, 9, &mut rcc).into_output(Level::Low);
    let mut led2 = Pin::new(p.gpiob, PortId::B, 8, &mut rcc).into_output(Level::Low);
    let mut delay = Delay::new(&clocks);

    loop {
        led1.set_high();
        led2.set_low();
        delay.delay_ms(250);

        led1.set_low();
        led2.set_high();
        delay.delay_ms(250);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
