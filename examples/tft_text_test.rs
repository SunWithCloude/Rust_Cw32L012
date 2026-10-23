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
const RGB565_RED: u16 = 0xF800;
const RGB565_BLACK: u16 = 0x0000;
#[allow(dead_code)]
const RGB565_WHITE: u16 = 0xFFFF;

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

fn draw_pixel(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    x: u16,
    y: u16,
    color: u16,
) {
    set_window(cs, dc, scl, sda, x, y, x, y);
    write_cmd(cs, dc, scl, sda, 0x2C);
    write_u16(cs, dc, scl, sda, color);
}

fn draw_char(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    x: u16,
    y: u16,
    ch: char,
    color: u16,
) {
    let glyph = match ch {
        'R' => [0x7F, 0x09, 0x19, 0x29, 0x49, 0x7F, 0x00],
        'E' => [0x7F, 0x49, 0x49, 0x49, 0x41, 0x00, 0x00],
        'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C, 0x00, 0x00],
        'S' => [0x49, 0x49, 0x49, 0x49, 0x36, 0x00, 0x00],
        'U' => [0x3F, 0x40, 0x40, 0x40, 0x3F, 0x00, 0x00],
        'N' => [0x7F, 0x01, 0x02, 0x04, 0x7F, 0x00, 0x00],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    };

    for row in 0..7 {
        let bits = glyph[row];
        for col in 0..7 {
            if (bits & (1 << col)) != 0 {
                let px = x + (col as u16 * 2);
                let py = y + (row as u16 * 2);
                draw_pixel(cs, dc, scl, sda, px, py, color);
                draw_pixel(cs, dc, scl, sda, px + 1, py, color);
                draw_pixel(cs, dc, scl, sda, px, py + 1, color);
                draw_pixel(cs, dc, scl, sda, px + 1, py + 1, color);
            }
        }
    }
}

fn draw_string(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    x: u16,
    y: u16,
    text: &str,
    color: u16,
) {
    let mut x_cursor = x;
    for ch in text.chars() {
        draw_char(cs, dc, scl, sda, x_cursor, y, ch, color);
        x_cursor += 14;
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

    res.set_low();
    delay.delay_ms(20);
    res.set_high();
    delay.delay_ms(100);

    // 先试 MADCTL=0x00
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x01);
    delay.delay_ms(120);
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x11);
    delay.delay_ms(120);
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x36);
    write_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x00);
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x3A);
    write_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x05);
    write_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x29);

    bl.set_high();
    fill_screen(&mut cs, &mut dc, &mut scl, &mut sda, RGB565_RED);
    draw_string(
        &mut cs,
        &mut dc,
        &mut scl,
        &mut sda,
        20,
        80,
        "Redsun",
        RGB565_BLACK,
    );

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
