//! BTIM 基础定时器（§14）
//!
//! 16-bit 自动重装载计数器 + 16-bit 预分频，内部计数模式时钟为 PCLK
//! 直接驱动（手册图 14-5，无 STM32 式 ×2 规则）。支持周期中断与
//! 单次触发，配合 [`runtime::interrupt!`](crate::runtime) 宏编写 ISR。
//!
//! # 用法（BTIM1 @2Hz 周期中断翻转 PC13）
//!
//! ```ignore
//! let mut tim = Btim::new(p.btim1, &mut rcc);
//! tim.start_periodic(pclk_hz, 2)?;      // 2Hz 溢出
//! tim.enable_update_irq();
//! unsafe { runtime::Interrupt::Btim1.enable(); }
//!
//! interrupt!("BTIM1_irq", BTIM1_HANDLER, {
//!     // 注意：在 ISR 中重新取得寄存器块（Peripherals::steal 或静态引用）
//!     BTIM1.clear_update_flag();
//!     led.toggle();
//! });
//! ```
//!
//! # 定时公式
//!
//! `溢出频率 = PCLK / ((PSC+1) × (ARR+1))`，PSC/ARR 均 16-bit。
//! `start_periodic` 自动选择使 ARR 余量最大的预分频组合。

use crate::pac::{self, btim as bt};
use crate::rcc::Rcc;

/// 定时频率不可实现（ARR/ PSC 均无法覆盖）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodError;

/// BTIM 基础定时器
pub struct Btim<'p> {
    regs: &'p pac::Btim,
    /// 当前 CK_CNT（Hz），`start_*` 后有效
    counter_hz: u32,
}

impl<'p> Btim<'p> {
    /// 获取定时器实例并使能 BTIM 时钟（APBEN2[2]，三实例共用）
    pub fn new(regs: &'p pac::Btim, rcc: &mut Rcc) -> Self {
        rcc.enable_btim();
        Btim {
            regs,
            counter_hz: 0,
        }
    }

    /// 启动周期溢出（溢出频率 `freq_hz`，更新中断可用）
    ///
    /// # Panics
    /// `freq_hz == 0` 时 panic。
    pub fn start_periodic(&mut self, pclk_hz: u32, freq_hz: u32) -> Result<(), PeriodError> {
        assert!(freq_hz > 0, "溢出频率必须大于 0");
        let (psc, arr, ck_cnt) = calc_period(pclk_hz, freq_hz).ok_or(PeriodError)?;
        self.regs.cr1.write(0); // 停止
        self.regs.psc.write(psc as u32);
        self.regs.arr.write(arr as u32);
        self.regs.icr.write(bt::ICR_CLEAR_UIF); // 清残留更新标志（R1W0）
        self.regs.egr.write(bt::EGR_UG); // 重装 PSC/ARR、清零计数器
        self.counter_hz = ck_cnt / (arr as u32 + 1);
        self.regs.dier.modify(|v| v | bt::DIER_UIE);
        self.regs.cr1.write(bt::CR1_EN);
        Ok(())
    }

    /// 启动单次模式：溢出一次后计数器自动停止（CR1.ONESHOT）
    pub fn start_one_shot(&mut self, pclk_hz: u32, freq_hz: u32) -> Result<(), PeriodError> {
        self.start_periodic(pclk_hz, freq_hz)?;
        self.regs.cr1.write(bt::CR1_EN | bt::CR1_ONESHOT);
        Ok(())
    }

    /// 停止计数
    pub fn stop(&mut self) {
        self.regs.cr1.modify(|v| v & !bt::CR1_EN);
    }

    /// 恢复计数（保留已配置的 PSC/ARR）
    pub fn start(&mut self) {
        self.regs.cr1.modify(|v| v | bt::CR1_EN);
    }

    /// 当前计数值（0..ARR）
    #[inline]
    pub fn count(&self) -> u16 {
        self.regs.cnt.read() as u16
    }

    /// 计数器时钟 CK_CNT（Hz）
    #[inline]
    pub fn counter_hz(&self) -> u32 {
        self.counter_hz
    }

    /// 更新中断标志是否挂起
    #[inline]
    pub fn update_pending(&self) -> bool {
        self.regs.isr.read() & bt::ISR_UIF != 0
    }

    /// 清除更新中断标志（ISR 内调用）
    #[inline]
    pub fn clear_update_flag(&mut self) {
        self.regs.icr.write(bt::ICR_CLEAR_UIF);
    }

    /// 使能/禁用更新中断（NVIC 层还需 [`runtime::Interrupt::BtimX`](crate::runtime::Interrupt)）
    pub fn enable_update_irq(&mut self, on: bool) {
        if on {
            self.regs.dier.modify(|v| v | bt::DIER_UIE);
        } else {
            self.regs.dier.modify(|v| v & !bt::DIER_UIE);
        }
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Btim {
        self.regs
    }
}

/// 计算周期配置：返回 (PSC, ARR, CK_CNT)。
///
/// 策略：选最小 PSC 使 ARR ≤ 0xFFFF（ARR 越大相对量化误差越小）。
pub fn calc_period(pclk_hz: u32, freq_hz: u32) -> Option<(u16, u16, u32)> {
    if freq_hz == 0 || pclk_hz == 0 {
        return None;
    }
    // psc 使 ck_cnt ≤ 65536×freq：psc = ceil(pclk/(65536×f)) - 1
    let mut psc = ((pclk_hz as u64 + 65_535 * freq_hz as u64 - 1) / (65_536 * freq_hz as u64))
        .saturating_sub(1);
    while psc <= 0xFFFF {
        let ck_cnt = pclk_hz / (psc as u32 + 1);
        // arr = round(ck_cnt/f) - 1
        let ticks = (ck_cnt as u64 + freq_hz as u64 / 2) / freq_hz as u64;
        if (2..=65_536).contains(&ticks) {
            return Some((psc as u16, (ticks - 1) as u16, ck_cnt));
        }
        psc += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_2hz_at_4mhz() {
        let (psc, arr, ck) = calc_period(4_000_000, 2).unwrap();
        assert_eq!(psc, 30); // ceil(4M/131072)-1
        assert_eq!(ck, 4_000_000 / 31);
        let f = ck as f64 / (arr as f64 + 1.0);
        assert!((f - 2.0).abs() < 0.001, "实际 {f}Hz");
    }

    #[test]
    fn periodic_1khz_at_48mhz() {
        let (psc, arr, ck) = calc_period(48_000_000, 1_000).unwrap();
        let f = ck as f64 / (arr as f64 + 1.0);
        assert!((f - 1000.0).abs() < 1.0, "实际 {f}Hz");
        assert!(psc < 8, "1kHz@48MHz 预分频应很小");
    }

    #[test]
    fn periodic_exact_division() {
        // 整除场景：4MHz / (3999+1) = 1kHz，psc=0 arr=3999
        let (psc, arr, _) = calc_period(4_000_000, 1_000).unwrap();
        assert_eq!(psc, 0);
        assert_eq!(arr, 3999);
    }

    #[test]
    fn periodic_extremes() {
        // 超慢：0.5Hz@96MHz 需要 psc≈2929（16bit 内）
        assert!(calc_period(96_000_000, 1).is_some());
        assert!(calc_period(96_000_000, 20).is_some());
        // 超快：f=PCLK/2（psc=0, arr=1）是上限
        assert!(calc_period(4_000_000, 2_000_000).is_some());
        assert!(calc_period(4_000_000, 3_000_000).is_none());
        assert!(calc_period(4_000_000, 0).is_none());
    }
}
