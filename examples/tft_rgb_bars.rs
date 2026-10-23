#![no_std]
#![no_main]

//! ST7789V3 240x280 彩色竖条诊断
//!
//! 目的：一次上板同时区分三类问题
//!   1) 窗口偏移（240x280 面板 RAM 是 240x320，可见区从 Y=20 开始）
//!   2) RGB / BGR 顺序（MADCTL bit3）
//!   3) 画面方向 / 镜像（MADCTL bit7/6）
//!
//! 显示内容：8 条竖条
//!   白 黄 青 绿 品红 红 蓝 黑
//! 判读方法：
//!   - 若「红」「蓝」位置互换、黄↔青互换 → BGR 位设错，改 BGR 常量
//!   - 若顶部/底部少一条边、内容整体上移或下移 → OFFSET_Y 设错
//!   - 若画面左右镜像或上下颠倒 → 改 MADCTL
//!
//! 调参只需改下面 3 个常量，然后重新编译烧录。

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Output, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

// ================= 可调参数 =================
/// MADCTL 基础值：0x00=正常，0x60=横屏(交换XY)，0xA0/0xC0=翻转
const MADCTL_BASE: u8 = 0x00;
/// BGR 位：面板是 BGR 顺序则置 true（0x08）。红蓝互换就改这里。
const MADCTL_BGR: bool = false;
/// 240x280 面板的 Y 偏移，通常是 20；若整屏 240x320 则填 0。
const OFFSET_Y: u16 = 20;
/// X 偏移，通常 0。
const OFFSET_X: u16 = 0;
// ===========================================

const LCD_W: u16 = 240;
const LCD_H: u16 = 280;

// RGB565 常用色
const WHITE: u16 = 0xFFFF;
const YELLOW: u16 = 0xFFE0;
const CYAN: u16 = 0x07FF;
const GREEN: u16 = 0x07E0;
const MAGENTA: u16 = 0xF81F;
const RED: u16 = 0xF800;
const BLUE: u16 = 0x001F;
const BLACK: u16 = 0x0000;

fn spi_write_byte(scl: &mut Output<'_>, sda: &mut Output<'_>, data: u8) {
    for bit in (0..8).rev() {
        scl.set_low();
        if (data & (1u8 << bit)) != 0 {
            sda.set_high();
        } else {
            sda.set_low();
        }
        for _ in 0..3 {
            core::hint::spin_loop();
        }
        scl.set_high();
        for _ in 0..3 {
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

/// 设置绘制窗口（自动加面板偏移）
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
    let xs = x0 + OFFSET_X;
    let xe = x1 + OFFSET_X;
    let ys = y0 + OFFSET_Y;
    let ye = y1 + OFFSET_Y;

    write_cmd(cs, dc, scl, sda, 0x2A); // CASET
    write_u16(cs, dc, scl, sda, xs);
    write_u16(cs, dc, scl, sda, xe);
    write_cmd(cs, dc, scl, sda, 0x2B); // RASET
    write_u16(cs, dc, scl, sda, ys);
    write_u16(cs, dc, scl, sda, ye);
    write_cmd(cs, dc, scl, sda, 0x2C); // RAMWR
}

/// 画一根竖条 x∈[x0,x1]
fn vbar(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    x0: u16,
    x1: u16,
    color: u16,
) {
    set_window(cs, dc, scl, sda, x0, 0, x1, LCD_H - 1);
    cs.set_low();
    dc.set_high();
    let pixels = (x1 - x0 + 1) as u32 * LCD_H as u32;
    for _ in 0..pixels {
        spi_write_byte(scl, sda, ((color >> 8) & 0xFF) as u8);
        spi_write_byte(scl, sda, (color & 0xFF) as u8);
    }
    cs.set_high();
}

fn init(
    cs: &mut Output<'_>,
    dc: &mut Output<'_>,
    scl: &mut Output<'_>,
    sda: &mut Output<'_>,
    delay: &mut Delay,
) {
    let madctl = MADCTL_BASE | if MADCTL_BGR { 0x08 } else { 0x00 };

    write_cmd(cs, dc, scl, sda, 0x01); // SWRESET
    delay.delay_ms(150);
    write_cmd(cs, dc, scl, sda, 0x11); // SLPOUT
    delay.delay_ms(150);

    write_cmd(cs, dc, scl, sda, 0x36); // MADCTL
    write_data(cs, dc, scl, sda, madctl);
    write_cmd(cs, dc, scl, sda, 0x3A); // COLMOD = 16bit
    write_data(cs, dc, scl, sda, 0x05);
    write_cmd(cs, dc, scl, sda, 0x21); // INVON（多数 ST7789 模块需开启反显）
    write_cmd(cs, dc, scl, sda, 0x29); // DISPON
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

    // 硬复位
    res.set_low();
    delay.delay_ms(20);
    res.set_high();
    delay.delay_ms(120);

    bl.set_high(); // 先开背光，方便观察

    init(&mut cs, &mut dc, &mut scl, &mut sda, &mut delay);

    // 8 条竖条，每条 30px
    let bars = [WHITE, YELLOW, CYAN, GREEN, MAGENTA, RED, BLUE, BLACK];
    let bar_w = LCD_W / 8; // 30
    for (i, c) in bars.iter().enumerate() {
        let x0 = i as u16 * bar_w;
        let x1 = if i == 7 { LCD_W - 1 } else { x0 + bar_w - 1 };
        vbar(&mut cs, &mut dc, &mut scl, &mut sda, x0, x1, *c);
    }

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
