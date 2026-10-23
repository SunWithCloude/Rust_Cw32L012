//! VC1–VC4 模拟电压比较器（手册 §27）
//!
//! 4 个独立比较器：正端可选 4 路外部输入（CH0..CH3），负端可选 2 路外部
//! 输入、内部 DAC 输出或内部电阻分压参考。比较结果可反相、加 20mV 迟滞、
//! 做窗口比较、经数字滤波后从 `VCx_OUT` 引脚输出或触发中断。
//!
//! # 分压参考源
//!
//! VC1/VC2 共用 `VC12_REF`，VC3/VC4 共用 `VC34_REF`：
//! `V_ref = VIN × (1 + DIV) / 8`，VIN 可切 VDDA 或 Vcore(≈1.6V)。
//! 例如 VDDA=3.3V、DIV=3 → 1.65V。
//!
//! # 用法（VC1：CH1 与 1.65V 比较，超阈值中断）
//!
//! ```ignore
//! use cw32l012_hal::vc::{Vc, NpInput, NnInput, Speed};
//!
//! let mut vc1 = Vc::new(p.vc1, p.vc12ref, &mut rcc);
//! vc1.ref_enable(3, false);                 // VDDA×4/8 = 1.65V
//! vc1.select_inputs(NpInput::Ch1, NnInput::RefDiv);
//! vc1.set_speed(Speed::High);               // 高速（自动开 BGR）
//! vc1.set_hysteresis(true);
//! vc1.set_irq(vc::CR1_RISEIE);              // 上升沿中断
//! vc1.enable_irq(true);
//! vc1.enable();
//! ```
//!
//! # 中断
//!
//! VC1/VC3 → IRQ14（`VC1_VC3_irq`），VC2/VC4 → IRQ15（`VC2_VC4_irq`），
//! 两者在深度休眠下均可唤醒 CPU（表 3-3 深度休眠列 YYY）。

use crate::pac::{self, vc as v};
use crate::rcc::Rcc;

/// 正端输入（VCx_CH0..CH3，对应引脚见手册表 27-2）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpInput {
    Ch0,
    Ch1,
    Ch2,
    Ch3,
}

impl NpInput {
    #[inline]
    fn code(self) -> u32 {
        match self {
            NpInput::Ch0 => 0,
            NpInput::Ch1 => 1,
            NpInput::Ch2 => 2,
            NpInput::Ch3 => 3,
        }
    }
}

/// 负端输入
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NnInput {
    /// VCx_CH0
    Ch0,
    /// VCx_CH1
    Ch1,
    /// 内部 DAC 输出（VC1/VC3 取 DAC_OUT1，VC2/VC4 取 DAC_OUT2）
    Dac,
    /// VC1/VC2（或 VC3/VC4）公共电阻分压参考
    RefDiv,
}

impl NnInput {
    #[inline]
    fn code(self) -> u32 {
        match self {
            NnInput::Ch0 => 0,
            NnInput::Ch1 => 1,
            NnInput::Dac => 2,
            NnInput::RefDiv => 3,
        }
    }
}

/// 响应速度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    /// 低速（更省电）
    Low,
    /// 高速（硬件自动使能 BGR）
    High,
}

/// 数字滤波时钟源（CR1.FLTCLK）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterClock {
    /// LSI（DeepSleep 下也可滤波，但持续耗电）
    Lsi,
    /// PCLK（默认；DeepSleep 前建议选它以免 LSI 耗电）
    Pclk,
}

/// 数字滤波时间（CR1.FLTTIME，见手册 §27.7.4 表）
///
/// 命名 `S<分频>N<采样数>`：采样时钟 = f_FLTCLK / 分频，连续 N 个采样一致才
/// 更新输出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum FilterTime {
    /// 无滤波
    Off,
    /// f/1, N=2
    S1N2,
    /// f/1, N=4
    S1N4,
    /// f/1, N=8
    S1N8,
    /// f/2, N=6
    S2N6,
    /// f/2, N=8
    S2N8,
    /// f/4, N=6
    S4N6,
    /// f/4, N=8
    S4N8,
    /// f/8, N=6
    S8N6,
    /// f/8, N=8
    S8N8,
    /// f/16, N=5
    S16N5,
    /// f/16, N=6
    S16N6,
    /// f/16, N=8
    S16N8,
    /// f/32, N=5
    S32N5,
    /// f/32, N=6
    S32N6,
    /// f/32, N=8
    S32N8,
}

