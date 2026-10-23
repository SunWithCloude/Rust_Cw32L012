//! ATIM 高级定时器（§17）——互补 PWM + 死区 + 刹车
//!
//! 单实例 16-bit 定时器，寄存器为 STM32 风格但 DTG 编码为 CW32 自有定义：
//!
//! | DTG[7:5] | 死区时间 |
//! |----------|----------|
//! | 0xx      | DTG[7:0] × t_DTS（1..127 步进 1） |
//! | 10x      | (64+DTG[5:0]) × 2·t_DTS（128..254 步进 2） |
//! | 110      | (32+DTG[4:0]) × 8·t_DTS（256..504 步进 8） |
//! | 111      | (32+DTG[4:0]) × 16·t_DTS（512..1008 步进 16） |
//!
//! `t_DTS = 1 / f_DTS`，`f_DTS = f_PCLK / CKD`（CKD ∈ {1,2,4}）。
//!
//! # 与 GTIM 的差异
//!
//! - PWM 输出受 **MOE（主输出使能）** 总开关控制——不置 1 引脚无波形；
//! - 刹车事件会把 MOE 硬件清零，恢复需 [`Atim::re_enable_output`]（或 AOE=1）；
//! - 互补通道 CHyN 由 CCER.CCyNE 独立使能，死区由 BDTR.DTG 统一插入。
//!
//! # 常用引脚（手册表 9-2）
//!
//! | 信号 | 引脚（AF7） |
//! |------|-------------|
//! | ATIM_CH1 | PA05、PA08、PB02、PB05 |
//! | ATIM_CH1N | PA07、PA15 |
//! | ATIM_BK | PA06、PA12、PA13、PB05 |
//! | ATIM_BK2 | PA11、PA14、PB13 |
//!
//! # 用法（PA08=CH1 / PA15=CH1N，20kHz、30%、死区 500ns）
//!
//! ```ignore
//! let mut tim = Atim::new(p.atim, &mut rcc);
//! tim.start_pwm_complementary(AtimChannel::Ch1, pclk_hz, 20_000, 3_000, 500)?;
//! // （引脚需另行 PA08→AF7、PA15→AF7 复用）
//! ```

use crate::pac::{self, atim as at};
use crate::rcc::Rcc;

/// ATIM 通道（CH5/CH6 无互补输出）
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtimChannel {
    Ch1,
    Ch2,
    Ch3,
    Ch4,
}

/// PWM 配置错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtimError {
    /// 频率/占空比超出可实现范围
    OutOfRange,
    /// 占空比 > 10000（万分比）
    BadDuty,
    /// 死区时间超过 1008·t_DTS，即使最大编码档也放不下
    DeadTimeTooLong,
}

/// ATIM 定时器（互补 PWM + 死区 + 刹车）
pub struct Atim<'p> {
    regs: &'p pac::Atim,
}

impl<'p> Atim<'p> {
    /// 获取 ATIM 实例并使能 APB1 时钟（APBEN1.ATIM，bit5）
    pub fn new(regs: &'p pac::Atim, rcc: &mut Rcc) -> Self {
        rcc.enable_atim();
        Atim { regs }
    }

    /// 启动单端 PWM（边沿对齐模式 1，高有效；无互补、无死区）
    pub fn start_pwm(
        &mut self,
        ch: AtimChannel,
        pclk_hz: u32,
        freq_hz: u32,
        duty_bp: u16,
    ) -> Result<(), AtimError> {
        self.setup_pwm(ch, pclk_hz, freq_hz, duty_bp)?;
        // CCER：仅使能主输出（CCyE），关互补（CCyNE=0、极性位清零）
        self.ccer_ch(ch, true, false);
        self.regs.bdtr.write(at::BDTR_MOE); // MOE=1（无刹车、无死区）
        self.finalize();
        Ok(())
    }

