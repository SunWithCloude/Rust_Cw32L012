//! DAC 数字模拟转换器（§26）
//!
//! 双通道 12-bit，本模块走**无触发直写模式**（TEN=0）：写 DHR 后数据
//! 经 1 个 APCLK 自动转入 DOR 并出现在输出上——正弦波/斜波的最简姿势。
//! 触发模式（TSEL+SWTRIG）与噪声/三角波发生器可经 `regs()` 原始访问。
//!
//! # 通道与引脚（手册 §26 / 表 9-2）
//!
//! | 通道 | 引脚输出       | 内部互联            |
//! |------|----------------|---------------------|
//! | CH1  | PA4（DAC_OUT1，与 OPA 共享，需 `CR1.OUT1`） | ADC1_IN13（DAC_OUT1） |
//! | CH2  | PA5（DAC_OUT2，与 OPA 共享，需 `CR1.OUT2`） | **ADC1_IN12（DAC_OUT2）** |
//!
//! ⚠ PA4/PA5 同时是 SPI1 的 SCK/MOSI（本板屏幕占用中）——引脚输出会与
//! SPI 冲突；但**内部互联（ADC 回环）不经过引脚，随时可用**。
//!
//! # 用法（DAC → ADC 内部回环自测，无需任何外部接线）
//!
//! ```ignore
//! let mut dac = Dac::new(p.dac, &mut rcc);
//! dac.set_ch2(2048)?;                       // ≈ VDDA/2
//! let raw = adc.convert(AdcChannel::In(12), SampleCycles::S24)?;
//! // raw ≈ 2048（±几 LSB）
//! ```

use crate::pac::{self, dac as dc};
use crate::rcc::Rcc;

/// DAC 错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueOutOfRange;

/// 12 位值域检查
fn check12(v: u16) -> Result<(), ValueOutOfRange> {
    if v > 0xFFF {
        Err(ValueOutOfRange)
    } else {
        Ok(())
    }
}

/// DAC（双通道，无触发直写模式）
pub struct Dac<'p> {
    regs: &'p pac::Dac,
    ch1_on: bool,
    ch2_on: bool,
}

impl<'p> Dac<'p> {
    /// 获取 DAC 并使能时钟（APBEN2[10]）
    pub fn new(regs: &'p pac::Dac, rcc: &mut Rcc) -> Self {
        rcc.enable_dac();
        Dac {
            regs,
            ch1_on: false,
            ch2_on: false,
        }
    }

    /// 写通道 1（0..4095；未使能通道时自动使能）
    pub fn set_ch1(&mut self, value: u16) -> Result<(), ValueOutOfRange> {
        check12(value)?;
        if !self.ch1_on {
            self.regs.cr0.modify(|v| v | dc::CR0_EN1);
            self.ch1_on = true;
        }
        self.regs.dhr12r1.write(value as u32);
        Ok(())
    }

    /// 写通道 2（0..4095；未使能通道时自动使能）
    pub fn set_ch2(&mut self, value: u16) -> Result<(), ValueOutOfRange> {
        check12(value)?;
        if !self.ch2_on {
            self.regs.cr0.modify(|v| v | dc::CR0_EN2);
            self.ch2_on = true;
        }
        self.regs.dhr12r2.write(value as u32);
        Ok(())
    }

    /// 同时写双通道（一次总线写）
    pub fn set_both(&mut self, ch1: u16, ch2: u16) -> Result<(), ValueOutOfRange> {
        check12(ch1)?;
        check12(ch2)?;
        let mut en = 0u32;
        if !self.ch1_on {
            en |= dc::CR0_EN1;
            self.ch1_on = true;
        }
        if !self.ch2_on {
            en |= dc::CR0_EN2;
            self.ch2_on = true;
        }
        if en != 0 {
            self.regs.cr0.modify(|v| v | en);
        }
        self.regs.dhr12rd.write((ch2 as u32) << 16 | ch1 as u32);
        Ok(())
    }

    /// 读通道 1 实际输出值（DOR，确认 DHR→DOR 传输完成）
    pub fn output1(&self) -> u16 {
        (self.regs.dor1.read() & dc::DHR12_MASK) as u16
    }

    /// 读通道 2 实际输出值
    pub fn output2(&self) -> u16 {
        (self.regs.dor2.read() & dc::DHR12_MASK) as u16
    }

    /// 通道 1 输出到 PA4 引脚（⚠ 与 SPI1_SCK/OPA 冲突，慎用）
    pub fn enable_pin1(&mut self) {
        self.regs.cr1.modify(|v| v | dc::CR1_OUT1);
    }

    /// 通道 2 输出到 PA5 引脚（⚠ 与 SPI1_MOSI/OPA 冲突，慎用）
    pub fn enable_pin2(&mut self) {
        self.regs.cr1.modify(|v| v | dc::CR1_OUT2);
    }

    /// 寄存器块原始访问（触发/波形发生器逃生口）
    pub fn regs(&mut self) -> &'p pac::Dac {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_range() {
        assert!(check12(0).is_ok());
        assert!(check12(0xFFF).is_ok());
        assert_eq!(check12(0x1000), Err(ValueOutOfRange));
        assert_eq!(check12(0xFFFF), Err(ValueOutOfRange));
    }
}