impl FilterTime {
    #[inline]
    fn code(self) -> u32 {
        use FilterTime::*;
        match self {
            Off => 0,
            S1N2 => 1,
            S1N4 => 2,
            S1N8 => 3,
            S2N6 => 4,
            S2N8 => 5,
            S4N6 => 6,
            S4N8 => 7,
            S8N6 => 8,
            S8N8 => 9,
            S16N5 => 10,
            S16N6 => 11,
            S16N8 => 12,
            S32N5 => 13,
            S32N6 => 14,
            S32N8 => 15,
        }
    }
}

/// 中断触发方式（可组合：`v::CR1_RISEIE | v::CR1_FALLIE`）
pub mod irq {
    pub use crate::pac::vc::{CR1_FALLIE, CR1_HIGHIE, CR1_RISEIE};
}

/// 分压参考源电压（mV）：`VIN × (1+DIV) / 8`
///
/// `vin_mv` 为 VDDA（mV）或 Vcore（≈1600mV）。
pub fn ref_voltage_mv(vin_mv: u32, div: u8) -> u32 {
    let d = (div as u32) & 0xF; // 先掩码再运算，避免 `1 + div & 0xF` 的优先级陷阱
    vin_mv * (1 + d) / 8
}

/// VCx 比较器
pub struct Vc<'p> {
    regs: &'p pac::Vc,
    refs: &'p pac::VcRef,
    /// 1..=4，仅用于文档/调试与共用参考源说明
    index: u8,
}

impl<'p> Vc<'p> {
    /// 初始化（使能 APBEN1.VC 时钟；比较器保持关闭）
    ///
    /// `refs` 传 `p.vc12ref`（VC1/VC2）或 `p.vc34ref`（VC3/VC4）。
    pub fn new(regs: &'p pac::Vc, refs: &'p pac::VcRef, rcc: &mut Rcc, index: u8) -> Self {
        rcc.enable_vc();
        Vc { regs, refs, index }
    }

    /// 实例编号（1..=4）
    pub fn index(&self) -> u8 {
        self.index
    }

    // ---------------- 分压参考源 ----------------

    /// 使能分压器：`div` = 0..=15，`vcore=true` 选 ≈1.6V 否则 VDDA
    pub fn ref_enable(&mut self, div: u8, vcore: bool) {
        let mut bits = v::REF_EN | (div as u32 & v::REF_DIV_MASK);
        if vcore {
            bits |= v::REF_VIN_VCORE;
        }
        self.refs
            .cr
            .modify(|v| (v & !(v::REF_DIV_MASK | v::REF_VIN_VCORE)) | bits);
    }

    /// 关闭分压器
    pub fn ref_disable(&mut self) {
        self.refs.cr.modify(|v| v & !v::REF_EN);
    }

    /// 当前分压配置（0=VDDA，返回 (div, vcore)）
    pub fn ref_config(&self) -> (u8, bool) {
        let v = self.refs.cr.read();
        ((v & v::REF_DIV_MASK) as u8, v & v::REF_VIN_VCORE != 0)
    }

    // ---------------- 输入/特性配置 ----------------

    /// 选择正端/负端输入
    pub fn select_inputs(&mut self, inp: NpInput, inn: NnInput) {
        self.regs.cr0.modify(|v| {
            (v & !(v::CR0_INP_MASK | v::CR0_INN_MASK))
                | (inp.code() << v::CR0_INP_POS)
                | (inn.code() << v::CR0_INN_POS)
        });
    }

    /// 响应速度（高速会硬件自动使能 BGR）
    pub fn set_speed(&mut self, speed: Speed) {
        match speed {
            Speed::Low => self.regs.cr0.modify(|v| v & !v::CR0_RESP),
            Speed::High => self.regs.cr0.modify(|v| v | v::CR0_RESP),
        }
    }

    /// 迟滞窗口（约 20mV）
    pub fn set_hysteresis(&mut self, on: bool) {
        if on {
            self.regs.cr0.modify(|v| v | v::CR0_HYS);
        } else {
            self.regs.cr0.modify(|v| v & !v::CR0_HYS);
        }
    }

    /// 输出极性（true = 反相：正端>负端输出低）
    pub fn set_inverted(&mut self, invert: bool) {
        if invert {
            self.regs.cr0.modify(|v| v | v::CR0_POL);
        } else {
            self.regs.cr0.modify(|v| v & !v::CR0_POL);
        }
    }

