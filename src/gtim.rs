//! GTIM 通用定时器（§16）——PWM 输出
//!
//! 4 实例 × 4 通道，16-bit PSC/ARR/CCR，STM32 风格寄存器模型。
//! 本模块封装**边沿对齐 PWM 输出**（模式 1：CNT<CCR 时输出有效电平），
//! 捕获/编码器/从模式可经 `regs()` 原始访问。
//!
//! # PWM 频率与占空比
//!
//! `f_pwm = PCLK / ((PSC+1) × (ARR+1))`，`占空比 = CCR / (ARR+1)`。
//! `start_pwm` 自动选取 PSC 使 ARR 尽量大（量化精度最高），
//! 占空比以**万分比**（0..10000）设置，避免浮点。
//!
//! # 引脚（手册表 9-2，均需配为对应 AF 复用输出）
//!
//! | 通道 | 常用引脚（AF6=GTIM1/2/3/4 通道见各列） |
//! |------|----------------------------------------|
//! | GTIM1_CH1 | PB04 (AF6)、PA00 (AF10)、PA08 等 |
//! | GTIM1_CH3 | PA03 (AF6) 等 |
//!
//! 完整映射见手册表 9-2；选脚前先查目标通道的 AF 编号。
//!
//! # 用法（GTIM1_CH1 @ PB04，1kHz、30% 占空比）
//!
//! ```ignore
//! let mut tim = Gtim::new(p.gtim1, &mut rcc);
//! tim.start_pwm(PwmChannel::Ch1, pclk_hz, 1_000, 3_000)?; // 30.00%
//! // （引脚需另行 PB04→AF6 复用；见 examples/gtim_pwm.rs）
//! ```

use crate::pac::{self, gtim as gt};
use crate::rcc::Rcc;

/// PWM 通道
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PwmChannel {
    Ch1,
    Ch2,
    Ch3,
    Ch4,
}

/// PWM 配置错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PwmError {
    /// 频率/占空比超出可实现范围
    OutOfRange,
    /// 占空比 > 10000（万分比）
    BadDuty,
}

/// GTIM 定时器（PWM 输出）
pub struct Gtim<'p> {
    regs: &'p pac::Gtim,
}

impl<'p> Gtim<'p> {
    /// 获取 GTIM 实例并按基地址使能 APB1 时钟
    pub fn new(regs: &'p pac::Gtim, rcc: &mut Rcc) -> Self {
        let addr = regs as *const _ as usize;
        if addr == pac::GTIM1_BASE {
            rcc.enable_gtim1();
        } else if addr == pac::GTIM2_BASE {
            rcc.enable_gtim2();
        } else if addr == pac::GTIM3_BASE {
            rcc.enable_gtim3();
        } else if addr == pac::GTIM4_BASE {
            rcc.enable_gtim4();
        }
        Gtim { regs }
    }