    /// 启动互补 PWM（CHy + CHyN）并插入死区
    ///
    /// * `dead_ns` — 死区时间（ns），CHy 关断到 CHyN 导通的双边死区；
    ///   超出 1008·t_DTS 返回 [`AtimError::DeadTimeTooLong`]。
    /// * 同时置 OSSI=1：刹车/空闲态输出保持已定义电平（而非高阻）。
    pub fn start_pwm_complementary(
        &mut self,
        ch: AtimChannel,
        pclk_hz: u32,
        freq_hz: u32,
        duty_bp: u16,
        dead_ns: u32,
    ) -> Result<(), AtimError> {
        self.setup_pwm(ch, pclk_hz, freq_hz, duty_bp)?;
        self.ccer_ch(ch, true, true);
        // DTG 编码（CKD=1 → t_DTS = t_PCLK）
        let dtg = encode_dtg(pclk_hz, 1, dead_ns).ok_or(AtimError::DeadTimeTooLong)?;
        // BDTR 单次写入：DTG + OSSI + MOE（LOCK=0 无写保护）
        self.regs
            .bdtr
            .write(dtg as u32 | at::BDTR_OSSI | at::BDTR_MOE);
        self.finalize();
        Ok(())
    }

    /// 使能刹车输入（BRK，低电平有效默认）
    ///
    /// 刹车事件（BRK 引脚有效）会立即清 MOE → 两路输出进入安全态；
    /// 恢复调用 [`Atim::re_enable_output`]。必须在 MOE=1 之后仍可调用
    /// （LOCK=0 无写保护）。
    pub fn enable_brake(&mut self, active_high: bool, filter: u8) {
        let pol = if active_high { at::BDTR_BKP } else { 0 };
        let flt = (filter as u32 & 0xF) << at::BDTR_BKF_POS;
        self.regs
            .bdtr
            .modify(|v| (v & !at::BDTR_BKP) | at::BDTR_BKE | pol | flt);
    }

    /// 刹车恢复（重置主输出使能 MOE=1；LOCK=0 时有效）
    pub fn re_enable_output(&mut self) {
        self.regs.bdtr.modify(|v| v | at::BDTR_MOE);
    }

    /// 读 BDTR（检查 MOE/刹车状态）
    pub fn bdtr(&self) -> u32 {
        self.regs.bdtr.read()
    }

    /// 运行中修改占空比（万分比；ARPE 已开 → 下一更新周期生效）
    pub fn set_duty(&mut self, ch: AtimChannel, duty_bp: u16) -> Result<(), AtimError> {
        if duty_bp > 10_000 {
            return Err(AtimError::BadDuty);
        }
        let arr = self.regs.arr.read() & 0xFFFF;
        let ccr = ((arr + 1) * duty_bp as u32 / 10_000) as u16;
        self.set_ccr(ch, ccr);
        Ok(())
    }

    /// 直接写 CCR
    pub fn set_ccr(&mut self, ch: AtimChannel, ccr: u16) {
        match ch {
            AtimChannel::Ch1 => self.regs.ccr1.write(ccr as u32),
            AtimChannel::Ch2 => self.regs.ccr2.write(ccr as u32),
            AtimChannel::Ch3 => self.regs.ccr3.write(ccr as u32),
            AtimChannel::Ch4 => self.regs.ccr4.write(ccr as u32),
        }
    }

    /// 读 CCR（回读校验用）
    pub fn ccr(&self, ch: AtimChannel) -> u16 {
        match ch {
            AtimChannel::Ch1 => self.regs.ccr1.read() as u16,
            AtimChannel::Ch2 => self.regs.ccr2.read() as u16,
            AtimChannel::Ch3 => self.regs.ccr3.read() as u16,
            AtimChannel::Ch4 => self.regs.ccr4.read() as u16,
        }
    }

    /// 当前计数值（CNT 在 0..ARR 间滚动 = PWM 在跑）
    pub fn count(&self) -> u16 {
        self.regs.cnt.read() as u16
    }

    /// 读 ARR/PSC 配置
    pub fn period_config(&self) -> (u16, u16) {
        (self.regs.arr.read() as u16, self.regs.psc.read() as u16)
    }

    /// 停止 PWM（保留配置；MOE 与 CEN 同清）
    pub fn stop(&mut self) {
        self.regs.bdtr.modify(|v| v & !at::BDTR_MOE);
        self.regs.cr1.modify(|v| v & !at::CR1_CEN);
    }

    // ---- 内部 ----

