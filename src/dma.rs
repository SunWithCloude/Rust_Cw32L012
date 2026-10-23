//! DMA 控制器（§8）
//!
//! 4 条独立通道、8/16/32 位位宽、软件/硬件触发、BLOCK/BULK 两种传输模式，
//! 支持自动重传（环形搬运）。通道号越小优先级越高（§8.5.4）。
//!
//! # 一次传输要配什么（§8.4.1 步骤 1~14）
//!
//! | 步骤 | 寄存器 | 本模块入口 |
//! |---|---|---|
//! | 1 传输模式 | `CSR.TRANS` | [`Mode`] |
//! | 2 触发方式 | `TRIG.TYPE` | [`Trigger`] |
//! | 3 位宽 | `CSR.SIZE` | [`Width`] |
//! | 4 数据块数量 | `CNT.CNT` | [`Config::len`] |
//! | 5 数据块大小 | `CNT.REPEAT` | [`Config::repeat`]（建议 1） |
//! | 6/7 源/目的地址 | `SRCADDR` / `DSTADDR` | [`Config::src`] / [`Config::dst`] |
//! | 8/9 地址增量 | `CSR.SRCINC` / `DSTINC` | [`Config::src_inc`] / [`Config::dst_inc`] |
//! | 10/11 地址重载 | `CSR.SRCLOAD` / `DSTLOAD` | [`Config::src_reload`] / [`Config::dst_reload`] |
//! | 12 自动重传 | `CSR.RESTART` | [`Config::restart`] |
//! | 13 使能 | `CSR.EN` | [`Channel::configure`] 内部置位 |
//! | 14 启动 | `TRIG.SOFTSRC` | [`Channel::start`] |
//!
//! # 用法
//!
//! ```ignore
//! let mut dma = Dma::new(p.dma, &mut rcc);
//! let mut dst = [0u32; 64];
//! dma.mem_to_mem(&SRC, &mut dst)?;                 // 存储器→存储器，阻塞
//!
//! // 外设→存储器：ADC 每次转换完成搬一个结果（硬件触发）
//! let mut ch = dma.channel(1);
//! ch.configure(&Config::periph_to_mem(
//!     pac::ADC1_BASE as u32 + 0x30,                  // RESULT0
//!     buf.as_mut_ptr() as u32,
//!     buf.len() as u16,
//!     Width::B16,
//!     pac::dma::hs::ADC1_SINGLE,
//! ));
//! ch.wait()?;
//! ```
//!
//! # 三个容易踩的点
//!
//! - **`CNT.REPEAT` 每次传输后被硬件清 0**（§8.5.2），所以每轮传输前都要重新
//!   [`Channel::configure`]，不能只改 `CNT`。
//! - **`ICR` 是 R1W0**：写 1 无动作、写 0 清标志。清一个通道的 TC 要写
//!   `!TC_bit`（本模块已封装）。
//! - **软件触发下 `SOFTSRC` 传输完成后自动清零**，所以"再传一次"必须重新写它；
//!   硬件触发则只需外设继续发请求（配合 `RESTART` 可做无限环形搬运）。

use crate::pac::{self, dma as dc};
use crate::rcc::Rcc;

/// 传输数据位宽（`CSR.SIZE`，同时决定地址自增量 1/2/4）
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Width {
    B8 = 0,
    B16 = 1,
    B32 = 2,
}

impl Width {
    /// 寄存器编码
    #[inline]
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// 单次搬运字节数
    #[inline]
    pub const fn bytes(self) -> usize {
        1 << (self as usize)
    }

    /// 按元素大小选位宽
    pub const fn for_size(size: usize) -> Option<Self> {
        match size {
            1 => Some(Width::B8),
            2 => Some(Width::B16),
            4 => Some(Width::B32),
            _ => None,
        }
    }
}

/// 传输模式（`CSR.TRANS`，表 8-1）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 块传输：每搬完一个数据块插入传输间隙，允许 CPU / 高优先级通道抢占
    Block,
    /// 批量传输：一口气搬完，期间不让出总线（数据量别太大）
    Bulk,
}

/// 触发方式（`TRIG.TYPE` + `TRIG.HARDSRC`，表 8-8）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// 软件触发：由 [`Channel::start`] 写 `SOFTSRC` 启动
    Software,
    /// 硬件触发：由外设事件启动，取值为 [`pac::dma::hs`] 中的常量
    Hardware(u8),
}

