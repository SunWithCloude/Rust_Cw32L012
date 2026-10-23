//! 电源控制（§3）——休眠 / 深度休眠
//!
//! CW32L012 共三种工作模式（表 3-1）：
//!
//! | 模式 | CPU | 高速时钟（HSE/HSIOSC） | 低速时钟（LSE/LSI/RC10K） |
//! |------|-----|------------------------|---------------------------|
//! | 运行 Active | 运行 | ON/OFF | ON/OFF |
//! | 休眠 Sleep | 停止 | 保持 | 保持 |
//! | 深度休眠 DeepSleep | 停止 | **自动关闭** | 保持 |
//!
//! 进入方式 = `WFI` + SCB->SCR.SLEEPDEEP（0=休眠、1=深度休眠）。
//!
//! # 深度休眠注意事项（手册 §3.3.1 注）
//!
//! - 进入前 FLASH 必须空闲（`FLASH_ISR.BUSY=0`）且 `FLASH_CR1.MODE=0`，
//!   否则 [`deep_sleep`] 返回 [`PwrError::FlashBusy`]；
//! - DeepSleep 会自动关闭高速时钟——需在休眠期间工作的外设
//!   必须先切换到低速时钟（LSE/LSI/RC10K）；
//! - `fast_wakeup=true` 时置 `SYSCTRL_CR2.WAKEUPCLK`：唤醒后系统时钟自动
//!   切为 HSI 4MHz（加速唤醒）；**唤醒后需用 `rcc` 重新配置系统时钟**。
//!
//! # 可唤醒 DeepSleep 的中断源（表 3-3）
//!
//! IWDT、LVD、RTC、RCC、GPIOA-F、LPTIM、VC1-4、UART1-3、FAULT。
//! （WWDT/FLASH/RAM/ADC/定时器/SPI/I2C 只能唤醒休眠，不能唤醒深度休眠。）
//!
//! # 用法
//!
//! ```ignore
//! use cw32l012_hal::pwr;
//!
//! // 休眠：外设全速运行，CPU 停，任意中断唤醒
//! pwr::sleep();
//!
//! // 深度休眠：高速时钟关闭，唤醒后快速切 HSI 4MHz
//! pwr::deep_sleep(true)?;
//! rcc.use_hsi_48mhz()?; // 唤醒后恢复业务时钟
//! ```

use crate::pac;
use core::convert::Infallible;

/// SCB->SCR（System Control Register，Cortex-M0+ §B3.2）
const SCB_SCR: *mut u32 = 0xE000_ED10 as *mut u32;
/// SLEEPDEEP 位（SCR[2]）
const SCR_SLEEPDEEP: u32 = 1 << 2;

/// 低功耗错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwrError {
    /// FLASH 忙（擦写进行中）或非读模式——手册 §3.3.1 禁止此时进入 DeepSleep
    FlashBusy,
}

/// 休眠（Sleep mode）
///
/// CPU 停止，全部外设（含 SysTick/NVIC）继续运行，时钟不变；
/// 任意已使能中断唤醒。若调用时已有中断挂起，WFI 直接返回。
///
/// # Panics（非 ARM 目标）
/// 仅在 `thumbv6m-none-eabi` 等 ARM 目标上可用。
pub fn sleep() {
    unsafe {
        // SLEEPDEEP=0（保险起见显式清位——深度休眠函数会复位它）
        core::ptr::write_volatile(SCB_SCR, core::ptr::read_volatile(SCB_SCR) & !SCR_SLEEPDEEP);
        arm_wfi();
    }
}

/// 深度休眠（DeepSleep mode）
///
/// 高速时钟自动关闭、CPU 停止；低速时钟保持，可被 IWDT/RTC/GPIO/LPTIM/
/// UART 等唤醒。唤醒后：
/// - `fast_wakeup=true`：系统时钟已自动切 HSI 4MHz（原时钟源保持使能）；
/// - `fast_wakeup=false`：继续用进入前的时钟源。
///
/// 返回后 SLEEPDEEP 已清零（后续 [`sleep`] 不会误入 DeepSleep）。
pub fn deep_sleep(fast_wakeup_hsi4m: bool) -> Result<(), PwrError> {
    prepare_deep_sleep(fast_wakeup_hsi4m)?;
    enter_deep_sleep();
    Ok(())
}

/// 深度休眠进入前的寄存器准备（FLASH 检查 + WAKEUPCLK）
fn prepare_deep_sleep(fast_wakeup: bool) -> Result<(), PwrError> {
    let fc = unsafe { &*(pac::FLASHCTRL_BASE as *const pac::Flashctrl) };
    let sc = unsafe { &*(pac::SYSCTRL_BASE as *const pac::Sysctrl) };
    // 手册 §3.3.1 注：MODE 非读模式或擦写 BUSY 时禁止进入 DeepSleep
    if fc.cr1.read() & 0x1C != 0 || fc.isr.read() & (1 << 5) != 0 {
        return Err(PwrError::FlashBusy);
    }
    // WAKEUPCLK（CR2 有 0x5A5A 写保护键）
    if fast_wakeup {
        let v = sc.cr2.read();
        sc.cr2.write(pac::sysctrl::KEY | v | (1 << 3));
    }
    Ok(())
}

/// SLEEPDEEP=1 + WFI + 唤醒后清位
#[cfg(target_arch = "arm")]
fn enter_deep_sleep() {
    unsafe {
        core::ptr::write_volatile(SCB_SCR, core::ptr::read_volatile(SCB_SCR) | SCR_SLEEPDEEP);
        arm_wfi();
        // 唤醒：恢复 SLEEPDEEP=0
        core::ptr::write_volatile(SCB_SCR, core::ptr::read_volatile(SCB_SCR) & !SCR_SLEEPDEEP);
    }
}

#[cfg(not(target_arch = "arm"))]
#[allow(unused_variables)]
fn enter_deep_sleep() {
    unimplemented!("deep_sleep 需要 ARM 目标（thumbv6m-none-eabi）");
}

/// 等待事件（WFE）——事件驱动主循环用；不会主动休眠到中断
///
/// # Panics（非 ARM 目标）
pub fn wait_for_event() {
    #[cfg(target_arch = "arm")]
    arm_wfe();
    #[cfg(not(target_arch = "arm"))]
    unimplemented!("wfe 需要 ARM 目标");
}

/// 类型别名占位（保持 API 形状稳定；当前入口无 Infallible 用例）
#[allow(dead_code)]
type _Never = Infallible;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scr_layout() {
        // M0+ SCR：bit1=SLEEPONEXIT、bit2=SLEEPDEEP；SEVONPEND=bit4
        assert_eq!(SCR_SLEEPDEEP, 0x4);
    }
}

/// `DSB; WFI; ISB`（非 ARM 目标为空操作，保证 host 编译通过）
#[inline]
fn arm_wfi() {
    #[cfg(target_arch = "arm")]
    unsafe {
        core::arch::asm!("dsb", "wfi", "isb");
    }
}

/// `WFE`（非 ARM 目标为空操作）
#[allow(dead_code)]
#[inline]
fn arm_wfe() {
    #[cfg(target_arch = "arm")]
    unsafe {
        core::arch::asm!("wfe");
    }
}
