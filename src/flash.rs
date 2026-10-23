//! FLASH 自编程（§7.5 / §7.6）
//!
//! 64KB / 128 页 / **页 512 字节**；支持按页擦除与 32 位字编程，擦写锁定按
//! “每 8 页一个 LOCKx 位”分组（表 7-2）。
//!
//! # 三条硬性约束（违反会静默失败或置错误标志）
//!
//! 1. **只能 1→0**：编程前目标区域必须已是 0xFF（擦除态），否则置 `ISR.PROG`；
//! 2. **必须先解锁**：`PAGELOCK.LOCKx=1` 才允许擦写，否则置 `ISR.PAGELOCK`；
//! 3. **不能碰 PC 所在页**：否则置 `ISR.PC` 且操作失败（硬件保护）。
//!
//! 另外擦写期间必须关掉 CACHE / FETCH（本模块自动处理，否则置 `ISR.CACHEON`）。
//!
//! # 为什么 API 是 `unsafe`
//!
//! 擦除会**不可恢复地**清掉一页。虽然本模块用链接期符号 `_etext` 把“代码区”
//! 挡在外面，但调用方仍须保证：目标页没有别的东西（常量表、校准数据、
//! bootloader 参数、另一个任务正在读的数据）。所以擦写入口一律 `unsafe`。
//!
//! # 用法
//!
//! ```ignore
//! let mut flash = Flash::new(p.flashctrl);
//! // 第 127 页（0xFE00..0xFFFF）存一条记录
//! unsafe {
//!     flash.erase_page(127)?;
//!     flash.program(0x0000_FE00, &[0xAA, 0x55, 0x01, 0x00])?;
//! }
//! assert_eq!(Flash::read_u32(0x0000_FE00), 0x0000_55AA);
//! ```

use crate::pac::{self, flash as fc};

/// FLASH 页大小（字节）
pub const PAGE_SIZE: u32 = 512;
/// 总页数（64KB / 512B）
pub const PAGES: u8 = 128;
/// FLASH 总容量（字节）
pub const FLASH_SIZE: u32 = 64 * 1024;

/// FLASH 错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashError {
    /// 页号越界（>= 128）
    BadPage,
    /// 地址落在代码区（< `_etext`），拒绝擦写
    CodeRegion,
    /// 地址/长度未按 4 字节对齐
    BadAlign,
    /// 等待 `ISR.BUSY` 清零超时
    Timeout,
    /// 目标区域不是全 0xFF（`ISR.PROG`）
    NotErased,
    /// 目标页被锁定（`ISR.PAGELOCK`）
    PageLocked,
    /// 擦写地址落在 PC 所在页（`ISR.PC`）
    PcPage,
    /// 擦写时 CACHE/FETCH 未关（`ISR.CACHEON`）
    CacheOn,
    /// 擦写区域含安全运行库（`ISR.SDKERR`）
    SdkRegion,
}

#[cfg(target_os = "none")]
extern "C" {
    /// 链接期符号：代码段结束地址（memory.x 中定义）
    static _etext: u8;
}

/// 代码区上界（该地址以下禁止擦写）
#[inline]
pub fn code_end() -> u32 {
    #[cfg(target_os = "none")]
    {
        core::ptr::addr_of!(_etext) as u32
    }
    #[cfg(not(target_os = "none"))]
    {
        0
    }
}

/// FLASH 控制器
pub struct Flash<'p> {
    regs: &'p pac::Flashctrl,
}

impl<'p> Flash<'p> {
    /// 绑定 FLASH 控制器，并**使能 FLASH 配置时钟**（AHBEN[1]，§7.4）
    ///
    /// 这一步不能省：`Rcc::new` 不会开它（只有改时钟时才顺带开），而没这个
    /// 时钟时 **CR1/CR2/PAGELOCK 的写入会被静默丢弃**——擦除和编程都变成
    /// 空操作，且 `ISR` 不报任何错误（本板实测踩坑）。
    pub fn new(regs: &'p pac::Flashctrl, rcc: &mut crate::rcc::Rcc) -> Self {
        rcc.enable_flash_clock();
        Flash { regs }
    }

    /// 读保护等级（0=Level0 未保护 … 3=Level3）
    pub fn security_level(&self) -> u8 {
        ((self.regs.cr1.read() & fc::CR1_SECURITY_MASK) >> fc::CR1_SECURITY_POS) as u8
    }

    /// 擦写是否进行中
    pub fn is_busy(&self) -> bool {
        self.regs.isr.read() & fc::ISR_BUSY != 0
    }

    /// 地址所在页号
    pub const fn page_of(addr: u32) -> u8 {
        ((addr / PAGE_SIZE) & 0x7F) as u8
    }

