//! LPTIM 低功耗定时器（手册 §15）
//!
//! 三个计数时钟源：PCLK（运行模式）、LSE 32.768kHz（外部晶体）、
//! LSI ≈32.8kHz（内部低速 RC）。LSI/LSE 时钟域在深度休眠下持续计数，
//! **ARRM/CMPM 中断可唤醒深度休眠**（IRQ19，表 3-3 深度休眠列 Y）。
//!
//! # 使用约束（手册 §15.3.2）
//!
//! - `CFGR` 与 `IER` **仅在 EN=0 时可写**，EN=1 时写入被硬件丢弃；
//! - 先 `enable()`（EN=1），再写 `CNTSTART`/`SNGSTART` 启动计数；
//! - EN=1 后需 2 个计数时钟周期 LPTIM 才真正开始计数；
//! - **LSI 复位默认关闭**（手册 §4.5.5）：`LptimClk::Lsi` 时 [`Lptim::new`]
//!   自动使能振荡器并等待 `STABLE`，否则计数器完全不动作。
//!
//! # 用法
//!
//! ```ignore
//! use cw32l012_hal::lptim::{Lptim, LptimClk, flags};
//!
//! let mut lpt = Lptim::new(p.lptim, &mut rcc, LptimClk::Lsi, 0); // DIV1
//! lpt.set_arr(0x7FFF);            // ≈1s @ LSI 32.8kHz
//! lpt.enable_irq(flags::ARRM);    // EN=0 时配置中断
//! lpt.enable();
//! lpt.start_continuous();
//! ```
//!
//! DeepSleep 中 LPTIM 继续计数，ARRM 中断唤醒 CPU（示例见
//! `examples/lptim_deepsleep.rs`，含 LSI 频率自校准）。

use crate::pac::{self, lptim as lp};
use crate::rcc::Rcc;

/// LSI 标称频率（时钟树 32.8kHz；RC 振荡器实际值建议板上校准）
pub const LSI_HZ_TYP: u32 = pac::sysctrl::LSI_HZ;
/// LSE 标称频率
pub const LSE_HZ: u32 = 32_768;

/// 计数时钟源（CFGR.ICLKSRC[26:25]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LptimClk {
    /// PCLK（深度休眠下停止，仅运行/休眠模式用）
    Pclk,
    /// 外部 32.768kHz 晶体（板卡需焊接 LSE 晶体）
    Lse,
    /// 内部低速 RC ≈32.8kHz（深度休眠域）
    Lsi,
}

/// 中断/状态标志（ISR/IER/ICR 位序共用）
pub mod flags {
    pub use crate::pac::lptim::{ARRM, CMPM};
}

/// 分频后的节拍频率（纯函数，便于单测）
#[inline]
fn tick_hz_of(iclk_hz: u32, div_log2: u8) -> u32 {
    iclk_hz / (1u32 << (div_log2 & 0x7))
}

/// LPTIM 低功耗定时器
pub struct Lptim<'p> {
    regs: &'p pac::Lptim,
    clk: LptimClk,
    div_log2: u8,
    /// 计数时钟是否已就绪（LSI 时 = 振荡器稳定标志）
    clk_ready: bool,
}

impl<'p> Lptim<'p> {
    /// 初始化（使能 APBEN2.LPTIM 时钟并配置 CFGR）
    ///
    /// `div_log2`：预分频指数 0..=7（ICLK/2^n → DIV1..DIV128）
    ///
    /// **`LptimClk::Lsi` 会自动调用 [`Rcc::enable_lsi`]**（LSI 复位默认关闭，
    /// 不启动则计数器完全不动作）；是否稳定可经 [`Lptim::clk_ready`] 查询。
    /// `LptimClk::Lse` 需板上有 32.768kHz 晶体且外部使能 LSE，本驱动不代管。
    pub fn new(regs: &'p pac::Lptim, rcc: &mut Rcc, clk: LptimClk, div_log2: u8) -> Self {
        rcc.enable_lptim();
        let clk_ready = match clk {
            LptimClk::Lsi => rcc.enable_lsi(),
            _ => true,
        };
        let iclksrc = match clk {
            LptimClk::Pclk => 0u32,
            LptimClk::Lse => 2,
            LptimClk::Lsi => 3,
        };
        // COUNTMD=0（定时模式）、TRIGEN=00（软件启动）、ENCMD=00
        regs.cfgr.write(
            (iclksrc << lp::CFGR_ICLKSRC_POS) | ((div_log2 as u32 & 0x7) << lp::CFGR_PRS_POS),
        );
        Lptim {
            regs,
            clk,
            div_log2,
            clk_ready,
        }
    }

    /// 计数时钟是否就绪（LSI 时即振荡器已稳定；其余时钟源恒 `true`）
    pub fn clk_ready(&self) -> bool {
        self.clk_ready
    }

