//! WWDT 窗口看门狗（§20）——窗口内喂狗，过早/过晚都复位
//!
//! 7-bit 递减计数器（PCLK/4096/2^PRS 驱动），三条件触发复位：
//! 1. 计数值递减到 0x3F（过晚喂狗）；
//! 2. 当前计数值 > 窗口值 WINR 时执行喂狗（过早喂狗）；
//! 3. 向 WCNT 写入 ≤ 0x3F 的值。
//!
//! 喂狗时间窗（手册 §20.3.4）：
//!
//! ```text
//! T_min = T_PCLK × 4096 × 2^PRS × (WCNT - WINR)
//! T_max = T_PCLK × 4096 × 2^PRS × (WCNT - 0x3F)
//! ```
//!
//! 上电默认关闭；[`Wwdt::start`] 一次配好（PRS/初值/窗口），**开启后不可关闭**
//! （仅系统复位可停）。[`Wwdt::feed`] 会先读当前计数值、落在窗口内才写——
//! 过早时返回 [`FeedError::TooEarly`] 而不是让芯片复位。
//!
//! # 用法（每 30ms 轮询主循环喂狗，窗口 15..=30ms @96MHz）
//!
//! ```ignore
//! let mut wwdt = Wwdt::new(p.wwdt, &mut rcc);
//! let cfg = WindowConfig::for_feed_interval(pclk_hz, 30).unwrap();
//! wwdt.start(cfg);
//! // 主循环：
//! wwdt.feed()?; // 过早喂狗会得到 Err 而不是复位
//! ```

use crate::pac::{self, wwdt as ww};
use crate::rcc::Rcc;

/// 窗口配置（由 [`WindowConfig::for_feed_interval`] 生成或手工构造）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowConfig {
    /// 预分频 PRS[0..7]：计数时钟 = PCLK / 4096 / 2^PRS
    pub prs: u8,
    /// 计数器初值 WCNT（0x40..=0x7F；≤0x3F 立即复位）
    pub wcnt: u8,
    /// 窗口值 WINR（0x40..=wcnt-1；喂狗下限）
    pub winr: u8,
}

/// 喂狗错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedError {
    /// 当前计数值仍在窗口值之上（喂得太早，真写会触发复位）
    TooEarly,
}

impl WindowConfig {
    /// 按喂狗周期自动求解 (PRS, WCNT, WINR)
    ///
    /// 目标：`T_max ≈ interval`（ceil 保证不短于请求）、`T_min ≈ interval/2`
    /// （软件每 interval 喂一次，落在窗口正中，两侧各有 ~25% 裕量）。
    /// 超出可实现范围（96MHz 下最长约 349ms、4MHz 下最长约 8.4s）返回 None。
    pub fn for_feed_interval(pclk_hz: u32, interval_ms: u32) -> Option<WindowConfig> {
        if pclk_hz == 0 || interval_ms == 0 {
            return None;
        }
        let interval_ns = interval_ms as u64 * 1_000_000;
        // 选最小 PRS 使 64 tick ≥ interval（64 = 0x7F-0x3F，最大计数跨度）
        let mut prs = None;
        for p in 0u8..8 {
            let tick = tick_ns(pclk_hz, p);
            if tick != 0 && tick * 64 >= interval_ns {
                prs = Some(p);
                break;
            }
        }
        let prs = prs?;
        let tick = tick_ns(pclk_hz, prs);
        // WCNT：ceil(interval / tick) 个 tick 后到 0x3F → T_max ≥ interval；
        // 至少 2 tick（保证 WINR < WCNT 有解）
        let mut wc_ticks = ((interval_ns + tick - 1) / tick) as u32;
        if wc_ticks < 2 {
            wc_ticks = 2;
        }
        let wcnt = 0x3F + wc_ticks; // ≤ 0x7F（前面已保证）
        if wcnt > 0x7F {
            return None;
        }
        // WINR：窗口 ≈ interval/2（但至少留 1 tick、至多留 wc_ticks-1 tick）
        let mut win_ticks = wc_ticks / 2;
        if win_ticks == 0 {
            win_ticks = 1;
        }
        if win_ticks >= wc_ticks {
            win_ticks = wc_ticks - 1;
        }
        let winr = (wcnt - win_ticks) as u8;
        Some(WindowConfig {
            prs,
            wcnt: wcnt as u8,
            winr,
        })
    }
}