    /// 启动指定通道 PWM（边沿对齐，模式 1，先低后有效电平极性）
    ///
    /// * `freq_hz` — PWM 频率（PCLK/65536 ≤ freq ≤ PCLK/2 可实现）
    /// * `duty_bp` — 占空比万分比（0..=10000）
    ///
    /// # Panics
    /// 无效占空比 panic；频率不可实现返回 [`PwmError::OutOfRange`]。
    pub fn start_pwm(
        &mut self,
        ch: PwmChannel,
        pclk_hz: u32,
        freq_hz: u32,
        duty_bp: u16,
    ) -> Result<(), PwmError> {
        if duty_bp > 10_000 {
            return Err(PwmError::BadDuty);
        }
        let (psc, arr) = calc_pwm_psc_arr(pclk_hz, freq_hz).ok_or(PwmError::OutOfRange)?;
        let ccr = ((arr as u32 + 1) * duty_bp as u32 / 10_000) as u16;

        self.regs.cr1.write(0); // 停止计数以便配置
        self.regs.psc.write(psc as u32);
        self.regs.arr.write(arr as u32);
        self.set_ccr(ch, ccr);

        // 通道输出比较模式：PWM1 + 预装载；CCER 使能输出（高有效）
        match ch {
            PwmChannel::Ch1 => {
                self.regs.ccmr1.modify(|v| {
                    (v & !(gt::OC1M_MASK | gt::OC1PE | 0x3)) | gt::OC1M_PWM1 | gt::OC1PE
                });
                self.regs.ccer.modify(|v| (v & !gt::CC1P) | gt::CC1E);
            }
            PwmChannel::Ch2 => {
                self.regs.ccmr1.modify(|v| {
                    (v & !(gt::OC2M_MASK | gt::OC2PE | (0x3 << 8))) | gt::OC2M_PWM1 | gt::OC2PE
                });
                self.regs.ccer.modify(|v| (v & !gt::CC2P) | gt::CC2E);
            }
            PwmChannel::Ch3 => {
                self.regs.ccmr2.modify(|v| {
                    (v & !(gt::OC3M_MASK | gt::OC3PE | 0x3)) | gt::OC3M_PWM1 | gt::OC3PE
                });
                self.regs.ccer.modify(|v| (v & !gt::CC3P) | gt::CC3E);
            }
            PwmChannel::Ch4 => {
                self.regs.ccmr2.modify(|v| {
                    (v & !(gt::OC4M_MASK | gt::OC4PE | (0x3 << 8))) | gt::OC4M_PWM1 | gt::OC4PE
                });
                self.regs.ccer.modify(|v| (v & !gt::CC4P) | gt::CC4E);
            }
        }

        // UG 重装影子寄存器 → ARPE + EN 启动
        self.regs.egr.write(gt::EGR_UG);
        self.regs.cr1.write(gt::CR1_ARPE | gt::CR1_EN);
        Ok(())
    }

    /// 运行中修改占空比（万分比；ARPE 已开 → 下一更新周期生效）
    pub fn set_duty(&mut self, ch: PwmChannel, duty_bp: u16) -> Result<(), PwmError> {
        if duty_bp > 10_000 {
            return Err(PwmError::BadDuty);
        }
        let arr = self.regs.arr.read() & 0xFFFF;
        let ccr = ((arr + 1) * duty_bp as u32 / 10_000) as u16;
        self.set_ccr(ch, ccr);
        Ok(())
    }

    /// 直接写 CCR（原始占空比控制）
    pub fn set_ccr(&mut self, ch: PwmChannel, ccr: u16) {
        match ch {
            PwmChannel::Ch1 => self.regs.ccr1.write(ccr as u32),
            PwmChannel::Ch2 => self.regs.ccr2.write(ccr as u32),
            PwmChannel::Ch3 => self.regs.ccr3.write(ccr as u32),
            PwmChannel::Ch4 => self.regs.ccr4.write(ccr as u32),
        }
    }

    /// 读 CCR（回读校验用）
    pub fn ccr(&self, ch: PwmChannel) -> u16 {
        match ch {
            PwmChannel::Ch1 => self.regs.ccr1.read() as u16,
            PwmChannel::Ch2 => self.regs.ccr2.read() as u16,
            PwmChannel::Ch3 => self.regs.ccr3.read() as u16,
            PwmChannel::Ch4 => self.regs.ccr4.read() as u16,
        }
    }

    /// 当前计数值（观测 CNT 在 0..ARR 间滚动 = PWM 在跑）
    pub fn count(&self) -> u16 {
        self.regs.cnt.read() as u16
    }

    /// 读 ARR/PSC 配置（回读校验用）
    pub fn period_config(&self) -> (u16, u16) {
        (self.regs.arr.read() as u16, self.regs.psc.read() as u16)
    }

    /// 停止 PWM（保留配置）
    pub fn stop(&mut self) {
        self.regs.cr1.modify(|v| v & !gt::CR1_EN);
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Gtim {
        self.regs
    }

    /// 取出绑定到指定通道的 PWM 句柄
    ///
    /// 句柄独占借用定时器，因此天然保证“同一时刻只有一个通道在被改写”。
    /// 它的主要用途是满足 `embedded_hal::pwm::SetDutyCycle`（该 trait 要求
    /// 占空比挂在类型上，而不是方法的参数里）。
    pub fn pwm_pin(&mut self, ch: PwmChannel) -> PwmPin<'_, 'p> {
        PwmPin { tim: self, ch }
    }
}

/// 绑定到固定通道的 PWM 句柄
///
/// 满量程固定为 `10000`（万分比），因此可以直接喂给 `embedded-hal` 的
/// `SetDutyCycle`（它按 `max_duty_cycle()` 归一化）。
pub struct PwmPin<'a, 'p> {
    tim: &'a mut Gtim<'p>,
    ch: PwmChannel,
}

impl<'a, 'p> PwmPin<'a, 'p> {
    /// 满量程（万分比）
    pub const MAX_DUTY: u16 = 10_000;