/// 通道配置（对应手册 §8.4.1 的步骤 1~12）
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 源地址
    pub src: u32,
    /// 目的地址
    pub dst: u32,
    /// 传输数据块数量（1..=65535）
    pub len: u16,
    /// 数据块大小（1..=15，建议 1；每次传输后硬件清 0）
    pub repeat: u8,
    /// 位宽
    pub width: Width,
    /// BLOCK / BULK
    pub mode: Mode,
    /// 触发方式
    pub trigger: Trigger,
    /// 源地址自增
    pub src_inc: bool,
    /// 目的地址自增
    pub dst_inc: bool,
    /// 每个数据块完成时重载源地址（环形缓冲）
    pub src_reload: bool,
    /// 每个数据块完成时重载目的地址（环形缓冲）
    pub dst_reload: bool,
    /// 自动重传
    pub restart: bool,
}

impl Config {
    /// 存储器→存储器（地址自增、BULK、软件触发）
    pub const fn mem_to_mem(src: u32, dst: u32, len: u16, width: Width) -> Self {
        Self {
            src,
            dst,
            len,
            repeat: 1,
            width,
            mode: Mode::Bulk,
            trigger: Trigger::Software,
            src_inc: true,
            dst_inc: true,
            src_reload: false,
            dst_reload: false,
            restart: false,
        }
    }

    /// 外设→存储器（源固定、目的自增、BLOCK、硬件触发）
    pub const fn periph_to_mem(src: u32, dst: u32, len: u16, width: Width, hs: u8) -> Self {
        Self {
            src,
            dst,
            len,
            repeat: 1,
            width,
            mode: Mode::Block,
            trigger: Trigger::Hardware(hs),
            src_inc: false,
            dst_inc: true,
            src_reload: false,
            dst_reload: false,
            restart: false,
        }
    }

    /// 存储器→外设（源自增、目的固定、BLOCK、硬件触发）
    pub const fn mem_to_periph(src: u32, dst: u32, len: u16, width: Width, hs: u8) -> Self {
        Self {
            src,
            dst,
            len,
            repeat: 1,
            width,
            mode: Mode::Block,
            trigger: Trigger::Hardware(hs),
            src_inc: true,
            dst_inc: false,
            src_reload: false,
            dst_reload: false,
            restart: false,
        }
    }

    /// 编码 `CSR` 的可写位（不含 `EN`；`REPEAT` 在 `CNT` 里，不参与）
    pub const fn encode_csr(&self) -> u32 {
        let mut csr = self.width.code() << dc::CSR_SIZE_POS;
        if self.src_inc {
            csr |= dc::CSR_SRCINC;
        }
        if self.dst_inc {
            csr |= dc::CSR_DSTINC;
        }
        if let Mode::Block = self.mode {
            csr |= dc::CSR_TRANS;
        }
        if self.src_reload {
            csr |= dc::CSR_SRCLOAD;
        }
        if self.dst_reload {
            csr |= dc::CSR_DSTLOAD;
        }
        if self.restart {
            csr |= dc::CSR_RESTART;
        }
        csr
    }

    /// 编码 `CNT`（CNT[15:0] + REPEAT[19:16]）
    pub const fn encode_cnt(&self) -> u32 {
        // 手册：REPEAT 不可为 0，合法范围 1..=15（const fn 里不能调 clamp）
        let r = if self.repeat == 0 {
            1
        } else if self.repeat > 15 {
            15
        } else {
            self.repeat
        };
        (self.len as u32 & dc::CNT_CNT_MASK) | ((r as u32) << dc::CNT_REPEAT_POS)
    }

    /// 编码 `TRIG`（TYPE + HARDSRC）
    pub const fn encode_trig(&self) -> u32 {
        match self.trigger {
            Trigger::Software => 0,
            Trigger::Hardware(hs) => dc::TRIG_TYPE | (((hs as u32) & 0x3F) << dc::TRIG_HARDSRC_POS),
        }
    }
}

/// DMA 错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DmaError {
    /// 等待完成超时
    Timeout,
    /// 通道号非法（应为 1..=4）
    BadChannel,
    /// 元素宽度不是 1/2/4 字节
    UnsupportedWidth,
    /// 源/目的长度不一致或为 0
    BadLength,
    /// `STATUS=001`：传输地址超出寻址范围
    AddressOutOfRange,
    /// `STATUS=010`：传输停止请求引起中止
    Aborted,
    /// `STATUS=011`：访问源地址出错
    SourceError,
    /// `STATUS=100`：访问目的地址出错
    DestError,
}

