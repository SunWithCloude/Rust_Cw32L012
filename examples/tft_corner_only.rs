#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Output, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

const TFT_WIDTH: u16 = 240;
const TFT_HEIGHT: u16 = 280;
const RED: u16 = 0xF800;
const WHITE: u16 = 0xFFFF;
const BLUE: u16 = 0x001F;

fn spi_write_byte(scl: &mut Output<'_>, sda: &mut Output<'_>, data: u8) {
    for bit in (0..8).rev() {
        scl.set_low();
        if (data & (1u8 << bit)) != 0 {
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
    data: u8,
) {
    cs.set_low();
    dc.set_high();
    spi_write_byte(scl, sda, data);
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

fn fill_screen(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    color: u16,
) {
    set_window(cs, dc, scl, sda, 0, 0, TFT_WIDTH - 1, TFT_HEIGHT - 1);
    write_cmd(cs, dc, scl, sda, 0x2C);
    cs.set_low();
    dc.set_high();
    for _ in 0..(TFT_WIDTH as u32 * TFT_HEIGHT as u32) {
        spi_write_byte(scl, sda, ((color >> 8) & 0xFF) as u8);
        spi_write_byte(scl, sda, (color & 0xFF) as u8);
    }
    cs.set_high();
}

fn draw_rect(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    color: u16,
) {
    for yy in y..(y + h) {
        for xx in x..(x + w) {
            set_window(cs, dc, scl, sda, xx, yy, xx, yy);
            write_cmd(cs, dc, scl, sda, 0x2C);
            write_u16(cs, dc, scl, sda, color);
        }
    }
}

fn init_with_madctl(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    madctl: u8,
    delay: &mut Delay,
) {
    write_cmd(cs, dc, scl, sda, 0x01);
    delay.delay_ms(120);
    write_cmd(cs, dc, scl, sda, 0x11);
    delay.delay_ms(120);
    write_cmd(cs, dc, scl, sda, 0x36);
    write_data(cs, dc, scl, sda, madctl);
    write_cmd(cs, dc, scl, sda, 0x3A);
    write_data(cs, dc, scl, sda, 0x05);
    write_cmd(cs, dc, scl, sda, 0x29);
    delay.delay_ms(50);
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

    let vals = [0x00u8, 0x60, 0xA0, 0xC0];

    loop {
        for mad in vals {
            res.set_low();
            delay.delay_ms(20);
            res.set_high();
            delay.delay_ms(80);

            init_with_madctl(&mut cs, &mut dc, &mut scl, &mut sda, mad, &mut delay);
            bl.set_high();

            fill_screen(&mut cs, &mut dc, &mut scl, &mut sda, RED);
            draw_rect(&mut cs, &mut dc, &mut scl, &mut sda, 10, 10, 80, 80, WHITE);
            draw_rect(&mut cs, &mut dc, &mut scl, &mut sda, 30, 110, 120, 40, BLUE);

            delay.delay_ms(2000);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