    /// 读一个字（任意地址，含非擦写区）
    pub fn read_u32(addr: u32) -> u32 {
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    /// 读一片字节到缓冲
    pub fn read(addr: u32, out: &mut [u8]) {
        for (i, b) in out.iter_mut().enumerate() {
            *b = unsafe { core::ptr::read_volatile((addr as usize + i) as *const u8) };
        }
    }

    /// 把 `ISR` 里的错误位翻译成 `Err`
    fn check(&self) -> Result<(), FlashError> {
        let isr = self.regs.isr.read();
        let e = if isr & fc::ISR_PC != 0 {
            Some(FlashError::PcPage)
        } else if isr & fc::ISR_PAGELOCK != 0 {
            Some(FlashError::PageLocked)
        } else if isr & fc::ISR_PROG != 0 {
            Some(FlashError::NotErased)
        } else if isr & fc::ISR_CACHEON != 0 {
            Some(FlashError::CacheOn)
        } else if isr & fc::ISR_SDKERR != 0 {
            Some(FlashError::SdkRegion)
        } else {
            None
        };
        match e {
            Some(err) => {
                // ICR 是 R1W0：写 0 清对应位，其余写 1
                self.regs.icr.write(!fc::ISR_ERRORS);
                Err(err)
            }
            None => Ok(()),
        }
    }

    fn wait_idle(&self) -> Result<(), FlashError> {
        // 页擦除典型 3~5ms，4MHz 下给足 200 万轮
        for _ in 0..2_000_000 {
            if !self.is_busy() {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(FlashError::Timeout)
    }

    /// 进入擦写模式
    ///
    /// 实测关键点（`flash_probe` 对比验证）：**MODE 必须"或"进现有 CR1**，
    /// 不能写成裸的 `KEY|mode`——直接覆盖会让编程静默失效（写入被丢弃、
    /// ISR 也不报错）。厂商 `FLASH_ErasePage` 同样是 `CR1 | 0x02`。
    ///
    /// CACHE/FETCH 只在确实开着时才关（4MHz 下复位即为 0），并返回原值供
    /// [`end`](Self::end) 恢复。进入前统一清标志（ICR 是 R1W0，写 0 清）。
    fn begin(&self, mode: u32) -> (u16, u16) {
        self.regs.icr.write(0);
        let cr1 = (self.regs.cr1.read() & 0xFFFF) as u16;
        let cr2 = (self.regs.cr2.read() & 0xFFFF) as u16;
        if cr2 as u32 & (fc::CR2_CACHE | fc::CR2_FETCH) != 0 {
            self.regs
                .cr2
                .write(fc::KEY | (cr2 as u32 & !(fc::CR2_CACHE | fc::CR2_FETCH)));
        }
        self.regs.cr1.write(fc::KEY | ((cr1 as u32) | mode));
        (cr1, cr2)
    }

    /// 退出擦写模式：恢复 CR1（含 MODE=读）与 CR2
    fn end(&self, saved: (u16, u16)) {
        self.regs.cr1.write(fc::KEY | saved.0 as u32);
        self.regs.cr2.write(fc::KEY | saved.1 as u32);
    }

    /// 解锁/锁定某一页（每 8 页共用一个 LOCKx）
    fn set_lock(&self, page: u8, unlock: bool) {
        let bit = 1u32 << (page / 8);
        let cur = self.regs.pagelock.read() & 0xFFFF;
        let v = if unlock { cur | bit } else { cur & !bit };
        self.regs.pagelock.write(fc::KEY | v);
    }

    /// 页擦除（整页变 0xFF）
    ///
    /// # Safety
    /// 该页内容会被不可恢复地清除。调用方须确认该页不存放任何在用数据。
    pub unsafe fn erase_page(&mut self, page: u8) -> Result<(), FlashError> {
        if page >= PAGES {
            return Err(FlashError::BadPage);
        }
        let addr = page as u32 * PAGE_SIZE;
        if addr < code_end() {
            return Err(FlashError::CodeRegion);
        }
        self.wait_idle()?;
        let saved = self.begin(fc::MODE_PAGE_ERASE);
        self.set_lock(page, true);
        // 步骤 4：向页内任意地址写任意数据即触发擦除（厂商用字节写）
        core::ptr::write_volatile(addr as *mut u8, 0x00);
        let busy = self.wait_idle();
        self.set_lock(page, false);
        self.end(saved);
        busy?;
        self.check()
    }

    /// 按 32 位字编程（地址与长度都必须 4 字节对齐，目标区域须已擦除）
    ///
    /// # Safety
    /// 调用方须保证 `addr..addr+len` 落在已擦除且可写的页内。
    pub unsafe fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), FlashError> {
        if addr % 4 != 0 || data.len() % 4 != 0 {
            return Err(FlashError::BadAlign);
        }
        if addr < code_end() {
            return Err(FlashError::CodeRegion);
        }
        if data.is_empty() {
            return Ok(());
        }
        if addr + data.len() as u32 > FLASH_SIZE {
            return Err(FlashError::BadPage);
        }
        self.wait_idle()?;
        let page = Self::page_of(addr);
        let saved = self.begin(fc::MODE_PROGRAM);
        self.set_lock(page, true);
        let mut res = Ok(());
        for (i, chunk) in data.chunks_exact(4).enumerate() {
            let w = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            core::ptr::write_volatile((addr as usize + i * 4) as *mut u32, w);
            if let Err(e) = self.wait_idle() {
                res = Err(e);
                break;
            }
            if let Err(e) = self.check() {
                res = Err(e);
                break;
            }
        }
        self.set_lock(page, false);
        self.end(saved);
        res
    }

    /// 擦除整页后从页首写入（一次调用搞定“写一条记录”）
    ///
    /// # Safety
    /// 同 [`erase_page`](Self::erase_page)。
    pub unsafe fn write_page(&mut self, page: u8, data: &[u8]) -> Result<(), FlashError> {
        if data.len() as u32 > PAGE_SIZE {
            return Err(FlashError::BadPage);
        }
        self.erase_page(page)?;
        self.program(page as u32 * PAGE_SIZE, data)
    }

    /// 寄存器块原始访问
    pub fn regs(&self) -> &'p pac::Flashctrl {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_math() {
        assert_eq!(Flash::page_of(0x0000_0000), 0);
        assert_eq!(Flash::page_of(0x0000_01FF), 0);
        assert_eq!(Flash::page_of(0x0000_0200), 1);
        assert_eq!(Flash::page_of(0x0000_FE00), 127);
        assert_eq!(Flash::page_of(0x0000_FFFF), 127);
        assert_eq!(PAGES as u32 * PAGE_SIZE, FLASH_SIZE);
    }
}
