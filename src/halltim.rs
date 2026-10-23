//! HALLTIM 霍尔传感器专用定时器（手册 §18）
//!
//! 24bit 递增计数器：计数时钟 = PCLK/DIV（DIV ∈ {1,2,4,8}）。
//! 三个捕获通道各带两级滤波（FLT1EN 低通 + FLT2LEN 数字滤波）；
//! 任一通道电平变化即把 CNT 存进 `WIDTH` 并**硬件清零 CNT**（CAPF 置位）；
//! CNT<CCR 时 OUT 引脚输出高电平（PWM），CNT==CCR 产生 MATCH 脉冲。
//!
//! # 用法（PWM 输出，PB00=HALLTIM_OUT AF9）
//!
//! ```ignore
//! use cw32l012_hal::halltim::{Halltim, HallDiv};
//!
//! let mut h = Halltim::new(p.halltim, &mut rcc, HallDiv::Div1);
//! h.set_period(0xFFFF);        // 24bit，PCLK 4MHz / 65536 ≈ 61Hz
//! h.set_duty_permille(300);    // 30%
//! h.enable();
//! ```
//!
//! # 中断
//!
//! HALLTIM 与 BTIM3 共享 IRQ22（`BTIM3_HALLTIM_irq`），ISR 中需自行区分标志。

use crate::pac::{self, halltim as ht};
use crate::rcc::Rcc;

/// 计数时钟分频（CR.DIV）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HallDiv {
    /// PCLK 直通
    Div1,
    /// PCLK/2
    Div2,
    /// PCLK/4
    Div4,
    /// PCLK/8
    Div8,
}

impl HallDiv {
    #[inline]
    fn code(self) -> u32 {
        match self {
            HallDiv::Div1 => 0,
            HallDiv::Div2 => 1,
            HallDiv::Div4 => 2,
            HallDiv::Div8 => 3,
        }
    }

    /// 分频系数
    #[inline]
    pub fn factor(self) -> u32 {
        1 << self.code()
    }
}

/// 触发输出源（CR.MMS），用于同步 ATIM/ADC 等
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trgo {
    /// 计数器溢出脉冲
    Overflow,
    /// PWM 波形
    Pwm,
    /// CCR 比较匹配脉冲
    Match,
    /// HALL 信号变化（捕获）脉冲
    HallChange,
    /// 滤波后的 CH3 波形
    Ch3,
    /// 滤波后的 CH2 波形
    Ch2,
    /// 滤波后的 CH1 波形
    Ch1,
    /// 三通道滤波信号异或
    Xor,
}

impl Trgo {
    #[inline]
    fn code(self) -> u32 {
        match self {
            Trgo::Overflow => 0,
            Trgo::Pwm => 1,
            Trgo::Match => 2,
            Trgo::HallChange => 3,
            Trgo::Ch3 => 4,
            Trgo::Ch2 => 5,
            Trgo::Ch1 => 6,
            Trgo::Xor => 7,
        }
    }
}

/// 数值超出 24bit 计数量程
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

/// HALLTIM 霍尔接口定时器
pub struct Halltim<'p> {
    regs: &'p pac::Halltim,
    pclk_hz: u32,
    div: HallDiv,
}

impl<'p> Halltim<'p> {
    /// 初始化（使能 APBEN2.HALLTIM 时钟 + 配置分频；模块保持未使能）
    pub fn new(regs: &'p pac::Halltim, rcc: &mut Rcc, div: HallDiv) -> Self {
        rcc.enable_halltim();
        let pclk_hz = rcc.clocks().pclk_hz;
        regs.cr
            .modify(|v| (v & !ht::CR_DIV_MASK & !ht::CR_EN) | (div.code() << ht::CR_DIV_POS));
        Halltim { regs, pclk_hz, div }
    }

    /// 计数时钟频率（Hz）
    pub fn count_hz(&self) -> u32 {
        self.pclk_hz / self.div.factor()
    }

    /// 使能模块（EN=1）
    pub fn enable(&mut self) {
        self.regs.cr.modify(|v| v | ht::CR_EN);
    }

    /// 停止模块（EN=0）
    pub fn disable(&mut self) {
        self.regs.cr.modify(|v| v & !ht::CR_EN);
    }

    /// 设置自动重载值（24bit；超范围返回 `Err`）
    pub fn set_period(&mut self, arr: u32) -> Result<(), OutOfRange> {
        if arr > ht::DATA_MASK {
            return Err(OutOfRange);
        }
        self.regs.arr.write(arr);
        Ok(())
    }

    /// 当前计数周期（Hz）
    pub fn period_hz(&self, arr: u32) -> u32 {
        self.count_hz() / (arr.saturating_add(1))
    }

    /// 设置比较值（CNT<CCR 输出高）
    pub fn set_compare(&mut self, ccr: u32) -> Result<(), OutOfRange> {
        if ccr > ht::DATA_MASK {
            return Err(OutOfRange);
        }
        self.regs.ccr.write(ccr);
        Ok(())
    }

    /// 按千分比设置占空比（0..=1000），依赖当前 ARR —— 返回实际 CCR
    pub fn set_duty_permille(&mut self, permille: u32) -> u32 {
        let arr = self.regs.arr.read() & ht::DATA_MASK;
        let ccr = ((arr as u64 + 1) * permille as u64 / 1000) as u32;
        self.regs.ccr.write(ccr & ht::DATA_MASK);
        ccr & ht::DATA_MASK
    }

