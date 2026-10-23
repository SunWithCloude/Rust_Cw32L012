//! I2C 主机驱动（§23）
//!
//! 阻塞式 7-bit 寻址主机，支持标准模式（100k）/快速模式（400k），
//! 时序参数按 I2C 规范推导并对照手册表 23-4 校准（±1 TCLKD 内）。
//!
//! 引脚复用（手册表 9-2，均需配为 AF 复用 + 开漏输出）：
//!
//! | I2C  | SCL                | SDA                |
//! |------|--------------------|--------------------|
//! | I2C1 | PA04 / PA09 (AF2)  | PA05 / PA10 (AF2)  |
//!
//! # 用法（I2C1：PA04=SCL、PA05=SDA，100kHz）
//!
//! ```ignore
//! let scl = Pin::new(p.gpioa, PortId::A, 4, &mut rcc)
//!     .into_af_output(Af::Af2);
//! scl.enable_open_drain();           // I2C 总线必须开漏！
//! let sda = Pin::new(p.gpioa, PortId::A, 5, &mut rcc)
//!     .into_af_output(Af::Af2);
//! sda.enable_open_drain();
//! let mut i2c = I2c::new(p.i2c1, scl, sda, &mut rcc,
//!     pclk_hz, I2cSpeed::Standard100k);
//! i2c.write(0x68, &[0x75])?;         // MPU6050 WHO_AM_I 指针
//! let mut id = [0u8; 1];
//! i2c.write_read(0x68, &[0x75], &mut id)?; // 0x68
//! ```
//!
//! # 超时
//!
//! 所有等待均带超时（约 125ms@任意 PCLK），总线卡死（SCL 被从机
//! 拉住等）返回 [`Error::PinLowTimeout`] 而不是永久阻塞。

use crate::gpio::AfPin;
use crate::pac::{self, i2c as ic};
use crate::rcc::Rcc;

/// I2C 速率档
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I2cSpeed {
    /// 标准模式 100 kHz
    Standard100k,
    /// 快速模式 400 kHz
    Fast400k,
}

/// I2C 时序参数（按 I2C 规范 + 手册表 23-4 拟合）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// TCLKD = TCLK / 2^PRESCALE
    pub prescale: u8,
    /// 起始保持 / 重复起始建立保持 / 停止建立（TCLKD 单位）
    pub sethold: u8,
    /// SCL 高电平周期（TCLKD 单位）
    pub clkhi: u8,
    /// SCL 低电平周期（TCLKD 单位）
    pub clklo: u8,
    /// SDA 数据保持（TCLKD 单位）
    pub datavd: u8,
}

/// 时序计算失败（PCLK 超出该速率档支持范围）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingError;

/// 按规范推导 I2C 时序（tLOW/tHIGH/tHD;STA 最小值 + 4 TCLKD 同步开销）。
///
/// 数值已对照手册表 23-4 官方示例校准（8MHz/48MHz @400k 误差 ≤1 TCLKD）。
pub fn calc_timing(pclk_hz: u32, speed: I2cSpeed) -> Result<Timing, TimingError> {
    // （目标频率, tLOW ns, tHD;STA ns）—— I2C 规范最小值
    let (f, t_low_ns, t_hd_sta_ns): (u32, u64, u64) = match speed {
        I2cSpeed::Standard100k => (100_000, 4_700, 4_000),
        I2cSpeed::Fast400k => (400_000, 1_300, 600),
    };
    for p in 0u8..=7 {
        let tclkd = (pclk_hz >> p) as u64;
        if tclkd == 0 {
            break;
        }
        let period = tclkd / f as u64; // 每个 SCL 周期的 TCLKD 数
        if period < 10 {
            continue; // 分频不足，继续加大 prescale
        }
        // CLKLO：不小于规范 tLOW（向上取整，≤63）
        let clklo = (t_low_ns * tclkd + 999_999_999) / 1_000_000_000;
        if clklo > 63 {
            continue;
        }
        // 同步 + 输出延迟开销：由表 23-4 拟合（8M/400k 与 48M/400k 均吻合）
        const SYNC: u64 = 4;
        let clkhi = period.saturating_sub(clklo + SYNC);
        if !(2..=63).contains(&clkhi) {
            continue;
        }
        // SETHOLD：不小于规范 tHD;STA（向下取整，官方表同款）
        let sethold = (t_hd_sta_ns * tclkd / 1_000_000_000).clamp(1, 63);
        // DATAVD：≈250ns（规范 tHD;DAT 上限 3.45µs 内的安全值）
        let datavd = (250 * tclkd / 1_000_000_000).clamp(1, 63);
        return Ok(Timing {
            prescale: p,
            sethold: sethold as u8,
            clkhi: clkhi as u8,
            clklo: clklo as u8,
            datavd: datavd as u8,
        });
    }
    Err(TimingError)
}