/// 单个计数值的时长（ns）= 4096 × 2^PRS / PCLK（u64 全程，防整数除法丢精度）
fn tick_ns(pclk_hz: u32, prs: u8) -> u64 {
    (4096u64 << prs) * 1_000_000_000 / pclk_hz as u64
}

/// WWDT 窗口看门狗
pub struct Wwdt<'p> {
    regs: &'p pac::Wwdt,
    initial: u8,
    winr: u8,
}

impl<'p> Wwdt<'p> {
    /// 获取 WWDT 实例并使能配置/工作时钟（APBEN2.WWDT，bit5）
    pub fn new(regs: &'p pac::Wwdt, rcc: &mut Rcc) -> Self {
        rcc.enable_wwdt();
        Wwdt {
            regs,
            initial: 0x7F,
            winr: 0x7F,
        }
    }

    /// 启动看门狗（开启后不可关闭，仅复位可停）
    ///
    /// 顺序：CR1(PRS/WINR) → CR0(WCNT 初值 + EN)。
    /// DeepSleep 期间停止计数，唤醒后恢复。
    pub fn start(&mut self, cfg: WindowConfig) {
        self.initial = cfg.wcnt;
        self.winr = cfg.winr;
        let cr1 =
            ((cfg.prs as u32 & 0x7) << ww::CR1_PRS_POS) | (cfg.winr as u32 & ww::CR1_WINR_MASK);
        self.regs.cr1.write(cr1);
        // 先设初值（EN=0 时安全），再置 EN（单次写同时带初值）
        self.regs.cr0.write(cfg.wcnt as u32 & ww::CR0_WCNT_MASK);
        self.regs
            .cr0
            .write(cfg.wcnt as u32 & ww::CR0_WCNT_MASK | ww::CR0_EN);
    }

    /// 喂狗（窗口安全版）
    ///
    /// 先读当前计数值：落在 `[WINR, 0x3F+1)` 才刷新为初值；
    /// 过早喂狗返回 [`FeedError::TooEarly`]（真写会复位）。
    pub fn feed(&mut self) -> Result<(), FeedError> {
        let current = (self.regs.cr0.read() & ww::CR0_WCNT_MASK) as u8;
        if current > self.winr {
            return Err(FeedError::TooEarly);
        }
        self.regs.cr0.write(self.initial as u32);
        Ok(())
    }

    /// 直接刷新计数器（绕过窗口检查——仅在你明确知道时机合法时使用）
    pub fn feed_forced(&mut self) {
        self.regs.cr0.write(self.initial as u32);
    }

    /// 当前计数值（0x3F..=0x7F）
    pub fn count(&self) -> u8 {
        (self.regs.cr0.read() & ww::CR0_WCNT_MASK) as u8
    }

    /// 当前窗口值 WINR
    pub fn winr(&self) -> u8 {
        (self.regs.cr1.read() & ww::CR1_WINR_MASK) as u8
    }

    /// 喂狗时间窗 `(T_min_us, T_max_us)`（按当前 PCLK 计算）
    pub fn window_us(&self, pclk_hz: u32) -> (u64, u64) {
        let prs = ((self.regs.cr1.read() & ww::CR1_PRS_MASK) >> ww::CR1_PRS_POS) as u8;
        let tick = tick_ns(pclk_hz, prs);
        let wcnt = self.initial;
        // ns → µs（四舍五入）
        let to_us = |ns: u64| (ns + 500) / 1000;
        (
            to_us((wcnt - self.winr) as u64 * tick),
            to_us((wcnt - 0x3F) as u64 * tick),
        )
    }

    /// 预溢出标志（计到 0x40 时置位；写 0 清除）
    pub fn pre_overflow(&self) -> bool {
        self.regs.sr.read() & ww::SR_POV != 0
    }

