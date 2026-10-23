//! OPA1/OPA2 运算放大器（手册 §29）
//!
//! 2 个独立运放，正端 4 路输入（INP1..INP3 外部引脚、INP4 = 内部 DAC），
//! 负端 2 路外部输入，1 路模拟输出 `OPAx_OUT`。
//!
//! - **外部放大模式**（MODE=0x）：反馈网络在片外，运放只做开环/闭环放大；
//! - **内部 PGA 模式**（MODE=10）：增益由 `AMP` 位选择 ×2/×4/×8/×16/×32；
//! - **跟随器模式**（MODE=11）：输出 = 正端电压。
//!
//! # BGR
//!
//! 使能 OPA 会**自动启动 BGR**（手册 §25.12.19 注），无需手动开基准；
//! 若需更高精度可先等 BGR 稳定（约 30µs）再使能。
//!
//! # 校准
//!
//! 校准分两种：**电平校准**（`START` 置 1 启动、清 0 停止，时长由用户控制）
//! 与**触发校准**（`SOFTTRIG`/硬件触发启动，时长 = `CALPERIOD` 个 OPACLK）。
//! 建议上电后做一次电平校准以消除失调。
//!
//! # 用法（OPA1 跟随器，输入 INP1）
//!
//! ```ignore
//! use cw32l012_hal::opa::{Opa, OpaMode};
//!
//! let mut opa1 = Opa::new(p.opa1, &mut rcc);
//! opa1.enable_inputs(pac::opa::CR_INP1EN, 0);
//! opa1.set_mode(OpaMode::Follower);
//! opa1.enable();
//! opa1.calibrate_level(CalPeriod::C65536);  // 电平校准
//! // 校准结束后输出可用 ADC 采样（OPA1_OUT 内部连 ADC）
//! ```

use crate::pac::{self, opa as o};
use crate::rcc::Rcc;

/// 工作模式（CR.MODE）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpaMode {
    /// 外部放大（片外反馈网络）
    External,
    /// 内部 PGA，增益由 `PgaGain` 决定
    Pga(PgaGain),
    /// 电压跟随器（增益 1）
    Follower,
}

/// PGA 增益（CR.AMP）：64/32=×2 … 64/2=×32
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgaGain {
    /// ×2（AMP=000）
    X2,
    /// ×4（AMP=001）
    X4,
    /// ×8（AMP=010）
    X8,
    /// ×16（AMP=011）
    X16,
    /// ×32（AMP=100）
    X32,
}

impl PgaGain {
    #[inline]
    fn code(self) -> u32 {
        match self {
            PgaGain::X2 => 0,
            PgaGain::X4 => 1,
            PgaGain::X8 => 2,
            PgaGain::X16 => 3,
            PgaGain::X32 => 4,
        }
    }

    /// 数值倍数（便于回代验算）
    #[inline]
    pub fn factor(self) -> u32 {
        1 << (self.code() + 1)
    }
}

/// 偏置电流（CR.BIAS）：code+1 µA
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bias {
    Ua1,
    Ua2,
    Ua3,
    Ua4,
    Ua5,
    Ua6,
    Ua7,
    /// 8µA（复位默认）
    Ua8,
}

impl Bias {
    #[inline]
    fn code(self) -> u32 {
        match self {
            Bias::Ua1 => 0,
            Bias::Ua2 => 1,
            Bias::Ua3 => 2,
            Bias::Ua4 => 3,
            Bias::Ua5 => 4,
            Bias::Ua6 => 5,
            Bias::Ua7 => 6,
            Bias::Ua8 => 7,
        }
    }

    /// 微安数
    #[inline]
    pub fn microamps(self) -> u32 {
        self.code() + 1
    }
}

/// 校准时长（CAL.CALPERIOD）：`8 << code` 个 OPACLK 周期
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum CalPeriod {
    C8,
    C16,
    C32,
    C64,
    C128,
    C256,
    C512,
    C1024,
    C2048,
    C4096,
    C8192,
    C16384,
    C32768,
    C65536,
    C131072,
    C262144,
}

impl CalPeriod {
    #[inline]
    fn code(self) -> u32 {
        use CalPeriod::*;
        match self {
            C8 => 0,
            C16 => 1,
            C32 => 2,
            C64 => 3,
            C128 => 4,
            C256 => 5,
            C512 => 6,
            C1024 => 7,
            C2048 => 8,
            C4096 => 9,
            C8192 => 10,
            C16384 => 11,
            C32768 => 12,
            C65536 => 13,
            C131072 => 14,
            C262144 => 15,
        }
    }

    /// 校准周期数
    #[inline]
    pub fn cycles(self) -> u32 {
        8u32 << self.code()
    }
}

/// OPAx 运算放大器
pub struct Opa<'p> {
    regs: &'p pac::Opa,
}

impl<'p> Opa<'p> {
    /// 初始化（使能 APBEN1.OPA 时钟；运放保持关闭）
    pub fn new(regs: &'p pac::Opa, rcc: &mut Rcc) -> Self {
        rcc.enable_opa();
        Opa { regs }
    }

    /// 配置工作模式与偏置电流（不改变输入导通与 EN）
    pub fn configure(&mut self, mode: OpaMode, bias: Bias) {
        let mode_bits = match mode {
            OpaMode::External => o::MODE_EXTERNAL,
            OpaMode::Pga(_) => o::MODE_PGA,
            OpaMode::Follower => o::MODE_FOLLOWER,
        };
        let amp_bits = match mode {
            OpaMode::Pga(g) => g.code() << o::CR_AMP_POS,
            _ => 0,
        };
        self.regs.cr.modify(|v| {
            (v & !(o::CR_MODE_MASK | o::CR_AMP_MASK | o::CR_BIAS_MASK))
                | mode_bits
                | amp_bits
                | (bias.code() << o::CR_BIAS_POS)
        });
    }