/// I2C 错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// 地址或数据被 NACK
    NoAcknowledge,
    /// 多主机仲裁丢失
    ArbitrationLost,
    /// FIFO 出错
    FifoError,
    /// 引脚低电平超时（从机拉住 SCL/SDA）
    PinLowTimeout,
    /// 等待超时（总线卡死）
    Timeout,
}

/// I2C 主机（阻塞式，7-bit 地址）
pub struct I2c<'p> {
    regs: &'p pac::I2c,
    _scl: AfPin<'p>,
    _sda: AfPin<'p>,
}

impl<'p> I2c<'p> {
    /// 初始化 I2C 主机
    ///
    /// * `regs` — I2C1/2 寄存器块（按基地址自动使能 APB2 时钟）
    /// * `scl` / `sda` — 已配置为 AF 复用 + **开漏输出**的引脚
    /// * `pclk_hz` — 当前 PCLK 频率（MCR0.CLKSRC 默认 PCLK）
    /// * `speed` — 速率档
    ///
    /// # Panics
    /// PCLK 无法满足所选速率档的规范时序时 panic。
    pub fn new(
        regs: &'p pac::I2c,
        scl: AfPin<'p>,
        sda: AfPin<'p>,
        rcc: &mut Rcc,
        pclk_hz: u32,
        speed: I2cSpeed,
    ) -> Self {
        let t = match calc_timing(pclk_hz, speed) {
            Ok(t) => t,
            Err(_) => panic!("I2C 时序无法满足：检查 PCLK 与速率档组合"),
        };

        let addr = regs as *const _ as usize;
        if addr == pac::I2C1_BASE {
            rcc.enable_i2c1();
        } else if addr == pac::I2C2_BASE {
            rcc.enable_i2c2();
        }

        // 复位 FIFO → 配时序 → 开漏引脚配置 → 使能主机
        regs.mcr0.write(ic::MCR0_TXFIFORST | ic::MCR0_RXFIFORST);
        regs.mcr2.write(
            (t.prescale as u32 & ic::MCR2_PRESCALE_MASK) << ic::MCR2_PRESCALE_POS
                | ic::MCR2_PINCFG_OD_IO,
        );
        regs.mccr.write(
            (t.datavd as u32) << ic::MCCR_DATAVD_POS
                | (t.sethold as u32) << ic::MCCR_SETHOLD_POS
                | (t.clkhi as u32) << ic::MCCR_CLKHI_POS
                | (t.clklo as u32) << ic::MCCR_CLKLO_POS,
        );
        regs.micr.write(0); // R1W0：写 0 清除全部标志
        regs.mcr0.write(ic::MCR0_MEN | ic::MCR0_CLKSRC_PCLK);

        I2c {
            regs,
            _scl: scl,
            _sda: sda,
        }
    }

    /// 等待 TXE（带超时）
    fn wait_txe(&mut self) -> Result<(), Error> {
        let mut n = TIMEOUT_ITERS;
        while self.regs.misr.read() & ic::MISR_TXE == 0 {
            if n == 0 {
                return Err(Error::Timeout);
            }
            n -= 1;
            core::hint::spin_loop();
        }
        Ok(())
    }

    /// 等待 RXNE（带超时）
    fn wait_rxne(&mut self) -> Result<(), Error> {
        let mut n = TIMEOUT_ITERS;
        while self.regs.misr.read() & ic::MISR_RXNE == 0 {
            if n == 0 {
                return Err(Error::Timeout);
            }
            n -= 1;
            core::hint::spin_loop();
        }
        Ok(())
    }

    /// 等待主机空闲（STOP 完成，带超时）
    fn wait_idle(&mut self) -> Result<(), Error> {
        let mut n = TIMEOUT_ITERS;
        while self.regs.misr.read() & ic::MISR_MSTBUSY != 0 {
            if n == 0 {
                return Err(Error::Timeout);
            }
            n -= 1;
            core::hint::spin_loop();
        }
        Ok(())
    }