    /// 本句柄绑定的通道
    pub fn channel(&self) -> PwmChannel {
        self.ch
    }

    /// 满量程值（`embedded_hal::pwm::SetDutyCycle::max_duty_cycle`）
    pub fn max_duty(&self) -> u16 {
        Self::MAX_DUTY
    }

    /// 设置占空比（万分比 0..=10000，下一更新周期生效）
    pub fn set_duty(&mut self, duty_bp: u16) -> Result<(), PwmError> {
        self.tim.set_duty(self.ch, duty_bp)
    }

    /// 读回当前占空比（万分比，由 CCR/ARR 反算）
    pub fn duty(&self) -> u16 {
        let (arr, _) = self.tim.period_config();
        let ccr = self.tim.ccr(self.ch) as u32;
        let period = arr as u32 + 1;
        if period == 0 {
            return 0;
        }
        ((ccr * 10_000) / period) as u16
    }

    /// 回到定时器本体
    pub fn release(self) -> &'a mut Gtim<'p> {
        self.tim
    }
}

/// 计算 PWM 的 (PSC, ARR)：PSC 取最小值使 ARR ≤ 0xFFFF。
pub fn calc_pwm_psc_arr(pclk_hz: u32, freq_hz: u32) -> Option<(u16, u16)> {
    if freq_hz == 0 || pclk_hz == 0 {
        return None;
    }
    let mut psc = ((pclk_hz as u64 + 65_535 * freq_hz as u64 - 1) / (65_536 * freq_hz as u64))
        .saturating_sub(1);
    while psc <= 0xFFFF {
        let ck_cnt = pclk_hz / (psc as u32 + 1);
        let ticks = (ck_cnt as u64 + freq_hz as u64 / 2) / freq_hz as u64;
        if (2..=65_536).contains(&ticks) {
            return Some((psc as u16, (ticks - 1) as u16));
        }
        psc += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pwm_1khz_at_4mhz() {
        // 4M/1k = 4000 ticks ≤ 65536 → PSC=0 ARR=3999
        let (psc, arr) = calc_pwm_psc_arr(4_000_000, 1_000).unwrap();
        assert_eq!(psc, 0);
        assert_eq!(arr, 3999);
    }

    #[test]
    fn pwm_20khz_at_96mhz() {
        // 96M/20k = 4800 ticks → PSC=0 ARR=4799
        let (psc, arr) = calc_pwm_psc_arr(96_000_000, 20_000).unwrap();
        assert_eq!(psc, 0);
        assert_eq!(arr, 4799);
    }

    #[test]
    fn pwm_slow_needs_prescaler() {
        // 2Hz@96MHz：PSC=733（ceil(96M/131072)-1=732）→ ck_cnt=96M/733≈131k
        let (psc, arr) = calc_pwm_psc_arr(96_000_000, 2).unwrap();
        let ck = 96_000_000u64 / (psc as u64 + 1);
        let f = ck / (arr as u64 + 1);
        assert!((f as i64 - 2).abs() <= 1, "实际 {f}Hz 偏离 2Hz");
    }

    #[test]
    fn pwm_out_of_range() {
        // f > PCLK/2 不可实现（ticks<2）
        assert!(calc_pwm_psc_arr(4_000_000, 3_000_000).is_none());
        assert!(calc_pwm_psc_arr(4_000_000, 0).is_none());
    }

    #[test]
    fn duty_mapping() {
        // ARR=3999：50% → CCR = 4000×5000/10000 = 2000
        let arr: u32 = 3999;
        let ccr = ((arr + 1) * 5000 / 10_000) as u16;
        assert_eq!(ccr, 2000);
        // 100% → CCR = 4000（=ARR+1，CNT 永不大于 CCR → 恒有效电平）
        let ccr = ((arr + 1) * 10_000 / 10_000) as u16;
        assert_eq!(ccr, 4000);
    }
}