    /// 仅设置工作模式（保持 AMP/BIAS 现值）
    pub fn set_mode(&mut self, mode: OpaMode) {
        let mode_bits = match mode {
            OpaMode::External => o::MODE_EXTERNAL,
            OpaMode::Pga(_) => o::MODE_PGA,
            OpaMode::Follower => o::MODE_FOLLOWER,
        };
        let amp_bits = match mode {
            OpaMode::Pga(g) => g.code() << o::CR_AMP_POS,
            _ => 0,
        };
        self.regs
            .cr
            .modify(|v| (v & !(o::CR_MODE_MASK | o::CR_AMP_MASK)) | mode_bits | amp_bits);
    }

    /// 输入导通控制（位掩码：`pac::opa::CR_INP1EN` 等，可或组合）
    ///
    /// 例如 `enable_inputs(CR_INP1EN, CR_INN1EN)` 只接通正端 INP1 与负端 INN1。
    pub fn enable_inputs(&mut self, pos_mask: u32, neg_mask: u32) {
        let pos = pos_mask & (o::CR_INP1EN | o::CR_INP2EN | o::CR_INP3EN | o::CR_INP4EN);
        let neg = neg_mask & (o::CR_INN1EN | o::CR_INN2EN);
        self.regs.cr.modify(|v| {
            (v & !(o::CR_INP1EN
                | o::CR_INP2EN
                | o::CR_INP3EN
                | o::CR_INP4EN
                | o::CR_INN1EN
                | o::CR_INN2EN))
                | pos
                | neg
        });
    }

    /// 使能运放
    pub fn enable(&mut self) {
        self.regs.cr.modify(|v| v | o::CR_EN);
    }

    /// 关闭运放
    pub fn disable(&mut self) {
        self.regs.cr.modify(|v| v & !o::CR_EN);
    }

    /// 电平校准：`START=1` 启动，持续 `period` 后需调用 [`Opa::stop_calibration`]
    ///
    /// 电平校准的实际时长由调用方控制（手册 §29.3.3 注），`period` 仅决定
    /// 内部时序参数；建议按 OPACLK 频率换算后延时足够时间再停止。
    pub fn start_level_calibration(&mut self, period: CalPeriod) {
        self.regs.cal.modify(|v| {
            (v & !o::CAL_PERIOD_MASK)
                | (period.code() << o::CAL_PERIOD_POS)
                | o::CAL_EN
                | o::CAL_MODE_LEVEL
                | o::CAL_START
        });
    }

    /// 停止电平校准
    pub fn stop_calibration(&mut self) {
        self.regs.cal.modify(|v| v & !(o::CAL_START | o::CAL_EN));
    }

    /// 触发校准（软件触发一次，时长 = `period` 个 OPACLK，自动结束）
    pub fn trigger_calibration(&mut self, period: CalPeriod) {
        self.regs.cal.modify(|v| {
            (v & !(o::CAL_PERIOD_MASK | o::CAL_MODE_LEVEL))
                | (period.code() << o::CAL_PERIOD_POS)
                | o::CAL_EN
        });
        self.regs.cal.modify(|v| v | o::CAL_SOFTTRIG);
    }

    /// 校准是否进行中（CAL.AZRUN）
    pub fn is_calibrating(&self) -> bool {
        self.regs.cal.read() & o::CAL_AZRUN != 0
    }

    /// 设置校准时钟分频（CAL.CLKDIV，0..=7）
    pub fn set_cal_clock_div(&mut self, div: u8) {
        self.regs
            .cal
            .modify(|v| (v & !o::CAL_CLKDIV_MASK) | ((div as u32 & 0x7) << o::CAL_CLKDIV_POS));
    }

    /// 原始控制寄存器值（调试用）
    pub fn cr(&self) -> u32 {
        self.regs.cr.read()
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Opa {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pga_gain_codes() {
        // 手册：000=64/32(×2) … 100=64/2(×32)
        assert_eq!(PgaGain::X2.code(), 0);
        assert_eq!(PgaGain::X32.code(), 4);
        assert_eq!(PgaGain::X2.factor(), 2);
        assert_eq!(PgaGain::X4.factor(), 4);
        assert_eq!(PgaGain::X8.factor(), 8);
        assert_eq!(PgaGain::X16.factor(), 16);
        assert_eq!(PgaGain::X32.factor(), 32);
    }

    #[test]
    fn bias_microamps() {
        assert_eq!(Bias::Ua1.microamps(), 1);
        assert_eq!(Bias::Ua8.microamps(), 8);
        // 复位默认 0xE000 → BIAS=111=8µA
        assert_eq!((0xE000u32 & o::CR_BIAS_MASK) >> o::CR_BIAS_POS, 7);
    }

    #[test]
    fn cal_period_cycles() {
        assert_eq!(CalPeriod::C8.cycles(), 8);
        assert_eq!(CalPeriod::C1024.cycles(), 1024);
        assert_eq!(CalPeriod::C262144.cycles(), 262_144);
    }

    #[test]
    fn mode_bits_do_not_collide() {
        // MODE[2:1] 与 AMP[12:10] 不重叠
        assert_eq!(o::CR_MODE_MASK & o::CR_AMP_MASK, 0);
        assert_eq!(o::MODE_PGA, 0b10 << 1);
        assert_eq!(o::MODE_FOLLOWER, 0b11 << 1);
    }
}
