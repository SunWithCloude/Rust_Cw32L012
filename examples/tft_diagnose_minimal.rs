#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Output, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

const LCD_WIDTH: u16 = 240;
const LCD_HEIGHT: u16 = 280;

fn spi_write_byte(scl: &mut Output<'_>, sda: &mut Output<'_>, byte: u8) {
    for bit in (0..8).rev() {
        scl.set_low();
        if (byte & (1u8 << bit)) != 0 {
            sda.set_high();
        } else {
            sda.set_low();
        }
        for _ in 0..2 {
            core::hint::spin_loop();
        }
        scl.set_high();
        for _ in 0..2 {
            core::hint::spin_loop();
        }
    }
}

fn write_cmd(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    cmd: u8,
) {
    cs.set_low();
    dc.set_low();
    spi_write_byte(scl, sda, cmd);
    cs.set_high();
}

fn write_data(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    dat: u8,
) {
    cs.set_low();
    dc.set_high();
    spi_write_byte(scl, sda, dat);
    cs.set_high();
}

fn write_u16(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    val: u16,
) {
    write_data(cs, dc, scl, sda, ((val >> 8) & 0xFF) as u8);
    write_data(cs, dc, scl, sda, (val & 0xFF) as u8);
}

fn set_window(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
) {
    write_cmd(cs, dc, scl, sda, 0x2A);
    write_u16(cs, dc, scl, sda, x0);
    write_u16(cs, dc, scl, sda, x1);
    write_cmd(cs, dc, scl, sda, 0x2B);
    write_u16(cs, dc, scl, sda, y0);
    write_u16(cs, dc, scl, sda, y1);
}

fn fill_red(cs: &mut Output<'_>, dc: &mut Output<'_>, scl: &mut Output<'_>, sda: &mut Output<'_>) {
    set_window(
        cs,
        dc,
        scl,
        sda,
        0,
        0,
        (LCD_WIDTH - 1) as u16,
        (LCD_HEIGHT - 1) as u16,
    );
    write_cmd(cs, dc, scl, sda, 0x2C);
    cs.set_low();
    dc.set_high();
    for _ in 0..(LCD_WIDTH as u32 * LCD_HEIGHT as u32) {
        spi_write_byte(scl, sda, 0xF8);
        spi_write_byte(scl, sda, 0x00);
    }
    cs.set_high();
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
    let mut delay = Delay::new(&clocks);

    let mut bl = Pin::new(p.gpiob, PortId::B, 4, &mut rcc).into_output(Level::Low);
    let mut cs = Pin::new(p.gpiob, PortId::B, 5, &mut rcc).into_output(Level::High);
    let mut dc = Pin::new(p.gpiob, PortId::B, 6, &mut rcc).into_output(Level::Low);
    let mut res = Pin::new(p.gpiob, PortId::B, 7, &mut rcc).into_output(Level::High);
    let mut sda = Pin::new(p.gpiob, PortId::B, 8, &mut rcc).into_output(Level::Low);
    let mut scl = Pin::new(p.gpiob, PortId::B, 9, &mut rcc).into_output(Level::High);

    cs.set_high();
    scl.set_low();
    sda.set_low();

    // 检查是否真的能做 RES 脉冲
    res.set_low();
    delay.delay_ms(50);
    res.set_high();
    delay.delay_ms(100);

    bl.set_high();

    // 最小 init: 只发几条命令，不打字，不填复杂内容
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x01);
    delay.delay_ms(120);
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x11);
    delay.delay_ms(120);
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x36);
    write_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x00);
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x3A);
    write_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x05);
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x29);
    delay.delay_ms(50);

    fill_red(&mut cs, &mut dc, &mut scl, &mut sda);

    loop {
        delay.delay_ms(1000);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