/// DMA 控制器
pub struct Dma<'p> {
    regs: &'p pac::Dma,
}

impl<'p> Dma<'p> {
    /// 初始化（使能 AHBEN.DMA）
    pub fn new(regs: &'p pac::Dma, rcc: &mut Rcc) -> Self {
        rcc.enable_dma();
        // 清掉所有通道的残留标志（上电/软复位后 ICR 复位值为全 1）
        regs.icr.write(0);
        Dma { regs }
    }

    /// 取通道（`n` = 1..=4，数字越小优先级越高）
    pub fn channel(&mut self, n: u8) -> Channel<'_> {
        assert!((1..=4).contains(&n), "DMA 通道号必须为 1..=4（Got {n}）");
        Channel {
            regs: self.regs,
            idx: (n - 1) as usize,
        }
    }

    /// 取通道（返回值版，越界不 panic）
    pub fn try_channel(&mut self, n: u8) -> Result<Channel<'_>, DmaError> {
        if !(1..=4).contains(&n) {
            return Err(DmaError::BadChannel);
        }
        Ok(Channel {
            regs: self.regs,
            idx: (n - 1) as usize,
        })
    }

    /// 存储器→存储器阻塞搬运（自动选通道 3，把 1/2 留给实时外设）
    ///
    /// `src` / `dst` 长度必须相等且非空；元素类型决定位宽（u8/u16/u32）。
    pub fn mem_to_mem<T: Copy>(&mut self, src: &[T], dst: &mut [T]) -> Result<(), DmaError> {
        let width = Width::for_size(core::mem::size_of::<T>()).ok_or(DmaError::UnsupportedWidth)?;
        if src.is_empty() || src.len() != dst.len() || src.len() > 0xFFFF {
            return Err(DmaError::BadLength);
        }
        let cfg = Config::mem_to_mem(
            src.as_ptr() as u32,
            dst.as_mut_ptr() as u32,
            src.len() as u16,
            width,
        );
        let ch = self.channel(3);
        ch.configure(&cfg);
        ch.start();
        ch.wait()
    }

    /// 寄存器块原始访问
    pub fn regs(&self) -> &'p pac::Dma {
        self.regs
    }
}

/// 单个 DMA 通道
pub struct Channel<'d> {
    regs: &'d pac::Dma,
    idx: usize,
}

