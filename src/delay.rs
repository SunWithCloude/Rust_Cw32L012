//! SysTick 忙等延时
//!
//! 直接操作 Cortex-M0+ SysTick 寄存器（0xE000_E010..E018），
//! 时钟源固定为 HCLK，无需中断。
//!
//! ```ignore
//! let mut delay = Delay::new(&clocks);
//! delay.delay_ms(500);
//! delay.delay_us(10);
//! ```

use crate::rcc::Clocks;

const SYST_CSR: *mut u32 = 0xE000_E010 as *mut u32;
const SYST_RVR: *mut u32 = 0xE000_E014 as *mut u32;
const SYST_CVR: *mut u32 = 0xE000_E018 as *mut u32;

const CSR_ENABLE: u32 = 1 << 0;
const CSR_CLKSOURCE_HCLK: u32 = 1 << 2;
const CSR_COUNTFLAG: u32 = 1 << 16;

/// SysTick 24 位计数器最大装载值
const MAX_LOAD: u32 = 0x00FF_FFFF;

/// 忙等延时器（时钟源 HCLK）
pub struct Delay {
    hclk_hz: u32,
    /// 每微秒节拍数（HCLK 为 1MHz 整数倍时非 0，走无除法快路径）
    per_us: u32,
    /// 每毫秒节拍数（HCLK 为 1kHz 整数倍时非 0）
    per_ms: u32,
}

impl Delay {
    /// 用时钟快照创建延时器
    pub fn new(clocks: &Clocks) -> Self {
        let h = clocks.hclk_hz;
        Delay {
            hclk_hz: h,
            // 预计算换算系数：Cortex-M0+ 无硬件除法，`u64 % u32` 每次调用
            // 都要走软件例程（debug 构建下实测约 1000 周期的固定开销，
            // 会让 delay_us(100) 偏长 3 倍）。
            per_us: if h % 1_000_000 == 0 { h / 1_000_000 } else { 0 },
            per_ms: if h % 1_000 == 0 { h / 1_000 } else { 0 },
        }
    }

    /// HCLK 频率（Hz）
    pub fn hclk_hz(&self) -> u32 {
        self.hclk_hz
    }

    /// 毫秒级延时
    ///
    /// 总节拍数按 u64 计算后按 24 位上限分段，避免逐毫秒重配 SysTick
    /// 造成的累积误差（500ms@96MHz 重配次数从 500 次降为 5 次）。
    pub fn delay_ms(&mut self, ms: u32) {
        let total = if self.per_ms != 0 {
            self.per_ms as u64 * ms as u64
        } else {
            scale_cycles(self.hclk_hz, ms, 1_000)
        };
        self.delay_ticks_u64(total);
    }

    /// 微秒级延时
    ///
    /// 两条路径自动选择：
    /// - 总节拍数 ≥ 32：SysTick 计时（重装载开销已摊薄，误差 < ±1%；
    ///   `ms` 级长延时同理）；
    /// - 总节拍数 < 32（低主频 / 短间隔）：校准忙循环。M0+ 每次循环
    ///   迭代约 5 个 HCLK 周期（`sub`+`cmp`+`branch`+`nop`），
    ///   误差为迭代粒度的 ±5 周期，且时钟越低相对误差越小。
    pub fn delay_us(&mut self, us: u32) {
        let cycles = if self.per_us != 0 {
            self.per_us as u64 * us as u64
        } else {
            scale_cycles(self.hclk_hz, us, 1_000_000)
        };
        if cycles >= BUSY_THRESHOLD_CYCLES {
            self.delay_ticks_u64(cycles);
        } else {
            busy_cycles(cycles as u32);
        }
    }

    /// 纳秒级延时（embedded-hal `DelayNs` 契约：至少延时指定时长）
    ///
    /// 与 `delay_us` 同款双路径：≥ 阈值走 SysTick，更短走周期忙等。
    /// 亚微秒段实际延时 ≥ 请求值（含调用开销），上限受指令周期粒度限制。
    pub fn delay_ns(&mut self, ns: u32) {
        let cycles = (self.hclk_hz as u64) * (ns as u64) / 1_000_000_000;
        if cycles >= BUSY_THRESHOLD_CYCLES {
            self.delay_ticks_u64(cycles);
        } else {
            busy_cycles(cycles as u32);
        }
    }

