//! `critical-section` 实现（feature `critical-section-impl`）
//!
//! 生态里大量 crate（`embedded-hal-bus`、总线共享封装、各类传感器 / 存储驱动、
//! 部分实时框架）都要求上层提供一个 [`critical_section`] 实现，否则链接期报
//! `no critical section implementation`。Cortex-M0+ 是单核、无中断嵌套屏蔽寄存器
//! 之外的机制的，标准做法就是用 `PRIMASK` 关全局中断。
//!
//! # 用法
//!
//! ```toml
//! [dependencies]
//! cw32l012-hal = { version = "0.9", features = ["critical-section-impl"] }
//! ```
//!
//! **一个固件只能有一个实现**。如果同时依赖 `cortex-m` 并开启了它的
//! `critical-section-single-core`，链接会报重复符号——两者选一个即可。
//!
//! # 实现细节
//!
//! [`acquire`](critical_section::Impl::acquire) 先读回 `PRIMASK` 再 `cpsid i`，
//! 因此**嵌套调用是安全的**：内层拿到 `1`，`release` 时不会误开中断。
//! 非 ARM 目标（host 单测、docs.rs）退化为空实现，保证 crate 在 x86 上也能编译。

#[cfg(target_arch = "arm")]
use core::arch::asm;

/// 单核 Cortex-M0+ 的全局中断屏蔽实现
struct SingleCoreCriticalSection;

critical_section::set_impl!(SingleCoreCriticalSection);

unsafe impl critical_section::Impl for SingleCoreCriticalSection {
    #[inline]
    unsafe fn acquire() -> critical_section::RawRestoreState {
        let primask = read_primask();
        disable_interrupts();
        primask
    }

    #[inline]
    unsafe fn release(restore_state: critical_section::RawRestoreState) {
        // 只有进入临界区之前中断是开着的（PRIMASK == 0），才需要重新开
        if restore_state == 0 {
            enable_interrupts();
        }
    }
}

/// 读 `PRIMASK`（0 = 中断开启，1 = 已屏蔽）
#[inline]
fn read_primask() -> u32 {
    #[cfg(target_arch = "arm")]
    unsafe {
        let v: u32;
        asm!("mrs {}, PRIMASK", out(reg) v, options(nomem, nostack, preserves_flags));
        v
    }
    #[cfg(not(target_arch = "arm"))]
    {
        0
    }
}

/// `CPSID i`：屏蔽可屏蔽中断
#[inline]
fn disable_interrupts() {
    #[cfg(target_arch = "arm")]
    unsafe {
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }
}

/// `CPSIE i`：恢复可屏蔽中断
#[inline]
fn enable_interrupts() {
    #[cfg(target_arch = "arm")]
    unsafe {
        asm!("cpsie i", options(nomem, nostack, preserves_flags));
    }
}

#[cfg(test)]
mod tests {
    /// 临界区在 host 上必须能正常进出（嵌套也不得死锁）
    #[test]
    fn critical_section_roundtrip() {
        let a = critical_section::with(|_| 1u32);
        let b = critical_section::with(|_| critical_section::with(|_| 2u32));
        assert_eq!(a + b, 3);
    }
}