    /// 窗口比较（VCx_OUTW = VC1_OUTP 与 VC2_OUTP 的组合，仅 VC1/VC2 组有效）
    pub fn set_window(&mut self, on: bool) {
        if on {
            self.regs.cr0.modify(|v| v | v::CR0_WINDOW);
        } else {
            self.regs.cr0.modify(|v| v & !v::CR0_WINDOW);
        }
    }

    /// 数字滤波：时钟源 + 时间
    pub fn set_filter(&mut self, clk: FilterClock, time: FilterTime) {
        self.regs.cr1.modify(|v| {
            let mut n = (v & !v::CR1_FLTTIME_MASK) | time.code();
            n = match clk {
                FilterClock::Pclk => n | v::CR1_FLTCLK_PCLK,
                FilterClock::Lsi => n & !v::CR1_FLTCLK_PCLK,
            };
            n
        });
    }

    /// 中断触发方式（`irq::CR1_RISEIE` 等可或组合；传 0 关闭全部触发）
    pub fn set_irq_triggers(&mut self, triggers: u32) {
        let mask = v::CR1_RISEIE | v::CR1_FALLIE | v::CR1_HIGHIE;
        self.regs.cr1.modify(|v| (v & !mask) | (triggers & mask));
    }

    /// BLANK 窗口触发源（CR2，每位见 `pac::vc::CR2_*`）
    pub fn set_blank_triggers(&mut self, mask: u32) {
        self.regs.cr2.write(mask);
    }

    /// 全局中断使能（CR0.IE）
    pub fn enable_irq(&mut self, on: bool) {
        if on {
            self.regs.cr0.modify(|v| v | v::CR0_IE);
        } else {
            self.regs.cr0.modify(|v| v & !v::CR0_IE);
        }
    }

    // ---------------- 运行 ----------------

    /// 使能比较器
    pub fn enable(&mut self) {
        self.regs.cr0.modify(|v| v | v::CR0_EN);
    }

    /// 关闭比较器
    pub fn disable(&mut self) {
        self.regs.cr0.modify(|v| v & !v::CR0_EN);
    }

    /// 滤波后的比较输出电平（SR.FLTV）
    pub fn output_high(&self) -> bool {
        self.regs.sr.read() & v::SR_FLTV != 0
    }

    /// 取中断标志并清除（SR.INTF 为 RW0：写 0 清除）
    pub fn take_irq(&mut self) -> bool {
        let fired = self.regs.sr.read() & v::SR_INTF != 0;
        if fired {
            self.regs.sr.write(0); // 写 0 → 清 INTF
        }
        fired
    }

    /// 仅清除中断标志
    pub fn clear_irq(&mut self) {
        self.regs.sr.write(0);
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Vc {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_voltage_formula() {
        // 手册公式：Vout = Vin × (1+DIV)/8
        assert_eq!(ref_voltage_mv(3300, 0), 412); // 3300/8
        assert_eq!(ref_voltage_mv(3300, 3), 1650); // ×4/8
        assert_eq!(ref_voltage_mv(3300, 7), 3300); // ×8/8 = 满量程
        assert_eq!(ref_voltage_mv(3300, 15), 6600); // ×16/8 = 2×VDDA（超量程）
        assert_eq!(ref_voltage_mv(1600, 3), 800); // Vcore 档
    }

    #[test]
    fn div_masked_to_4bit() {
        // 传入超范围值时按低 4 位处理，不会溢出到 VIN 位
        assert_eq!(ref_voltage_mv(3300, 0x13), ref_voltage_mv(3300, 3));
    }

    #[test]
    fn filter_codes_match_manual_table() {
        assert_eq!(FilterTime::Off.code(), 0);
        assert_eq!(FilterTime::S1N2.code(), 1);
        assert_eq!(FilterTime::S2N6.code(), 4);
        assert_eq!(FilterTime::S8N8.code(), 9);
        assert_eq!(FilterTime::S16N8.code(), 12);
        assert_eq!(FilterTime::S32N8.code(), 15);
    }

    #[test]
    fn input_codes_match_manual() {
        assert_eq!(NpInput::Ch3.code(), 3);
        assert_eq!(NnInput::Dac.code(), 2);
        assert_eq!(NnInput::RefDiv.code(), 3);
    }

    #[test]
    fn cr0_field_positions() {
        // INP[13:12]、INN[9:8] 不能互相覆盖
        assert_eq!(v::CR0_INP_MASK & v::CR0_INN_MASK, 0);
        assert_eq!(v::CR0_INP_MASK >> v::CR0_INP_POS, 0x3);
    }
}