    /// 检查 NACK / 仲裁 / FIFO 错误并转为 Error
    fn check_errors(&mut self) -> Result<(), Error> {
        let isr = self.regs.misr.read();
        if isr & (ic::MISR_NACK | ic::MISR_ARBI | ic::MISR_FIFO | ic::MISR_PINLOW) != 0 {
            let e = if isr & ic::MISR_NACK != 0 {
                Error::NoAcknowledge
            } else if isr & ic::MISR_ARBI != 0 {
                Error::ArbitrationLost
            } else if isr & ic::MISR_PINLOW != 0 {
                Error::PinLowTimeout
            } else {
                Error::FifoError
            };
            // 清除全部错误标志（R1W0：写 0 清除）
            self.regs.micr.write(0);
            // 出错后补发 STOP 释放总线
            self.regs.mtdr.write(ic::CMD_STOP << ic::MTDR_CMD_POS);
            return Err(e);
        }
        Ok(())
    }

    /// 发送一条 MTDR 命令（等待 TXE + 错误检查）
    fn cmd(&mut self, cmd: u32, data: u8) -> Result<(), Error> {
        self.wait_txe()?;
        self.check_errors()?;
        self.regs
            .mtdr
            .write((cmd << ic::MTDR_CMD_POS) | data as u32);
        Ok(())
    }

    /// (Repeated)START + 发送地址字节，校验 ACK
    fn start_addr(&mut self, addr7: u8, read: bool, expect_nack: bool) -> Result<(), Error> {
        let byte = (addr7 << 1) | read as u8;
        let cmd = if expect_nack {
            ic::CMD_START_NACK
        } else {
            ic::CMD_START_ACK
        };
        self.cmd(cmd, byte)?;
        // 等地址字节被移出（TXE 恢复）后检查 NACK
        self.wait_txe()?;
        self.check_errors()?;
        Ok(())
    }

    /// 阻塞发送（START + ADDR+W + 数据 + STOP）
    pub fn write(&mut self, addr7: u8, bytes: &[u8]) -> Result<(), Error> {
        self.start_addr(addr7, false, false)?;
        for &b in bytes {
            self.cmd(ic::CMD_TX, b)?;
            self.wait_txe()?;
            self.check_errors()?;
        }
        self.cmd(ic::CMD_STOP, 0)?;
        self.wait_idle()
    }

    /// 阻塞接收（START + ADDR+R + 读 n 字节 + STOP）
    pub fn read(&mut self, addr7: u8, buf: &mut [u8]) -> Result<(), Error> {
        if buf.is_empty() {
            return Ok(());
        }
        self.start_addr(addr7, true, false)?;
        // 接收 n 字节：CMD=001，DATA=n-1（硬件收 DATA+1 个）
        self.cmd(ic::CMD_RXCOUNT, (buf.len() - 1) as u8)?;
        for slot in buf.iter_mut() {
            self.wait_rxne()?;
            let r = self.regs.mrdr.read();
            if r & ic::MRDR_EMPTY != 0 {
                self.regs.micr.write(0);
                self.regs.mtdr.write(ic::CMD_STOP << ic::MTDR_CMD_POS);
                return Err(Error::FifoError);
            }
            *slot = (r & ic::MRDR_DATA_MASK) as u8;
        }
        self.cmd(ic::CMD_STOP, 0)?;
        self.wait_idle()
    }

    /// 写后读（Repeated START，不释放中间总线）——读寄存器的标准姿势
    pub fn write_read(&mut self, addr7: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), Error> {
        self.start_addr(addr7, false, false)?;
        for &b in wr {
            self.cmd(ic::CMD_TX, b)?;
            self.wait_txe()?;
            self.check_errors()?;
        }
        if rd.is_empty() {
            self.cmd(ic::CMD_STOP, 0)?;
            return self.wait_idle();
        }
        self.start_addr(addr7, true, false)?;
        self.cmd(ic::CMD_RXCOUNT, (rd.len() - 1) as u8)?;
        for slot in rd.iter_mut() {
            self.wait_rxne()?;
            let r = self.regs.mrdr.read();
            if r & ic::MRDR_EMPTY != 0 {
                self.regs.micr.write(0);
                self.regs.mtdr.write(ic::CMD_STOP << ic::MTDR_CMD_POS);
                return Err(Error::FifoError);
            }
            *slot = (r & ic::MRDR_DATA_MASK) as u8;
        }
        self.cmd(ic::CMD_STOP, 0)?;
        self.wait_idle()
    }

