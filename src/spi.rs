//! SPI 主机驱动（§22）
//!
//! 阻塞式收发，8-bit 帧，四线全双工或单工单发。软件片选（CS 用普通
//! GPIO 输出控制，见 [`Spi`] 文档末尾的时序说明）。
//!
//! 引脚复用（手册表 9-2 常用组合，完整表见手册）：
//!
//! | SPI  | SCK              | MOSI               | MISO               |
//! |------|------------------|--------------------|--------------------|
//! | SPI1 | PA05 / PA10 (AF3) | PA07 / PA12 (AF3) | PA06 / PA11 (AF3)  |
//! | SPI2 | PB03 (AF?)       | PB05 (AF?)         | PB04 (AF?)         |
//! | SPI3 | PA10 (AF?)       | PA11 (AF?)         | PA10 (AF?)         |
//!
//! 注：SPI2/SPI3 引脚因封装引脚数较少通常不可用（CW32L012 常见封装
//! 仅引出 GPIOA 与部分 GPIOC），具体以所用封装引脚定义表为准。
//!
//! # 用法（SPI1 + ST7789 屏幕：PA05=SCK、PA07=MOSI、PA02=CS）
//!
//! ```ignore
//! let p = Peripherals::take().unwrap();
//! let mut rcc = rcc::Rcc::new(p.sysctrl, p.flashctrl);
//! let sck = Pin::new(p.gpioa, PortId::A, 5, &mut rcc).into_af_output(Af::Af3);
//! let mosi = Pin::new(p.gpioa, PortId::A, 7, &mut rcc).into_af_output(Af::Af3);
//! let mut spi = Spi::new_tx_only(p.spi1, sck, mosi, &mut rcc,
//!     SpiConfig { baud_n: 0, ..SpiConfig::default() }); // PCLK/2
//! spi.write(&[0x11]); // Sleep Out 命令
//! ```
//!
//! # 发送完成时序
//!
//! `write`/`write_byte` 返回前已等待 `BUSY=0`（移位寄存器排空），
//! 因此返回后可立即拉高 CS，末字节不会截断。

use crate::gpio::AfPin;
use crate::pac::{self, spi as sp};
use crate::rcc::Rcc;

/// SPI 时序模式（CPOL/CPHA 组合）
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpiMode {
    /// CPOL=0, CPHA=0
    Mode0,
    /// CPOL=0, CPHA=1
    Mode1,
    /// CPOL=1, CPHA=0
    Mode2,
    /// CPOL=1, CPHA=1
    Mode3,
}

/// 数据位序
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BitOrder {
    /// MSB 先行（默认）
    #[default]
    MsbFirst,
    /// LSB 先行
    LsbFirst,
}

/// SPI 配置
#[derive(Debug, Clone, Copy)]
pub struct SpiConfig {
    /// 时序模式（默认 Mode0）
    pub mode: SpiMode,
    /// 位序（默认 MSB）
    pub bit_order: BitOrder,
    /// 波特率分频 N：`SCK = PCLK / (2×(N+1))`，0..=127（默认 0 = PCLK/2）
    pub baud_n: u8,
}

impl Default for SpiConfig {
    fn default() -> Self {
        SpiConfig {
            mode: SpiMode::Mode0,
            bit_order: BitOrder::MsbFirst,
            baud_n: 0,
        }
    }
}

impl SpiConfig {
    /// 按目标 SCK 频率选择最接近（不高于）的分频 N。
    ///
    /// `pclk_hz` 为当前 PCLK 频率；返回实际 SCK = `pclk_hz / (2×(N+1))`。
    pub fn with_baud_closest(mut self, pclk_hz: u32, target_sck_hz: u32) -> Self {
        self.baud_n = baud_n_for(pclk_hz, target_sck_hz);
        self
    }

    /// 实际 SCK 频率
    pub fn sck_hz(&self, pclk_hz: u32) -> u32 {
        pclk_hz / 2 / (self.baud_n as u32 + 1)
    }
}

/// 计算不超过 `target_sck_hz` 的最大分频档 N（0..=127）。
///
/// 数学：`SCK = PCLK/(2(N+1)) ≤ target` → `N ≥ PCLK/(2·target) − 1`
pub fn baud_n_for(pclk_hz: u32, target_sck_hz: u32) -> u8 {
    if target_sck_hz == 0 {
        return 127; // 最慢档
    }
    // ceil(pclk / (2*target)) - 1
    let n = ((pclk_hz + 2 * target_sck_hz - 1) / (2 * target_sck_hz)).saturating_sub(1);
    n.min(127) as u8
}