impl<'d> Channel<'d> {
    #[inline]
    fn csr(&self) -> &'d pac::Rw<u32> {
        &self.regs.ch[self.idx].csr
    }

    /// 通道号（1..=4）
    #[inline]
    pub fn index(&self) -> u8 {
        (self.idx + 1) as u8
    }

    /// 按配置装载通道（自动 `DISABLE → 清标志 → 写寄存器 → EN=1`）
    ///
    /// **每轮传输前都要调用**：`CNT.REPEAT` 会被硬件清 0（§8.5.2）。
    pub fn configure(&self, cfg: &Config) {
        let ch = &self.regs.ch[self.idx];
        ch.csr.modify(|v| v & !dc::CSR_EN);
        self.clear_flags();
        ch.cnt.write(cfg.encode_cnt());
        ch.src.write(cfg.src);
        ch.dst.write(cfg.dst);
        ch.trig.write(cfg.encode_trig());
        ch.csr.write(cfg.encode_csr() | dc::CSR_EN);
    }

    /// 软件触发启动（硬件触发模式下无需调用）
    ///
    /// `SOFTSRC` 在传输完成后由硬件自动清零，重复传输时要重新写。
    pub fn start(&self) {
        self.regs.ch[self.idx].trig.modify(|v| v | dc::TRIG_SOFTSRC);
    }

    /// 只置 `EN`（用于"先配好、等外设就绪再开"的场景）
    pub fn enable(&self) {
        self.csr().modify(|v| v | dc::CSR_EN);
    }

    /// 清 `EN`（停止通道；正在进行的传输会被中止）
    pub fn disable(&self) {
        self.csr().modify(|v| v & !dc::CSR_EN);
    }

    /// 使能传输完成中断（不改变其它配置）
    pub fn enable_irq(&self) {
        self.csr().modify(|v| v | dc::CSR_TCIE);
    }

    /// 还剩多少个数据块没搬（`CCNT`）
    pub fn remaining(&self) -> u16 {
        (self.regs.ch[self.idx].ccnt.read() & 0xFFFF) as u16
    }

    /// 通道状态（`STATUS`，表 8-2 编码）
    pub fn status(&self) -> u8 {
        ((self.csr().read() & dc::CSR_STATUS_MASK) >> dc::CSR_STATUS_POS) as u8
    }

    /// 是否传输完成（`CSR.TC`）
    pub fn is_done(&self) -> bool {
        self.csr().read() & dc::CSR_TC != 0
    }

    /// 是否出错（`CSR.TE`）
    pub fn is_error(&self) -> bool {
        self.csr().read() & dc::CSR_TE != 0
    }

    /// 解析当前状态为 `Result`
    pub fn check(&self) -> Result<(), DmaError> {
        let csr = self.csr().read();
        if csr & dc::CSR_TE != 0 {
            return Err(match (csr & dc::CSR_STATUS_MASK) >> dc::CSR_STATUS_POS {
                1 => DmaError::AddressOutOfRange,
                2 => DmaError::Aborted,
                3 => DmaError::SourceError,
                4 => DmaError::DestError,
                _ => DmaError::Aborted,
            });
        }
        Ok(())
    }

    /// 清本通道的 TC/TE 标志（`ICR` 为 R1W0：写 0 清除）
    pub fn clear_flags(&self) {
        let tc = 1u32 << (2 * self.idx);
        let te = 1u32 << (2 * self.idx + 1);
        self.regs.icr.write(!(tc | te));
    }

    /// 阻塞等待完成（带自旋上限，约 2^26 次）
    pub fn wait(&self) -> Result<(), DmaError> {
        self.wait_timeout(1 << 26)
    }

    /// 阻塞等待完成，`spins` 为轮询上限
    pub fn wait_timeout(&self, spins: u32) -> Result<(), DmaError> {
        let mut n = spins;
        loop {
            if self.is_done() {
                return self.check();
            }
            if self.is_error() {
                return self.check();
            }
            if n == 0 {
                return Err(DmaError::Timeout);
            }
            n -= 1;
            core::hint::spin_loop();
        }
    }

    /// 寄存器块原始访问
    pub fn regs(&self) -> &'d pac::DmaChannel {
        &self.regs.ch[self.idx]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_codes_and_bytes() {
        assert_eq!(Width::B8.code(), 0);
        assert_eq!(Width::B16.code(), 1);
        assert_eq!(Width::B32.code(), 2);
        assert_eq!(Width::B8.bytes(), 1);
        assert_eq!(Width::B16.bytes(), 2);
        assert_eq!(Width::B32.bytes(), 4);
        assert_eq!(Width::for_size(4), Some(Width::B32));
        assert_eq!(Width::for_size(3), None);
    }

    #[test]
    fn csr_encoding_matches_manual_bits() {
        let cfg = Config::mem_to_mem(0x2000_0000, 0x2000_0100, 32, Width::B32);
        let csr = cfg.encode_csr();
        assert_eq!(csr & dc::CSR_SIZE_MASK, 2 << dc::CSR_SIZE_POS); // 32bit
        assert_ne!(csr & dc::CSR_SRCINC, 0);
        assert_ne!(csr & dc::CSR_DSTINC, 0);
        assert_eq!(csr & dc::CSR_TRANS, 0); // BULK
        assert_eq!(csr & dc::CSR_EN, 0); // EN 由 configure 单独置位
    }

    #[test]
    fn cnt_and_trig_encoding() {
        let mut cfg = Config::mem_to_mem(0, 0, 1000, Width::B16);
        cfg.repeat = 3;
        assert_eq!(cfg.encode_cnt(), 1000 | (3 << 16));
        assert_eq!(cfg.encode_trig(), 0); // 软件触发

        let cfg =
            Config::periph_to_mem(0x4000_0030, 0x2000_0000, 8, Width::B16, dc::hs::ADC1_SINGLE);
        assert_eq!(
            cfg.encode_trig(),
            dc::TRIG_TYPE | ((dc::hs::ADC1_SINGLE as u32) << dc::TRIG_HARDSRC_POS)
        );
    }

    #[test]
    fn repeat_is_clamped_to_valid_range() {
        let mut cfg = Config::mem_to_mem(0, 0, 1, Width::B8);
        cfg.repeat = 0; // 手册：REPEAT 不可为 0
        assert_eq!(cfg.encode_cnt() >> 16, 1);
        cfg.repeat = 0xFF;
        assert_eq!(cfg.encode_cnt() >> 16, 15);
    }
}