    /// 扫描 7-bit 地址空间，返回有 ACK 应答的地址列表（最大 120 个）
    ///
    /// 对 0x00..0x78 逐个发 START+ADDR 并立即 STOP（不产生数据传输）。
    /// 用于排查接线 / 地址错误；耗时约 120×2×10 bit / 波特率。
    pub fn scan(&mut self) -> ([u8; 120], usize) {
        let mut found = [0u8; 120];
        let mut n = 0;
        for a in 0x00u8..0x78 {
            self.start_addr(a, false, false).ok();
            // 无论成功失败都补 STOP，且必须等总线真正释放
            self.cmd(ic::CMD_STOP, 0).ok();
            let _ = self.wait_idle();
            if self.regs.misr.read() & ic::MISR_NACK == 0 && n < 120 {
                found[n] = a;
                n += 1;
            }
            self.regs.micr.write(0);
        }
        (found, n)
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::I2c {
        self.regs
    }
}

/// 超时迭代数（每迭代约 5 HCLK 周期 → 约 125ms@任意 PCLK）
const TIMEOUT_ITERS: u32 = 1 << 18;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_matches_manual_table_23_4() {
        // 表 23-4：Fast 8MHz 400kbps PRESCALE=0 SETHOLD=0x04 CLKLO=0x0B CLKHI=0x05 DATAVD=0x02
        let t = calc_timing(8_000_000, I2cSpeed::Fast400k).unwrap();
        assert_eq!(t.prescale, 0);
        assert_eq!(t.clklo, 0x0B);
        assert_eq!(t.clkhi, 0x05);
        assert_eq!(t.sethold, 0x04);
        assert_eq!(t.datavd, 0x02);

        // 表 23-4：Fast 48MHz 400Kbps PRESCALE=0 CLKLO=0x3E CLKHI=0x35
        // 本公式：CLKLO=63（ceil(1.3µs)，表值 62，+1）、CLKHI=53（=，120-63-4）
        let t = calc_timing(48_000_000, I2cSpeed::Fast400k).unwrap();
        assert_eq!(t.prescale, 0);
        assert_eq!(t.clklo, 63, "ceil(1.3µs×48M)=63（表值 62，±1 内）");
        assert_eq!(t.clkhi, 0x35, "48MHz CLKHI 应为 53（120-63-4）");
    }

    #[test]
    fn timing_std_100k_at_4mhz_default_clock() {
        // 复位默认 PCLK=4MHz，100k：period=40，CLKLO=19（4.7µs），CLKHI=17
        let t = calc_timing(4_000_000, I2cSpeed::Standard100k).unwrap();
        assert_eq!(t.prescale, 0);
        assert_eq!(t.clklo, 19);
        assert_eq!(t.clkhi, 17);
        // 实际 SCL ≈ 4MHz/(19+17+4) ≈ 100kHz
        let f = 4_000_000 / (t.clklo as u32 + t.clkhi as u32 + 4);
        assert!((90_000..=110_000).contains(&f), "实际 {f}Hz 偏离 100k");
    }

    #[test]
    fn timing_rejects_impossible_combo() {
        // 400k 需要 tLOW=1.3µs ≥ 63 TCLKD → PCLK < 48.5MHz 都可行；
        // 极低时钟（如 32kHz LSE）无法满足任何档
        assert!(calc_timing(32_768, I2cSpeed::Standard100k).is_err());
    }

    #[test]
    fn timing_fast_range_check() {
        // 400k 在 ≥8MHz 可行（4MHz 时 period=10，扣除 CLKLO=6 与同步 4
        // 后 CLKHI<2，tHIGH 达不到规范 0.6µs → 正确拒绝）
        for pclk in [8_000_000u32, 12_000_000, 24_000_000, 48_000_000, 96_000_000] {
            let t = calc_timing(pclk, I2cSpeed::Fast400k);
            assert!(t.is_ok(), "{pclk}Hz 应支持 400k");
            let t = t.unwrap();
            assert!(t.clklo <= 63 && t.clkhi <= 63);
        }
        assert!(
            calc_timing(4_000_000, I2cSpeed::Fast400k).is_err(),
            "4MHz@400k tHIGH 不足规范值，应拒绝"
        );
    }
}