/// SPI 主机（阻塞式，8-bit 帧）
///
/// 片选（CS）由调用方用普通 GPIO 输出管理：拉低 CS → `write`/`transfer` →
/// 拉高 CS。`write` 系列返回时移位已全部完成，可直接操作 CS。
pub struct Spi<'p> {
    regs: &'p pac::Spi,
    _sck: AfPin<'p>,
    _mosi: Option<AfPin<'p>>,
    _miso: Option<AfPin<'p>>,
    tx_only: bool,
}

impl<'p> Spi<'p> {
    /// 初始化 SPI 主机（四线全双工）
    ///
    /// * `regs` — SPI1/2/3 寄存器块（按基地址自动识别并使能对应 APB1 时钟）
    /// * `sck` / `mosi` / `miso` — 已配置为对应 AF 的引脚
    pub fn new(
        regs: &'p pac::Spi,
        sck: AfPin<'p>,
        mosi: AfPin<'p>,
        miso: AfPin<'p>,
        rcc: &mut Rcc,
        cfg: SpiConfig,
    ) -> Self {
        let mut s = Self::init(
            regs,
            sck,
            Some(mosi),
            Some(miso),
            rcc,
            cfg,
            sp::CR1_MODE_FULL_DUPLEX,
        );
        s.tx_only = false;
        s
    }

    /// 初始化 SPI 主机（单工单发，无 MISO；屏幕/数码管等只写器件）
    pub fn new_tx_only(
        regs: &'p pac::Spi,
        sck: AfPin<'p>,
        mosi: AfPin<'p>,
        rcc: &mut Rcc,
        cfg: SpiConfig,
    ) -> Self {
        let mut s = Self::init(regs, sck, Some(mosi), None, rcc, cfg, sp::CR1_MODE_TX_ONLY);
        s.tx_only = true;
        s
    }

    /// 初始化 SPI 主机（单工单收，无 MOSI；仅读器件）
    pub fn new_rx_only(
        regs: &'p pac::Spi,
        sck: AfPin<'p>,
        miso: AfPin<'p>,
        rcc: &mut Rcc,
        cfg: SpiConfig,
    ) -> Self {
        let mut s = Self::init(regs, sck, None, Some(miso), rcc, cfg, sp::CR1_MODE_RX_ONLY);
        s.tx_only = false;
        s
    }

    fn init(
        regs: &'p pac::Spi,
        sck: AfPin<'p>,
        mosi: Option<AfPin<'p>>,
        miso: Option<AfPin<'p>>,
        rcc: &mut Rcc,
        cfg: SpiConfig,
        mode_bits: u32,
    ) -> Self {
        // 按基地址自动识别 SPI 实例并使能 APB1 时钟
        let addr = regs as *const _ as usize;
        if addr == pac::SPI1_BASE {
            rcc.enable_spi1();
        } else if addr == pac::SPI2_BASE {
            rcc.enable_spi2();
        } else if addr == pac::SPI3_BASE {
            rcc.enable_spi3();
        }

        let (cpol_cpha, lsb) = match (cfg.mode, cfg.bit_order) {
            (SpiMode::Mode0, BitOrder::MsbFirst) => (0, 0),
            (SpiMode::Mode0, BitOrder::LsbFirst) => (0, sp::CR1_LSBF),
            (SpiMode::Mode1, BitOrder::MsbFirst) => (sp::CR1_CPHA, 0),
            (SpiMode::Mode1, BitOrder::LsbFirst) => (sp::CR1_CPHA, sp::CR1_LSBF),
            (SpiMode::Mode2, BitOrder::MsbFirst) => (sp::CR1_CPOL, 0),
            (SpiMode::Mode2, BitOrder::LsbFirst) => (sp::CR1_CPOL, sp::CR1_LSBF),
            (SpiMode::Mode3, BitOrder::MsbFirst) => (sp::CR1_CPOL | sp::CR1_CPHA, 0),
            (SpiMode::Mode3, BitOrder::LsbFirst) => (sp::CR1_CPOL | sp::CR1_CPHA, sp::CR1_LSBF),
        };

        // CR2.EN=0 期间才允许写 CR1（手册 §22.7.1 注意事项）
        regs.cr2.write(0);
        regs.cr1.write(
            ((cfg.baud_n as u32) & 0x7F) << sp::CR1_BR_POS
                | mode_bits
                | cpol_cpha
                | lsb
                | 7 << sp::CR1_WIDTH_POS // 8-bit 帧（WIDTH+1）
                | sp::CR1_SSM   // 主机：CS 由 SSI 输出（自保持未选中，防 MODF）
                | sp::CR1_MSTR,
        );
        regs.ssi.write(sp::SSI_HIGH); // 主机自保持"未选中"
        regs.icr.write(0); // R1W0：写 0 清除全部标志
        regs.cr2.write(sp::CR2_EN);

        Spi {
            regs,
            _sck: sck,
            _mosi: mosi,
            _miso: miso,
            tx_only: mode_bits == sp::CR1_MODE_TX_ONLY,
        }
    }

