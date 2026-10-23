//! IWDT 独立看门狗（§19）
//!
//! 计数时钟来自独立 RC10K（≈10kHz，与主时钟域完全独立），主时钟
//! 崩溃/跑飞时仍能复位芯片。键值机：`0xCCCC` 启动 → `0xAAAA` 喂狗 →
//! `0x5555` 解锁改配置 → `0x5A5A`+`0xA5A5` 停止（仅调试期使用）。
//!
//! # 用法
//!
//! ```ignore
//! let mut wdt = Iwdt::new(p.iwdt);
//! wdt.start(TimeoutMs(1500))?;   // ≈1.5s 超时（RC10K ±%级漂移）
//! loop {
//!     do_work();
//!     wdt.feed();                 // 忘了喂 → 芯片复位
//! }
//! ```
//!
//! # 超时公式
//!
//! `T = (PRS 分频) × (ARR+1) / RC10K`，PRS ∈ {4,8,…,512}，ARR ≤ 0xFFF。
//! 范围：0.4ms（÷4, ARR=3）～ 209ms×… 至 `512×4096/10k ≈ 209.7s`。
//!
//! # 实机验证要点
//!
//! 停止喂狗后芯片会在超时点复位——PC13 闪烁计数清零、探针会看到
//! SWD 短暂掉线再恢复，即为看门狗复位生效的硬证据。

use crate::pac::{self, iwdt as wd};
use crate::rcc::Rcc;

/// 看门狗超时（毫秒）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutMs(pub u32);

/// 看门狗配置错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutError;

/// 计算能覆盖 `TimeoutMs` 的最小分频组合（PR 分频、ARR）
///
/// 返回 `(prs_code, arr)`；不可实现（超时 < 0.4ms 或 > 209.7s）时 Err。
pub fn calc_timeout(t: TimeoutMs) -> Result<(u32, u16), TimeoutError> {
    let ms = t.0 as u64;
    if ms == 0 {
        return Err(TimeoutError);
    }
    // 每档分频下的最小超时（ARR=1）与最大超时（ARR=0xFFF）
    for (code, div) in [4u64, 8, 16, 32, 64, 128, 256, 512].iter().enumerate() {
        // ticks = ceil(ms × RC10K / 1000 / div)（整体向上取整，保证不短于请求值）
        let ticks = (ms * wd::RC10K_HZ as u64 + 1000 * div - 1) / (1000 * div);
        if ticks >= 2 && ticks <= wd::ARR_MAX as u64 {
            return Ok((code as u32, (ticks - 1) as u16));
        }
    }
    Err(TimeoutError)
}

/// IWDT 独立看门狗
pub struct Iwdt<'p> {
    regs: &'p pac::Iwdt,
}

impl<'p> Iwdt<'p> {
    /// 获取 IWDT（配置/计数时钟为独立 RC10K，APBEN2[4] 仅门控寄存器访问）
    pub fn new(regs: &'p pac::Iwdt, rcc: &mut Rcc) -> Self {
        rcc.enable_iwdt();
        Iwdt { regs }
    }

    /// 启动看门狗（溢出动作 = 复位），超时 ≈ `t`（RC10K 有 ±% 级漂移）
    ///
    /// 已启动时再次调用会先解锁重配（不停止计数）。
    pub fn start(&mut self, t: TimeoutMs) -> Result<(), TimeoutError> {
        let (prs, arr) = calc_timeout(t)?;
        self.regs.kr.write(wd::KR_UNLOCK); // 解锁 CR/ARR/WINR
        self.regs
            .cr
            .write(prs & wd::CR_PRS_MASK | wd::CR_ACTION_RESET);
        self.regs.arr.write(arr as u32);
        self.regs.kr.write(wd::KR_FEED); // 装载新 ARR 并从零计起
        self.regs.kr.write(wd::KR_START); // 启动（未启动时）/ 重新上锁
        Ok(())
    }

    /// 启动看门狗于"溢出中断"模式（不复位）——低功耗唤醒源
    ///
    /// ACTION=1（溢出走中断而非复位）+ IE=1 + DeepSleep 不暂停
    /// （CR.PAUSE=0，RC10K 域在 DeepSleep 下继续计数）。
    /// 溢出中断号 `WDT_irq`（IRQ 0，见手册表 3-3）；ISR 常规做法是
    /// 喂狗 + 记录事件。NVIC 使能需 `runtime::Interrupt::Wdt`。
    pub fn start_interrupt(&mut self, t: TimeoutMs) -> Result<(), TimeoutError> {
        let (prs, arr) = calc_timeout(t)?;
        self.regs.kr.write(wd::KR_UNLOCK);
        self.regs
            .cr
            .write((prs & wd::CR_PRS_MASK) | wd::CR_IE | wd::CR_ACTION_IRQ); // 溢出=中断不复位
        self.regs.arr.write(arr as u32);
        self.regs.kr.write(wd::KR_FEED);
        self.regs.kr.write(wd::KR_START);
        Ok(())
    }

    /// 喂狗（重载计数值）
    #[inline]
    pub fn feed(&mut self) {
        self.regs.kr.write(wd::KR_FEED);
    }

    /// 当前计数值（0..ARR，向下计数语义以手册 §19.6.4 为准）
    #[inline]
    pub fn count(&self) -> u16 {
        (self.regs.cnt.read() & 0xFFF) as u16
    }

    /// 停止看门狗（仅调试；产线代码不应使用——0x5A5A+0xA5A5 双键）
    pub fn stop(&mut self) {
        self.regs.kr.write(wd::KR_STOP1);
        self.regs.kr.write(wd::KR_STOP2);
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Iwdt {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_short() {
        // 10ms：ticks = ceil(10×10/4) = 25 → PRS=0(÷4) ARR=24
        let (prs, arr) = calc_timeout(TimeoutMs(10)).unwrap();
        assert_eq!(prs, 0);
        assert_eq!(arr, 24);
    }

    #[test]
    fn timeout_mid_uses_smallest_divisor() {
        // 500ms：÷4 → ticks=1250 ≤ 0xFFF ✓（无需升档）
        let (prs, arr) = calc_timeout(TimeoutMs(500)).unwrap();
        assert_eq!(prs, 0);
        assert_eq!(arr, 1249);
    }

    #[test]
    fn timeout_large_needs_max_divisor() {
        // 200s：÷512 → ticks=ceil(200000×10/512)=3907 → ARR=3906
        let (prs, arr) = calc_timeout(TimeoutMs(200_000)).unwrap();
        assert_eq!(prs, 7);
        assert_eq!(arr, 3906);
    }

    #[test]
    fn timeout_out_of_range() {
        // 上限：÷512、ARR=0xFFF → 4095×512/10 ≈ 209.6s
        assert!(calc_timeout(TimeoutMs(210_000)).is_err());
        assert!(calc_timeout(TimeoutMs(0)).is_err());
    }

    #[test]
    fn timeout_rounding_never_shorter_than_requested() {
        // 计算出的实际超时必须 ≥ 请求值（向上取整保证）
        for ms in [1u32, 3, 7, 50, 123, 999, 5000, 60_000] {
            if let Ok((prs, arr)) = calc_timeout(TimeoutMs(ms)) {
                let div = 4u64 << prs;
                let actual_us = (arr as u64 + 1) * div * 100;
                assert!(
                    actual_us >= ms as u64 * 1000,
                    "{ms}ms: 实际 {actual_us}µs 偏短"
                );
            }
        }
    }
}
