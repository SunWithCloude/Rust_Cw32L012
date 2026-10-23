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
const COLOR_RED: u16 = 0xF800;
const COLOR_BLACK: u16 = 0x0000;

fn lcd_spi_write(scl: &mut Output<'_>, sda: &mut Output<'_>, dat: u8) {
    for bit in (0..8).rev() {
        if (dat & (1 << bit)) != 0 {
            sda.set_high();
        } else {
            sda.set_low();
        }
        scl.set_high();
        scl.set_low();
    }
}

fn lcd_cmd(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    cmd: u8,
) {
    cs.set_low();
    dc.set_low();
    lcd_spi_write(scl, sda, cmd);
    cs.set_high();
}

fn lcd_data(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    dat: u8,
) {
    cs.set_low();
    dc.set_high();
    lcd_spi_write(scl, sda, dat);
    cs.set_high();
}

fn lcd_data16(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    dat: u16,
) {
    cs.set_low();
    dc.set_high();
    lcd_spi_write(scl, sda, (dat >> 8) as u8);
    lcd_spi_write(scl, sda, (dat & 0xFF) as u8);
    cs.set_high();
}

fn lcd_set_window(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    x1: u16,
    y1: u16,
    x2: u16,
    y2: u16,
) {
    lcd_cmd(cs, dc, scl, sda, 0x2A);
    lcd_data16(cs, dc, scl, sda, x1);
    lcd_data16(cs, dc, scl, sda, x2);
    lcd_cmd(cs, dc, scl, sda, 0x2B);
    lcd_data16(cs, dc, scl, sda, y1);
    lcd_data16(cs, dc, scl, sda, y2);
}

fn lcd_clear(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    color: u16,
) {
    lcd_set_window(cs, dc, scl, sda, 0, 0, LCD_WIDTH - 1, LCD_HEIGHT - 1);
    lcd_cmd(cs, dc, scl, sda, 0x2C);
    for _ in 0..(LCD_WIDTH as u32 * LCD_HEIGHT as u32) {
        lcd_data16(cs, dc, scl, sda, color);
    }
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
    lcd_set_window(cs, dc, scl, sda, x, y, x, y);
    lcd_cmd(cs, dc, scl, sda, 0x2C);
    lcd_data16(cs, dc, scl, sda, color);
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
    let mut cursor_x = x;
    for ch in text.chars() {
        draw_char(cs, dc, scl, sda, cursor_x, y, ch, color);
        cursor_x += 14;
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
    delay.delay_ms(10);
    res.set_high();
    delay.delay_ms(10);

    lcd_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x01);
    delay.delay_ms(120);
    lcd_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x11);
    delay.delay_ms(120);
    lcd_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x36);
    lcd_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x00);
    lcd_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x3A);
    lcd_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x05);
    lcd_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x21);
    lcd_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0xB2);
    lcd_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x0C);
    lcd_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x0C);
    lcd_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x00);
    lcd_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x33);
    lcd_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x33);
    lcd_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0xB7);
    lcd_data(&mut cs, &mut dc, &mut scl, &mut sda, 0x35);
    lcd_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x13);
    lcd_cmd(&mut cs, &mut dc, &mut scl, &mut sda, 0x29);

    bl.set_high();
    lcd_clear(&mut cs, &mut dc, &mut scl, &mut sda, COLOR_RED);
    draw_string(
        &mut cs,
        &mut dc,
        &mut scl,
        &mut sda,
        20,
        80,
        "Redsun",
        COLOR_BLACK,
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