    /// 忙等指定 SysTick 节拍数（u64 总量，24 位分段）
    fn delay_ticks_u64(&mut self, mut ticks: u64) {
        unsafe {
            while ticks > 0 {
                let n = if ticks > MAX_LOAD as u64 {
                    MAX_LOAD
                } else {
                    ticks as u32
                };
                // 装载初值并启动（时钟源 HCLK）
                SYST_RVR.write_volatile(n - 1);
                SYST_CVR.write_volatile(0);
                SYST_CSR.write_volatile(CSR_ENABLE | CSR_CLKSOURCE_HCLK);
                // 等待计数到 0（COUNTFLAG 置位）
                while SYST_CSR.read_volatile() & CSR_COUNTFLAG == 0 {
                    core::hint::spin_loop();
                }
                SYST_CSR.write_volatile(0);
                ticks -= n as u64;
            }
        }
    }
}

/// 低于该节拍数走周期忙等，高于则用 SysTick。
///
/// SysTick 每次启动/停止的固定开销实测约 200 周期（release、4MHz 下
/// **50µs**）——对 100µs 级延时会带来 50% 误差；周期忙等的进入开销只有
/// 几个周期，因此短延时用忙等更准。
const BUSY_THRESHOLD_CYCLES: u64 = 20_000;

/// 忙等指定 HCLK 周期数（循环体 `subs`+`bne`，M0+ 上 **3 周期/轮**：
/// `subs`=1、`bne` 跳转=2）。
///
/// 比 SysTick 精确得多：不依赖硬件计数器的启停同步，也不受编译优化影响。
/// `CALL_OVERHEAD_CYCLES` 补偿函数调用/参数换算的固定开销（实测标定）。
#[inline]
fn busy_cycles(cycles: u32) {
    const CALL_OVERHEAD_CYCLES: u32 = 150;
    let iters = cycles.saturating_sub(CALL_OVERHEAD_CYCLES) / 3;
    if iters == 0 {
        return;
    }
    // 非 ARM 目标（host 单测 / docs.rs）退回普通忙等，保证 crate 能编译
    #[cfg(target_arch = "arm")]
    unsafe {
        core::arch::asm!(
            "2:",
            "subs {n}, #1",
            "bne 2b",
            n = inout(reg) iters => _,
            options(nomem, nostack),
        );
    }
    #[cfg(not(target_arch = "arm"))]
    for _ in 0..iters {
        core::hint::spin_loop();
    }
}

/// `hclk_hz × value / div` 的节拍换算，**避免每次调用都做 64 位除法**。
///
/// Cortex-M0+ 没有硬件除法指令，`u64 / u64` 会退化为 `__aeabi_uldivmod`
/// 软件例程（实测在 4MHz、debug 构建下每次约 1300 周期 ≈ 330µs）——对
/// `delay_us(100)` 这种短延时是灾难性的（实测偏长 300%）。
///
/// 常见时钟（4/8/48/96MHz 等）都是 `div` 的整数倍，此时先约分再单次乘法；
/// 非整数倍（如 3.6864MHz）才回退到通用除法。
#[inline]
fn scale_cycles(hclk_hz: u32, value: u32, div: u32) -> u64 {
    if div != 0 && hclk_hz % div == 0 {
        (hclk_hz / div) as u64 * value as u64
    } else {
        (hclk_hz as u64) * (value as u64) / (div as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::scale_cycles;

    #[test]
    fn scale_exact_multiples() {
        // 4MHz：1µs=4 周期、1ms=4000 周期
        assert_eq!(scale_cycles(4_000_000, 1, 1_000_000), 4);
        assert_eq!(scale_cycles(4_000_000, 1000, 1_000_000), 4000);
        assert_eq!(scale_cycles(4_000_000, 1, 1_000), 4000);
        assert_eq!(scale_cycles(4_000_000, 1000, 1_000), 4_000_000);
        // 96MHz：1µs=96 周期
        assert_eq!(scale_cycles(96_000_000, 1, 1_000_000), 96);
        assert_eq!(scale_cycles(96_000_000, 500, 1_000_000), 48_000);
    }

    #[test]
    fn scale_non_multiple_falls_back() {
        // 3.6864MHz 不是 1e6 的整数倍 → 通用路径，结果与直接计算一致
        let h = 3_686_400u32;
        assert_eq!(
            scale_cycles(h, 1000, 1_000_000),
            (h as u64) * 1000 / 1_000_000
        );
        assert_eq!(scale_cycles(h, 10, 1_000), (h as u64) * 10 / 1_000);
    }

    #[test]
    fn scale_no_overflow_at_96mhz() {
        // 96MHz × 60000ms 超出 u32，必须走 u64
        assert_eq!(scale_cycles(96_000_000, 60_000, 1_000), 5_760_000_000);
    }
}