    /// 等待发送缓冲空（可写下一帧）
    #[inline]
    fn wait_txe(&self) {
        while self.regs.isr.read() & sp::ISR_TXE == 0 {}
    }

    /// 等待总线空闲（移位寄存器排空；发送完成的判据）
    #[inline]
    fn wait_idle(&self) {
        while self.regs.isr.read() & sp::ISR_BUSY != 0 {}
    }

    /// 等待接收缓冲非空
    #[inline]
    fn wait_rxne(&self) {
        while self.regs.isr.read() & sp::ISR_RXNE == 0 {}
    }

    /// 阻塞发送一个字节（等待 TXE 后写入 DR；**不等待移位完成**）
    ///
    /// 连续调用即可流水发送；需要确保发完（如拉高 CS）时调用 [`Spi::flush`]。
    pub fn write_byte(&mut self, b: u8) {
        self.wait_txe();
        self.regs.dr.write(b as u32);
    }

    /// 等待全部移位完成（BUSY=0）
    pub fn flush(&mut self) {
        self.wait_idle();
    }

    /// 全双工收发一个字节（写入的同时读回）
    pub fn transfer_byte(&mut self, b: u8) -> u8 {
        self.wait_txe();
        self.regs.dr.write(b as u32);
        self.wait_rxne();
        self.regs.dr.read() as u8
    }

    /// 阻塞发送缓冲区（返回前已完成移位，可直接操作 CS）
    pub fn write(&mut self, buf: &[u8]) {
        for &b in buf {
            self.write_byte(b);
        }
        self.flush();
    }

    /// 全双工交换：`buf[i]` 发出的同时读回原位覆盖
    pub fn transfer_in_place(&mut self, buf: &mut [u8]) {
        for b in buf.iter_mut() {
            *b = self.transfer_byte(*b);
        }
    }

    /// 全双工交换：发送 `tx`，读收到 `rx`（两者长度必须一致）
    ///
    /// # Panics
    /// `tx` 与 `rx` 长度不一致时 panic。
    pub fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) {
        assert_eq!(tx.len(), rx.len(), "transfer: tx/rx 长度必须一致");
        for i in 0..tx.len() {
            rx[i] = self.transfer_byte(tx[i]);
        }
    }

    /// 全双工读取（发送哑元 0x00 驱动时钟）
    pub fn read(&mut self, buf: &mut [u8]) {
        for b in buf.iter_mut() {
            *b = self.transfer_byte(0x00);
        }
    }

    /// 是否处于单工单发模式（无 RXNE 可等）
    pub fn is_tx_only(&self) -> bool {
        self.tx_only
    }

    /// 寄存器块原始访问（未封装功能的逃生口）
    pub fn regs(&mut self) -> &'p pac::Spi {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baud_div_matches_manual_formula() {
        // SCK = PCLK / (2×(N+1))
        assert_eq!(baud_n_for(48_000_000, 48_000_000), 0); // PCLK/2
        assert_eq!(baud_n_for(48_000_000, 24_000_000), 0); // 24M ≤ 24M → N=0
        assert_eq!(baud_n_for(48_000_000, 20_000_000), 1); // → PCLK/4 = 12M
        assert_eq!(baud_n_for(4_000_000, 250_000), 7); // PCLK/16
        assert_eq!(baud_n_for(4_000_000, 1), 127); // 钳位最慢档
        assert_eq!(baud_n_for(4_000_000, 0), 127); // 目标 0 → 最慢
    }

    #[test]
    fn config_sck_hz() {
        let cfg = SpiConfig::default().with_baud_closest(48_000_000, 12_000_000);
        assert_eq!(cfg.baud_n, 1);
        assert_eq!(cfg.sck_hz(48_000_000), 12_000_000);
    }

    #[test]
    fn default_is_mode0_msb_pclk_div2() {
        let cfg = SpiConfig::default();
        assert_eq!(cfg.mode, SpiMode::Mode0);
        assert_eq!(cfg.bit_order, BitOrder::MsbFirst);
        assert_eq!(cfg.baud_n, 0);
        assert_eq!(cfg.sck_hz(4_000_000), 2_000_000);
    }
}