    /// 清除预溢出标志
    pub fn clear_pre_overflow(&mut self) {
        self.regs.sr.write(0);
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Wwdt {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_50ms_at_96mhz() {
        // tick(prs)=4096×2^prs×1e6/96e6；prs=5 → 1365.3us；64×1365=87ms ≥ 50ms ✓
        let cfg = WindowConfig::for_feed_interval(96_000_000, 50).unwrap();
        assert_eq!(cfg.prs, 5);
        assert_eq!(cfg.wcnt, 0x64); // 0x3F + ceil(50000/1365)=37
        assert_eq!(cfg.winr, 0x52); // 0x64 - 18
    }

    #[test]
    fn interval_1s_at_4mhz() {
        // 4MHz：tick(prs=4)=16384000ns；64×16.384ms=1.049s ≥ 1s ✓（prs=3 仅 0.52s）
        let cfg = WindowConfig::for_feed_interval(4_000_000, 1000).unwrap();
        assert_eq!(cfg.prs, 4);
        assert_eq!(cfg.wcnt, 0x7D); // 0x3F + ceil(1e9/16.384e6)=62
        assert_eq!(cfg.winr, 0x5E); // 0x7D - 31
    }

    #[test]
    fn tmax_never_shorter_than_interval() {
        // ceil 保证：T_max = (wcnt-0x3F)×tick ≥ interval
        for &(pclk, ms) in &[
            (96_000_000u32, 1u32),
            (96_000_000, 30),
            (96_000_000, 300),
            (4_000_000, 100),
            (24_000_000, 200),
        ] {
            let cfg = WindowConfig::for_feed_interval(pclk, ms).unwrap();
            let tick = tick_ns(pclk, cfg.prs);
            let t_max = (cfg.wcnt - 0x3F) as u64 * tick;
            let t_min = (cfg.wcnt - cfg.winr) as u64 * tick;
            assert!(
                t_max >= ms as u64 * 1_000_000,
                "{ms}ms@{pclk}: T_max={t_max}ns 偏短"
            );
            assert!(
                t_min * 2 <= t_max,
                "{ms}ms@{pclk}: 窗口过窄 T_min={t_min}ns"
            );
            assert!(cfg.winr >= 0x40, "WINR 越界：0x{:02X}", cfg.winr);
            assert!(cfg.wcnt > cfg.winr, "WCNT 必须 > WINR");
        }
    }

    #[test]
    fn out_of_range() {
        // 96MHz 最长 64×5461us ≈ 349ms
        assert!(WindowConfig::for_feed_interval(96_000_000, 400).is_none());
        assert!(WindowConfig::for_feed_interval(96_000_000, 0).is_none());
        assert!(WindowConfig::for_feed_interval(0, 100).is_none());
    }

    #[test]
    fn window_matches_manual_example() {
        // 手册 §20.3.4 例：24MHz、PRS=1、WCNT=0x6F、WINR=0x4F →
        // T_min≈10.922ms、T_max=16.384ms
        let tick = tick_ns(24_000_000, 1); // 341333ns（精确值 341333.3）
        let t_min = (0x6F - 0x4F) as u64 * tick;
        let t_max = (0x6F - 0x3F) as u64 * tick;
        assert!(
            t_min >= 10_900_000 && t_min <= 10_940_000,
            "T_min={t_min}ns 应≈10922us"
        );
        assert!(
            t_max >= 16_380_000 && t_max <= 16_384_000,
            "T_max={t_max}ns 应≈16384us"
        );
    }

    #[test]
    fn feed_semantics_unit_check() {
        // feed() 的判定逻辑（纯数值复现）：current > winr → TooEarly
        let winr = 0x52u8;
        assert!((0x60u8) > winr); // 过早
        assert!((0x52u8) <= winr); // 窗口开启
        assert!((0x40u8) <= winr); // 仍可喂（>0x3F）
        assert!((0x3Fu8) <= winr); // 但 0x3F 已触发复位
    }
}