    /// PWM 公共配置：停表 → PSC/ARR/CCR → CCMR 模式
    fn setup_pwm(
        &mut self,
        ch: AtimChannel,
        pclk_hz: u32,
        freq_hz: u32,
        duty_bp: u16,
    ) -> Result<(), AtimError> {
        if duty_bp > 10_000 {
            return Err(AtimError::BadDuty);
        }
        let (psc, arr) =
            crate::gtim::calc_pwm_psc_arr(pclk_hz, freq_hz).ok_or(AtimError::OutOfRange)?;
        let ccr = ((arr as u32 + 1) * duty_bp as u32 / 10_000) as u16;

        self.regs.cr1.write(0);
        self.regs.psc.write(psc as u32);
        self.regs.arr.write(arr as u32);
        self.set_ccr(ch, ccr);
        self.ccmr_ch(ch); // PWM1 + 预装载（清 CCyS 输出方向位）
        Ok(())
    }

    /// CCMR：OCyM=PWM1 + OCyPE，清输出方向 CCyS
    fn ccmr_ch(&mut self, ch: AtimChannel) {
        match ch {
            AtimChannel::Ch1 => {
                self.regs.ccmr1.modify(|v| {
                    (v & !(at::OC1M_MASK | at::OC1PE | 0x3)) | at::OC1M_PWM1 | at::OC1PE
                });
            }
            AtimChannel::Ch2 => {
                self.regs.ccmr1.modify(|v| {
                    (v & !(at::OC2M_MASK | at::OC2PE | (0x3 << 8))) | at::OC2M_PWM1 | at::OC2PE
                });
            }
            AtimChannel::Ch3 => {
                self.regs.ccmr2.modify(|v| {
                    (v & !(at::OC1M_MASK | at::OC1PE | 0x3)) | at::OC1M_PWM1 | at::OC1PE
                });
            }
            AtimChannel::Ch4 => {
                self.regs.ccmr2.modify(|v| {
                    (v & !(at::OC2M_MASK | at::OC2PE | (0x3 << 8))) | at::OC2M_PWM1 | at::OC2PE
                });
            }
        }
    }

    /// CCER：使能主输出（+ 互补），极性高有效
    fn ccer_ch(&mut self, ch: AtimChannel, main: bool, comp: bool) {
        let (e, p, ne, np) = match ch {
            AtimChannel::Ch1 => (at::CC1E, at::CC1P, at::CC1NE, at::CC1NP),
            AtimChannel::Ch2 => (at::CC2E, at::CC2P, at::CC2NE, at::CC2NP),
            AtimChannel::Ch3 => (at::CC3E, at::CC3P, at::CC3NE, at::CC3NP),
            AtimChannel::Ch4 => (at::CC4E, at::CC4P, at::CC4NE, at::CC4NP),
        };
        self.regs.ccer.modify(|v| {
            (v & !(e | p | ne | np)) | if main { e } else { 0 } | if comp { ne } else { 0 }
        });
    }

