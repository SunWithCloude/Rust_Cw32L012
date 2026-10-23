//! ST7789 硬件 SPI 彩条测试（旗舰例程：v0.3 新增的 [`spi`] 模块实机验证）
//!
//! 接线与 cw32l012-toolbox 的 colorbars 例程完全一致——**无需改线**：
//!
//! | CW32L012 | ST7789V3 | 说明 |
//! |---|---|---|
//! | PA5  | SCL/SCK  | SPI1_SCK（AF3，硬件时钟！） |
//! | PA7  | SDA/MOSI | SPI1_MOSI（AF3，硬件数据！） |
//! | PA3  | RES      | 复位 |
//! | PA1  | DC       | 命令/数据 |
//! | PA2  | CS       | 片选（init 后保持低） |
//! | PA4  | BLK      | 背光 |
//!
//! 主频拉到 96MHz（`run_hsi_at_96mhz`，FLASH 等待周期自动同步），
//! SPI 跑 PCLK/4=24MHz——相比 4MHz 软 SPI 位敲，整屏绘制从约 2 分钟
//! 缩短到亚秒级。面板默认 1.69" 240×280（y 偏移 20）。

#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Af, Level, Output, Pin, PortId},
    rcc::Rcc,
    spi::{Spi, SpiConfig},
    Peripherals,
};

// ST7789 命令
const SWRESET: u8 = 0x01;
const SLPOUT: u8 = 0x11;
const INVON: u8 = 0x21;
const NORON: u8 = 0x13;
const DISPON: u8 = 0x29;
const CASET: u8 = 0x2A;
const RASET: u8 = 0x2B;
const RAMWR: u8 = 0x2C;
const MADCTL: u8 = 0x36;
const COLMOD: u8 = 0x3A;

// RGB565
const BLACK: u16 = 0x0000;
const RED: u16 = 0xF800;
const GREEN: u16 = 0x07E0;
const BLUE: u16 = 0x001F;
const WHITE: u16 = 0xFFFF;

const W: u16 = 240;
const H: u16 = 280;
const ROW_OFF: u16 = 20; // 1.69" 面板

struct Lcd<'p> {
    spi: Spi<'p>,
    dc: Output<'p>,
    cs: Output<'p>,
}

impl<'p> Lcd<'p> {
    /// 写命令（DC=0，单字节；保持 CS 低期间只发这一帧）
    fn cmd(&mut self, c: u8) {
        self.dc.set_low();
        self.spi.write(&[c]);
    }

    /// 写命令 + 参数（参数段 DC=1）
    fn cmd_param(&mut self, c: u8, params: &[u8]) {
        self.cmd(c);
        self.dc.set_high();
        self.spi.write(params);
    }

    /// 复位 → 唤醒 → 配置 → 清屏 → 点背光（时序复刻 toolbox 驱动）
    fn init(&mut self, res: &mut Output<'p>, bl: &mut Output<'p>, delay: &mut Delay) {
        bl.set_low();
        self.cs.set_high();
        res.set_high();
        delay.delay_ms(10);
        res.set_low();
        delay.delay_ms(20);
        res.set_high();
        delay.delay_ms(150);

        self.cs.set_low(); // 单从机总线：此后 CS 保持低
        self.cmd(SWRESET);
        delay.delay_ms(150);
        self.cmd(SLPOUT);
        delay.delay_ms(120);
        self.cmd_param(COLMOD, &[0x55]); // 16bit RGB565
        delay.delay_ms(10);
        self.cmd_param(MADCTL, &[0x00]);
        self.cmd(INVON); // IPS 正色；TFT 面板发白改 INVOFF(0x20)
        self.cmd(NORON);
        delay.delay_ms(10);
        self.fill_screen(BLACK);
        self.cmd(DISPON);
        delay.delay_ms(50);
        bl.set_high();
    }

    /// 设写入窗口并进入 RAMWR
    fn set_window(&mut self, x: u16, y: u16, w: u16, h: u16) {
        let (x0, x1) = (x, x + w - 1);
        let (y0, y1) = (ROW_OFF + y, ROW_OFF + y + h - 1);
        self.cmd_param(CASET, &[hi(x0), lo(x0), hi(x1), lo(x1)]);
        self.cmd_param(RASET, &[hi(y0), lo(y0), hi(y1), lo(y1)]);
        self.cmd(RAMWR);
    }

    /// 填充矩形（64B 同色块流式发送）
    fn fill_rect(&mut self, x: u16, y: u16, w: u16, h: u16, color: u16) {
        self.set_window(x, y, w, h);
        self.dc.set_high();
        let (chi, clo) = ((color >> 8) as u8, (color & 0xFF) as u8);
        let mut chunk = [0u8; 64]; // 32 像素/块
        for pair in chunk.chunks_exact_mut(2) {
            pair[0] = chi;
            pair[1] = clo;
        }
        let total = w as u32 * h as u32;
        let mut left = total;
        while left > 0 {
            let n = (32.min(left)) as usize;
            self.spi.write(&chunk[..n * 2]);
            left -= n as u32;
        }
    }

    fn fill_screen(&mut self, color: u16) {
        self.fill_rect(0, 0, W, H, color);
    }
}

const fn hi(v: u16) -> u8 {
    (v >> 8) as u8
}
const fn lo(v: u16) -> u8 {
    (v & 0xFF) as u8
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
    rcc.run_hsi_at_96mhz(); // 96MHz：软 SPI 时代 2 分钟的活，现在毫秒级
    let clocks = rcc.clocks();

    let mut res = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_output(Level::High);
    let dc = Pin::new(p.gpioa, PortId::A, 1, &mut rcc).into_output(Level::Low);
    let cs = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_output(Level::High);
    let mut bl = Pin::new(p.gpioa, PortId::A, 4, &mut rcc).into_output(Level::Low);

    // SPI1 硬件外设：PA5=SCK/AF3、PA7=MOSI/AF3，目标 24MHz（自动取不超档）
    let sck = Pin::new(p.gpioa, PortId::A, 5, &mut rcc).into_af_output(Af::Af3);
    let mosi = Pin::new(p.gpioa, PortId::A, 7, &mut rcc).into_af_output(Af::Af3);
    let cfg = SpiConfig::default().with_baud_closest(clocks.pclk_hz, 24_000_000);
    let spi = Spi::new_tx_only(p.spi1, sck, mosi, &mut rcc, cfg);

    let mut delay = Delay::new(&clocks);
    let mut lcd = Lcd { spi, dc, cs };
    lcd.init(&mut res, &mut bl, &mut delay);

    // 彩条：红/绿/蓝/白四横带 + 黑色分隔线 + 白色外框
    let band_h = H / 4;
    lcd.fill_rect(0, 0, W, band_h, RED);
    lcd.fill_rect(0, band_h, W, band_h, GREEN);
    lcd.fill_rect(0, 2 * band_h, W, band_h, BLUE);
    lcd.fill_rect(0, 3 * band_h, W, H - 3 * band_h, WHITE);
    for i in 1..4 {
        lcd.fill_rect(0, i * band_h - 1, W, 2, BLACK);
    }
    lcd.fill_rect(0, 0, W, 2, WHITE);
    lcd.fill_rect(0, H - 2, W, 2, WHITE);
    lcd.fill_rect(0, 0, 2, H, WHITE);
    lcd.fill_rect(W - 2, 0, 2, H, WHITE);

    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
