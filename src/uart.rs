//! UART（§21）—— 阻塞式驱动
//!
//! 支持 UART1 / UART2 / UART3，固定 8-N-1 异步模式。
//!
//! 波特率公式（16 倍采样，CR1.OVER=00）：
//! `Baud = UCLK / (BRRI × 16 + BRRF)`，UCLK 默认来自 PCLK
//! （CR1.SOURCE=00）。
//!
//! 常用引脚（已逐行对照手册表 9-2；一个 UART 可有多组引脚，同一物理引脚
//! 同一时刻只能归属一个 UART）：
//!
//! | UART  | 组合 A                     | 组合 B                     |
//! |-------|----------------------------|----------------------------|
//! | UART1 | PA02=TXD / PA03=RXD（AF1） | PA09=TXD / PA10=RXD（AF1） |
//! | UART2 | PA02=TXD / PA03=RXD（AF2） | PA07=TXD / PA06=RXD（AF1） |
//! | UART3 | PA04=TXD / PA05=RXD（AF1） | PA01=TXD / PA00=RXD（AF2） |
//!
//! 注：UART1/UART2 可共用 PA02/PA03 物理引脚（AF1/AF2 二选一）；UART3
//! 另有 PA08/PA09、PA13/PA14、PB02/PB05 组合。完整表见手册表 9-2。
//!
//! 使用示例：
//! ```ignore
//! let tx = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_af_output(Af::Af1);
//! let rx = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_af_input(Af::Af1);
//! let mut uart = Uart::new(p.uart1, tx, rx, &mut rcc, clocks.pclk_hz, 115_200);
//! uart.write_str("hello\r\n");
//! ```

use crate::gpio::AfPin;
use crate::pac::{self, uart as ub};
use crate::rcc::Rcc;

/// UART 波特率计算错误（BRRI 为 0 或超出 16bit 范围）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BaudError;

/// 接收错误（由 `ISR` 的四个错误标志解码，读取时按优先级取其一）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum UartError {
    /// 溢出：数据没被及时读走，接收缓冲已覆盖
    Overrun,
    /// 帧错误：停止位不对（波特率不匹配 / 线路干扰）
    Framing,
    /// 校验错
    Parity,
    /// 噪声：采样到毛刺
    Noise,
}

impl core::fmt::Display for UartError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            UartError::Overrun => "receiver overrun (data lost)",
            UartError::Framing => "framing error (baud rate mismatch?)",
            UartError::Parity => "parity error",
            UartError::Noise => "noise detected",
        };
        f.write_str(s)
    }
}

/// 计算波特率分频：返回 `(BRRI, BRRF)`。
///
/// `Baud = UCLK / (BRRI*16 + BRRF)`（四舍五入）。
pub fn calc_brr(uclk_hz: u32, baud: u32) -> Result<(u32, u32), BaudError> {
    if baud == 0 || uclk_hz == 0 {
        return Err(BaudError);
    }
    // 四舍五入：div = round(uclk / baud)
    let div = (uclk_hz as u64 * 2 + baud as u64) / (baud as u64 * 2);
    let brri = div / 16;
    let brrf = div % 16;
    if brri == 0 || brri > 0xFFFF {
        return Err(BaudError);
    }
    Ok((brri as u32, brrf as u32))
}

/// UART 阻塞式驱动（8-N-1）
pub struct Uart<'p> {
    regs: &'p pac::Uart,
    _tx: AfPin<'p>,
    _rx: AfPin<'p>,
}

impl<'p> Uart<'p> {
    /// 初始化 UART（8-N-1，PCLK 为波特率时钟源）。
    ///
    /// * `regs` — UART1/2/3 寄存器块（按基地址自动识别并使能对应 APB1 时钟）
    /// * `tx` / `rx` — 已配置为对应复用功能的引脚（见模块文档引脚表）
    /// * `pclk_hz` — 当前 PCLK 频率
    /// * `baud` — 目标波特率
    ///
    /// # Panics
    /// 波特率超出 PCLK 支持范围（`PCLK/16 < baud` 或 BRRI 溢出）时 panic。
    pub fn new(
        regs: &'p pac::Uart,
        tx: AfPin<'p>,
        rx: AfPin<'p>,
        rcc: &mut Rcc,
        pclk_hz: u32,
        baud: u32,
    ) -> Self {
        // 按基地址自动识别 UART 实例并使能时钟
        let addr = regs as *const _ as usize;
        if addr == pac::UART1_BASE {
            rcc.enable_uart1();
        } else if addr == pac::UART2_BASE {
            rcc.enable_uart2();
        } else if addr == pac::UART3_BASE {
            rcc.enable_uart3();
        }

        let (brri, brrf) =
            calc_brr(pclk_hz, baud).expect("波特率超出 PCLK 支持范围（检查时钟配置与波特率参数）");

        // 停收发 → 配波特率 → 清标志 → 使能收发
        regs.cr1.write(0);
        regs.brri.write(brri);
        regs.brrf.write(brrf);
        regs.icr.write(0); // ICR 写 0 清除全部 13 个标志
        regs.cr1.write(ub::CR1_TXEN | ub::CR1_RXEN);

        Uart {
            regs,
            _tx: tx,
            _rx: rx,
        }
    }

    /// 阻塞发送一个字节
    pub fn write_byte(&mut self, b: u8) {
        while self.regs.isr.read() & ub::ISR_TXBE == 0 {
            core::hint::spin_loop();
        }
        self.regs.tdr.write(b as u32);
    }