    /// 读计数值（24bit）
    pub fn count(&self) -> u32 {
        self.regs.cnt.read() & ht::DATA_MASK
    }

    /// 计数器清零（CNT 为 RW0 语义：写 0 清零）
    pub fn reset_count(&self) {
        self.regs.cnt.write(0);
    }

    /// 最近一次捕获的 HALL 脉冲宽度（计数单位，24bit）
    pub fn pulse_width(&self) -> u32 {
        self.regs.width.read() & ht::DATA_MASK
    }

    /// 第一级滤波器（5/7 低通）
    pub fn set_filter1(&mut self, on: bool) {
        if on {
            self.regs.cr.modify(|v| v | ht::CR_FLT1EN);
        } else {
            self.regs.cr.modify(|v| v & !ht::CR_FLT1EN);
        }
    }

    /// 第二级数字滤波宽度（滤除窄于 `len` 个计数时钟的脉冲；`len=0` 关闭）
    pub fn set_filter2(&mut self, len: u16) {
        let len = (len as u32) & (ht::CR_FLT2LEN_MASK >> ht::CR_FLT2LEN_POS);
        self.regs
            .cr
            .modify(|v| (v & !ht::CR_FLT2LEN_MASK) | (len << ht::CR_FLT2LEN_POS));
    }

    /// 选择触发输出源（TRGO）
    pub fn set_trgo(&mut self, src: Trgo) {
        self.regs
            .cr
            .modify(|v| (v & !ht::CR_MMS_MASK) | (src.code() << ht::CR_MMS_POS));
    }

    /// 软件捕获：生成一次捕获事件（CNT→WIDTH、CNT 清零、CAPF 置位）
    pub fn software_capture(&self) {
        self.regs.cr.modify(|v| v | ht::CR_SOFTCAP);
    }

    /// 三通道**原始**电平（STATE.CHxS）
    pub fn levels(&self) -> (bool, bool, bool) {
        let s = self.regs.state.read();
        (
            s & ht::STATE_CH1S != 0,
            s & ht::STATE_CH2S != 0,
            s & ht::STATE_CH3S != 0,
        )
    }

    /// 三通道**滤波后**电平（STATE.CHxF）
    pub fn filtered_levels(&self) -> (bool, bool, bool) {
        let s = self.regs.state.read();
        (
            s & ht::STATE_CH1F != 0,
            s & ht::STATE_CH2F != 0,
            s & ht::STATE_CH3F != 0,
        )
    }

    /// 原始 ISR
    pub fn isr(&self) -> u32 {
        self.regs.isr.read()
    }

    /// 使能中断源（CAPIE/OVIE/MATCHIE，可或组合）
    pub fn enable_irq(&mut self, flags: u32) {
        self.regs.dier.modify(|v| v | (flags & ht::FLAGS_MASK));
    }

    /// 关闭中断源
    pub fn disable_irq(&mut self, flags: u32) {
        self.regs.dier.modify(|v| v & !(flags & ht::FLAGS_MASK));
    }

    /// 清中断标志（ICR 为 R1W0：对应位写 0 清除，其余位写 1 无动作）
    pub fn clear_irq(&mut self, flags: u32) {
        self.regs
            .icr
            .write(ht::FLAGS_MASK & !(flags & ht::FLAGS_MASK));
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Halltim {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div_codes_and_factors() {
        assert_eq!(HallDiv::Div1.code(), 0b00);
        assert_eq!(HallDiv::Div2.code(), 0b01);
        assert_eq!(HallDiv::Div4.code(), 0b10);
        assert_eq!(HallDiv::Div8.code(), 0b11);
        assert_eq!(HallDiv::Div8.factor(), 8);
    }

    #[test]
    fn trgo_codes_match_manual() {
        assert_eq!(Trgo::Overflow.code(), 0);
        assert_eq!(Trgo::Pwm.code(), 1);
        assert_eq!(Trgo::Match.code(), 2);
        assert_eq!(Trgo::HallChange.code(), 3);
        assert_eq!(Trgo::Xor.code(), 7);
    }

    #[test]
    fn icr_clear_value_only_zeroes_target() {
        // R1W0：清 CAPF 时其余位写 1
        assert_eq!(ht::FLAGS_MASK & !ht::CAPF, 0x6);
        assert_eq!(ht::FLAGS_MASK & !ht::OVF, 0x5);
        assert_eq!(ht::FLAGS_MASK & !(ht::CAPF | ht::OVF | ht::MATCHF), 0x0);
    }

    #[test]
    fn data_mask_is_24bit() {
        assert_eq!(ht::DATA_MASK, 0xFF_FFFF);
        assert_eq!(ht::CR_FLT2LEN_MASK >> ht::CR_FLT2LEN_POS, 0x7FFF);
    }

    #[test]
    fn duty_permille_math() {
        // ARR=999 → 周期 1000 计数；30% → CCR=300
        let arr = 999u32;
        let ccr = (arr as u64 + 1) * 300 / 1000;
        assert_eq!(ccr, 300);
        // ARR=0xFFFF（65536）→ 50% = 32768
        let ccr = (0xFFFFu64 + 1) * 500 / 1000;
        assert_eq!(ccr, 32768);
    }
}