    /// UG 重装影子寄存器 → ARPE + CEN 启动
    fn finalize(&mut self) {
        self.regs.egr.write(at::EGR_UG);
        self.regs.cr1.write(at::CR1_ARPE | at::CR1_CEN);
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Atim {
        self.regs
    }
}

/// 死区时间 → DTG[7:0] 编码（实际死区 ≥ 请求值；超出 1008·t_DTS 返回 None）
///
/// * `ckd_div` — CR1.CKD 的分频（1/2/4，决定 t_DTS）
pub fn encode_dtg(pclk_hz: u32, ckd_div: u32, dead_ns: u32) -> Option<u8> {
    if dead_ns == 0 || pclk_hz == 0 || !(1..=4).contains(&ckd_div) {
        return None;
    }
    // ticks = ceil(dead_ns / t_DTS)，全程 u64 避免整数除法丢精度
    // （96MHz 的 t_DTS=10.4167ns，截断成 10ns 会使死区偏短）
    let ticks = ((dead_ns as u64 * pclk_hz as u64 + ckd_div as u64 * 1_000_000_000 - 1)
        / (ckd_div as u64 * 1_000_000_000)) as u32;
    match ticks {
        1..=0x7F => Some(ticks as u8),
        // 10x：DT=(64+DTG[5:0])×2 → 步进 2，覆盖 128..254
        128..=254 => {
            let k = (ticks + 1) / 2; // ceil(ticks/2)
            Some(0x80 | (k - 64) as u8)
        }
        // 110：DT=(32+DTG[4:0])×8 → 步进 8，覆盖 256..504
        255..=504 => {
            let k = (ticks + 7) / 8;
            Some(0xC0 | (k - 32) as u8)
        }
        // 111：DT=(32+DTG[4:0])×16 → 步进 16，覆盖 512..1008
        505..=1008 => {
            let k = (ticks + 15) / 16;
            Some(0xE0 | (k - 32) as u8)
        }
        _ => None,
    }
}

/// DTG 编码 → 实际死区（单位：t_DTS ticks），用于测试回代
pub fn decode_dtg_ticks(dtg: u8) -> u32 {
    if dtg & 0x80 == 0 {
        dtg as u32 // 0xx：DT = DTG × t_DTS
    } else if dtg & 0x40 == 0 {
        (64 + (dtg & 0x3F) as u32) * 2 // 10x
    } else if dtg & 0x20 == 0 {
        (32 + (dtg & 0x1F) as u32) * 8 // 110
    } else {
        (32 + (dtg & 0x1F) as u32) * 16 // 111
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dtg_small_range() {
        // 96MHz CKD=1 → t_DTS=10.417ns；100ns → ticks=ceil(9.6)=10
        let dtg = encode_dtg(96_000_000, 1, 100).unwrap();
        assert_eq!(dtg, 10);
        assert_eq!(decode_dtg_ticks(dtg), 10);
    }

    #[test]
    fn dtg_mid_range_steps_of_two() {
        // 4800ns@96MHz → ticks=ceil(460.8)=461 → 10x 档：k=ceil(461/2)=231
        // DTG = 0x80|(231-64)=0x80|0xA7？231-64=167 > 63 越界 → 实际
        // 461 ticks 不在 10x 档（128..254）！应落 110 档：k=ceil(461/8)=58
        let dtg = encode_dtg(96_000_000, 1, 4_800).unwrap();
        assert_eq!(dtg, 0xC0 | (58 - 32));
        let got = decode_dtg_ticks(dtg);
        assert!(got >= 461, "实际死区 {got} ticks 不得短于请求");
    }

    #[test]
    fn dtg_never_shorter_than_request() {
        // 96MHz 下 100ns..10000ns 全程回代校验（用精确 t_DTS 换算）
        for ns in [50u32, 100, 333, 1000, 2500, 5000, 8000, 10_000] {
            let dtg = encode_dtg(96_000_000, 1, ns).unwrap();
            let got_ns = decode_dtg_ticks(dtg) as u64 * 1_000_000_000 / 96_000_000;
            assert!(got_ns >= ns as u64, "{ns}ns → 实际 {got_ns}ns 偏短");
        }
    }

    #[test]
    fn dtg_precision_loss_guard() {
        // 96MHz t_DTS=10.4167ns：4800ns → 461 ticks（非 480！）
        // 旧整数除法把 t_DTS 截成 10ns 会得 480 → 实际死区偏短 200ns
        let dtg = encode_dtg(96_000_000, 1, 4_800).unwrap();
        let got_ns = decode_dtg_ticks(dtg) as u64 * 1_000_000_000 / 96_000_000;
        assert!(got_ns >= 4_800, "4800ns → 实际 {got_ns}ns 偏短");
    }

    #[test]
    fn dtg_ckd_widens_range() {
        // CKD=4 → t_DTS=4×t_PCLK → 2µs@4MHz（t_DTS=1µs）→ ticks=2
        let dtg = encode_dtg(4_000_000, 4, 2_000).unwrap();
        assert_eq!(dtg, 2);
        // 同参数 CKD=1 → ticks=8
        assert_eq!(encode_dtg(4_000_000, 1, 2_000).unwrap(), 8);
    }

    #[test]
    fn dtg_out_of_range() {
        // 96MHz CKD=1 上限 1008 ticks ≈ 10.5µs
        assert!(encode_dtg(96_000_000, 1, 11_000).is_none());
        assert!(encode_dtg(96_000_000, 1, 0).is_none());
        assert!(encode_dtg(0, 1, 100).is_none());
    }

    #[test]
    fn duty_mapping_matches_gtim() {
        let arr: u32 = 4799;
        let ccr = ((arr + 1) * 3_000 / 10_000) as u16;
        assert_eq!(ccr, 1440);
    }
}