    /// 计数时钟源频率（PCLK 由 Rcc 推算；LSI/LSE 为标称值）
    pub fn iclk_hz(&self, rcc: &Rcc) -> u32 {
        match self.clk {
            LptimClk::Pclk => rcc.clocks().pclk_hz,
            LptimClk::Lse => LSE_HZ,
            LptimClk::Lsi => LSI_HZ_TYP,
        }
    }

    /// 分频后的计数节拍频率（Hz）
    pub fn tick_hz(&self, iclk_hz: u32) -> u32 {
        tick_hz_of(iclk_hz, self.div_log2)
    }

    /// 预分频指数（0..=7）
    pub fn div_log2(&self) -> u8 {
        self.div_log2
    }

    /// 设置自动重载值（CNT 达 ARR 触发 ARRM）
    pub fn set_arr(&mut self, arr: u16) {
        self.regs.arr.write(arr as u32);
    }

    /// 设置比较值（CNT 达 CMP 触发 CMPM）
    pub fn set_cmp(&mut self, cmp: u16) {
        self.regs.cmp.write(cmp as u32);
    }

    /// 使能内核（EN=1；生效需 2 个计数时钟周期）
    pub fn enable(&mut self) {
        self.regs.cr0.modify(|v| v | lp::CR0_EN);
    }

    /// 禁用内核（EN=0；此后 CFGR/IER 可写）
    pub fn disable(&mut self) {
        self.regs.cr0.modify(|v| v & !lp::CR0_EN);
    }

    /// 连续计数启动（须先 [`Lptim::enable`]；EN=0 时写入被丢弃）
    pub fn start_continuous(&mut self) {
        self.regs.cr0.modify(|v| v | lp::CR0_CNTSTART);
    }

    /// 单次启动（一个计数周期后停止）
    pub fn start_single(&mut self) {
        self.regs.cr0.modify(|v| v | lp::CR0_SNGSTART);
    }

    /// 读计数值（手册要求：连续两次读到相同值才有效）
    pub fn count(&self) -> u16 {
        let mut a = self.regs.cnt.read() as u16;
        loop {
            let b = self.regs.cnt.read() as u16;
            if a == b {
                return a;
            }
            a = b;
        }
    }

    /// 异步复位计数值（ARST W0 语义：读到 1 时写 0，复位完成自动回 1）
    pub fn reset_count(&mut self) {
        let v = self.regs.cr0.read();
        if v & lp::CR0_ARST != 0 {
            // 同时清启动位，避免复位后重复触发
            self.regs
                .cr0
                .write(v & !lp::CR0_ARST & !lp::CR0_CNTSTART & !lp::CR0_SNGSTART);
        }
        while self.regs.cr0.read() & lp::CR0_ARST == 0 {}
    }

    /// 使能中断源（IER；**仅 EN=0 时可写**，EN=1 时硬件丢弃本次写入）
    pub fn enable_irq(&mut self, flags: u32) {
        debug_assert!(
            self.regs.cr0.read() & lp::CR0_EN == 0,
            "LPTIM IER 仅在 EN=0 时可写"
        );
        self.regs.ier.modify(|v| v | (flags & lp::FLAGS_MASK));
    }

    /// 清除中断标志（ICR R1W0：对应位写 0 清除，其余位写 1 无动作）
    pub fn clear_irq(&mut self, flags: u32) {
        self.regs
            .icr
            .write(lp::FLAGS_MASK & !(flags & lp::FLAGS_MASK));
    }

    /// 原始 ISR 状态（轮询用）
    pub fn isr(&self) -> u32 {
        self.regs.isr.read()
    }

    /// ARRM 标志（计数值到达 ARR）
    pub fn arrm_flag(&self) -> bool {
        self.regs.isr.read() & lp::ARRM != 0
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Lptim {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_math() {
        // LSI 32.8kHz：DIV1 全速，DIV32 → 1025Hz
        assert_eq!(tick_hz_of(LSI_HZ_TYP, 0), 32_800);
        assert_eq!(tick_hz_of(LSI_HZ_TYP, 5), 1_025);
        assert_eq!(tick_hz_of(LSE_HZ, 7), 256);
        // PCLK 4MHz：DIV128 → 31.25kHz
        assert_eq!(tick_hz_of(4_000_000, 7), 31_250);
    }

    #[test]
    fn tick_never_divides_by_zero_and_wraps_log2() {
        // div_log2 超出 3bit 时按 &0x7 处理，不会 1<<32 溢出
        assert_eq!(tick_hz_of(32_800, 8), tick_hz_of(32_800, 0));
        assert_eq!(tick_hz_of(32_800, 255), tick_hz_of(32_800, 7));
    }

    #[test]
    fn icr_clear_value_keeps_other_flags() {
        // R1W0：清 ARRM 时其余标志位写 1（无动作），窗口限 0x7F
        assert_eq!(lp::FLAGS_MASK & !lp::ARRM, 0x7D);
        assert_eq!(lp::FLAGS_MASK & !lp::CMPM, 0x7E);
        assert_eq!(lp::FLAGS_MASK & !(lp::ARRM | lp::CMPM), 0x7C);
    }
}