    /// 阻塞接收一个字节
    ///
    /// 接收路径自动处理错误标志（ORE/NE/PE/FE）：发现错误立即清除并丢弃
    /// 残留数据，防止溢出标志挂起导致接收假死（上板常见死锁来源）。
    pub fn read_byte(&mut self) -> u8 {
        loop {
            let isr = self.regs.isr.read();
            if isr & ub::ISR_RC != 0 {
                let b = self.regs.rdr.read() as u8;
                // 清除接收完成标志（ICR 写 0 清除，其余位写 1 无动作）
                self.regs.icr.write(ub::ICR_NONE & !ub::ISR_RC);
                return b;
            }
            if isr & (ub::ISR_ORE | ub::ISR_NE | ub::ISR_PE | ub::ISR_FE) != 0 {
                // 清错误标志 + 读 RDR 释放接收缓冲
                self.regs.icr.write(ub::ICR_CLEAR_RX_ERRORS);
                let _ = self.regs.rdr.read();
            }
            core::hint::spin_loop();
        }
    }

    /// 非阻塞接收（无数据返回 `None`）；顺带清除挂起的错误标志
    pub fn try_read_byte(&mut self) -> Option<u8> {
        let isr = self.regs.isr.read();
        if isr & (ub::ISR_ORE | ub::ISR_NE | ub::ISR_PE | ub::ISR_FE) != 0 {
            self.regs.icr.write(ub::ICR_CLEAR_RX_ERRORS);
            let _ = self.regs.rdr.read();
        }
        if self.regs.isr.read() & ub::ISR_RC != 0 {
            Some(self.read_byte())
        } else {
            None
        }
    }

    /// 等待发送全部完成（含停止位，TC 标志）
    pub fn flush(&mut self) {
        while self.regs.isr.read() & ub::ISR_TC == 0 {
            core::hint::spin_loop();
        }
    }

    /// 阻塞发送字节切片
    pub fn write_bytes(&mut self, data: &[u8]) {
        for &b in data {
            self.write_byte(b);
        }
    }

    /// 阻塞发送字符串（UTF-8 字节流）
    pub fn write_str(&mut self, s: &str) {
        self.write_bytes(s.as_bytes());
    }

    /// 读取错误标志（ORE / NE / PE / FE 位）
    pub fn error_flags(&self) -> u32 {
        self.regs.isr.read() & (ub::ISR_ORE | ub::ISR_NE | ub::ISR_PE | ub::ISR_FE)
    }

    /// 清除全部接收错误标志（ORE/NE/PE/FE）并丢弃接收缓冲残留数据
    pub fn clear_errors(&mut self) {
        self.regs.icr.write(ub::ICR_CLEAR_RX_ERRORS);
        let _ = self.regs.rdr.read();
    }

    /// 解码并清除接收错误标志；无错误返回 `None`
    ///
    /// 多个标志同时挂起时按 **溢出 > 帧错 > 校验错 > 噪声** 的优先级返回其一
    /// （溢出最严重：数据已经丢了，其余只是这一字节不可信）。
    pub fn take_error(&mut self) -> Option<UartError> {
        let f = self.error_flags();
        if f == 0 {
            return None;
        }
        self.clear_errors();
        Some(if f & ub::ISR_ORE != 0 {
            UartError::Overrun
        } else if f & ub::ISR_FE != 0 {
            UartError::Framing
        } else if f & ub::ISR_PE != 0 {
            UartError::Parity
        } else {
            UartError::Noise
        })
    }

    /// 原始寄存器访问（逃生舱）
    pub fn regs(&mut self) -> &'p pac::Uart {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::calc_brr;

    #[test]
    fn brr_typical_4mhz_115200() {
        // div = round(4_000_000/115200) = 35 → 35/16=2 余 3
        let (i, f) = calc_brr(4_000_000, 115_200).unwrap();
        assert_eq!((i, f), (2, 3));
    }

    #[test]
    fn brr_typical_96mhz_115200() {
        // div = round(96_000_000/115200) = 833（833.33）→ 833/16=52 余 1；
        // 实际波特率 = 96M/833 ≈ 115246（+0.04%）
        let (i, f) = calc_brr(96_000_000, 115_200).unwrap();
        assert_eq!((i, f), (52, 1));
    }

    #[test]
    fn brr_exact_multiple() {
        // 4MHz / 250000 = 16 → brri=1, brrf=0
        let (i, f) = calc_brr(4_000_000, 250_000).unwrap();
        assert_eq!((i, f), (1, 0));
    }

    #[test]
    fn brr_too_fast_for_pclk() {
        // 4MHz / 300000 = 13.3 < 16 → brri=0 → 拒绝
        assert!(calc_brr(4_000_000, 300_000).is_err());
    }

    #[test]
    fn brr_overflow_rejected() {
        // 96MHz / 100 波特 → brri = 960_000/16 ≈ 60000 > 0xFFFF? 实际 6000 不溢出；
        // 极端值：uclk=0xFFFF*16*baud+ 边界。构造溢出：baud=1, uclk=u32::MAX 截断前
        // (0xFFFFFFFF*2+1)/(1*2) → div≈4.29e9，brri≈2.68e8 > 65535
        assert!(calc_brr(u32::MAX, 1).is_err());
    }
}

impl<'p> core::fmt::Write for Uart<'p> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        Uart::write_str(self, s);
        Ok(())
    }
}
