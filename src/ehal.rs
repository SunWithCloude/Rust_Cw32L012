//! embedded-hal 1.0 trait 实现（feature `ehal-1`，默认开启）
//!
//! 让本 HAL 对接整个 embedded 生态：任何要求 `embedded_hal::xxx` 的
//! 驱动（传感器、屏幕、EEPROM…）都能直接使用本 crate 的外设对象。
//!
//! 覆盖面：
//! - [`delay::DelayNs`](embedded_hal::delay::DelayNs) ← [`crate::delay::Delay`]
//! - [`digital`](embedded_hal::digital)（InputPin / OutputPin / StatefulOutputPin，含原子 toggle）← [`crate::gpio::Output`] / [`crate::gpio::Input`]
//! - [`spi::SpiBus`](embedded_hal::spi::SpiBus) ← [`crate::spi::Spi`]
//! - [`spi::SpiDevice`](embedded_hal::spi::SpiDevice) ← [`SpiDevice`]（总线 + 片选封装，支持事务操作）
//!
//! 关闭 feature（`default-features = false`）后 crate 回到零外部依赖。
//!
//! ```ignore
//! use cw32l012_hal::{ehal::SpiDevice, spi::Spi};
//! // 用生态驱动驱动屏幕：display-interface-spi、st7789 等
//! let iface = display_interface_spi::SPIInterface::new(spi_device, dc_pin, NoCS);
//! ```

use core::convert::Infallible;

use crate::delay::Delay;
use crate::gpio::{Input, Output};
use crate::i2c::I2c;
use crate::rcc::Clocks;
use crate::spi::Spi;

// ============================ delay ============================

impl embedded_hal::delay::DelayNs for Delay {
    #[inline]
    fn delay_ns(&mut self, ns: u32) {
        Delay::delay_ns(self, ns);
    }

    #[inline]
    fn delay_us(&mut self, us: u32) {
        Delay::delay_us(self, us);
    }

    #[inline]
    fn delay_ms(&mut self, ms: u32) {
        Delay::delay_ms(self, ms);
    }
}

// ============================ digital ============================

impl embedded_hal::digital::ErrorType for Output<'_> {
    type Error = Infallible;
}

impl embedded_hal::digital::OutputPin for Output<'_> {
    #[inline]
    fn set_low(&mut self) -> Result<(), Self::Error> {
        Output::set_low(self);
        Ok(())
    }

    #[inline]
    fn set_high(&mut self) -> Result<(), Self::Error> {
        Output::set_high(self);
        Ok(())
    }
}

impl embedded_hal::digital::StatefulOutputPin for Output<'_> {
    #[inline]
    fn is_set_high(&mut self) -> Result<bool, Self::Error> {
        Ok(Output::is_set_high(self))
    }

    #[inline]
    fn is_set_low(&mut self) -> Result<bool, Self::Error> {
        Ok(Output::is_set_low(self))
    }

    /// 覆写默认实现：用 GPIO TOG 寄存器原子翻转（读改写免锁）
    #[inline]
    fn toggle(&mut self) -> Result<(), Self::Error> {
        Output::toggle(self);
        Ok(())
    }
}

/// 输出脚回读实际电平（开漏总线场景；IDR 独立于 ODR）
impl embedded_hal::digital::InputPin for Output<'_> {
    #[inline]
    fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(Output::pin_is_high(self))
    }

    #[inline]
    fn is_low(&mut self) -> Result<bool, Self::Error> {
        Ok(!Output::pin_is_high(self))
    }
}

impl embedded_hal::digital::ErrorType for Input<'_> {
    type Error = Infallible;
}

impl embedded_hal::digital::InputPin for Input<'_> {
    #[inline]
    fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(Input::is_high(self))
    }

    #[inline]
    fn is_low(&mut self) -> Result<bool, Self::Error> {
        Ok(Input::is_low(self))
    }
}

// ============================ spi ============================

impl embedded_hal::spi::ErrorType for Spi<'_> {
    type Error = Infallible;
}

impl embedded_hal::spi::SpiBus<u8> for Spi<'_> {
    #[inline]
    fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        Spi::write(self, words);
        Ok(())
    }

    #[inline]
    fn read(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        Spi::read(self, words);
        Ok(())
    }

    /// ehal 语义：`write` 与 `read` 长度可不同——
    /// write 更长时多收的字节丢弃；read 更长时补发哑元 0x00 驱动时钟。
    fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        let n = read.len().max(write.len());
        for i in 0..n {
            let w = write.get(i).copied().unwrap_or(0);
            let r = self.transfer_byte(w);
            if let Some(slot) = read.get_mut(i) {
                *slot = r;
            }
        }
        Ok(())
    }

    #[inline]
    fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        Spi::transfer_in_place(self, words);
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        Spi::flush(self);
        Ok(())
    }
}

/// SPI 总线 + 片选的事务封装（embedded-hal `SpiDevice`）
///
/// 事务期间 CS 恒为低；`Operation::DelayNs` 用内部 Delay 实现；
/// 事务结束先等移位完成再拉高 CS（末字节不截断）。
pub struct SpiDevice<'p> {
    bus: Spi<'p>,
    cs: Output<'p>,
    delay: Delay,
}

impl<'p> SpiDevice<'p> {
    /// 创建 SPI 器件封装
    ///
    /// * `bus` — 已初始化的 [`Spi`] 主机
    /// * `cs` — 片选 GPIO 输出（创建时即为高电平）
    /// * `clocks` — 时钟快照（供 `Operation::DelayNs`）
    pub fn new(bus: Spi<'p>, cs: Output<'p>, clocks: &Clocks) -> Self {
        SpiDevice {
            bus,
            cs,
            delay: Delay::new(clocks),
        }
    }

    /// 拆回总线与片选
    pub fn release(self) -> (Spi<'p>, Output<'p>) {
        (self.bus, self.cs)
    }
}

impl embedded_hal::spi::ErrorType for SpiDevice<'_> {
    type Error = Infallible;
}

impl embedded_hal::spi::SpiDevice<u8> for SpiDevice<'_> {
    fn transaction(
        &mut self,
        operations: &mut [embedded_hal::spi::Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        use embedded_hal::spi::Operation;

        self.cs.set_low();
        for op in operations {
            match op {
                Operation::Write(buf) => self.bus.write(buf),
                Operation::Read(buf) => self.bus.read(buf),
                Operation::Transfer(read, write) => {
                    embedded_hal::spi::SpiBus::transfer(&mut self.bus, read, write)?
                }
                Operation::TransferInPlace(buf) => self.bus.transfer_in_place(buf),
                Operation::DelayNs(ns) => self.delay.delay_ns(*ns),
            }
        }
        // CS 拉高前确保移位完成
        self.bus.flush();
        self.cs.set_high();
        Ok(())
    }
}

// ============================ i2c ============================

impl embedded_hal::i2c::Error for crate::i2c::Error {
    fn kind(&self) -> embedded_hal::i2c::ErrorKind {
        use crate::i2c::Error as E;
        use embedded_hal::i2c::{ErrorKind, NoAcknowledgeSource};
        match self {
            E::NoAcknowledge => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Unknown),
            E::ArbitrationLost => ErrorKind::ArbitrationLoss,
            _ => ErrorKind::Other,
        }
    }
}

impl embedded_hal::i2c::ErrorType for I2c<'_> {
    type Error = crate::i2c::Error;
}

impl embedded_hal::i2c::I2c for I2c<'_> {
    #[inline]
    fn read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Self::Error> {
        I2c::read(self, address, read)
    }

    #[inline]
    fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Self::Error> {
        I2c::write(self, address, write)
    }

    #[inline]
    fn write_read(
        &mut self,
        address: u8,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), Self::Error> {
        I2c::write_read(self, address, write, read)
    }

    fn transaction(
        &mut self,
        address: u8,
        operations: &mut [embedded_hal::i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        use embedded_hal::i2c::Operation;
        // 事务语义：仅一次 START/STOP（write_read 恰好覆盖两段组合）
        let mut wr: Option<&[u8]> = None;
        let mut rd: Option<&mut [u8]> = None;
        for op in operations {
            match op {
                Operation::Write(w) => match wr {
                    None => wr = Some(w),
                    Some(_) => return Err(crate::i2c::Error::FifoError),
                },
                Operation::Read(r) => match rd {
                    None => rd = Some(r),
                    Some(_) => return Err(crate::i2c::Error::FifoError),
                },
            }
        }
        match (wr, rd) {
            (Some(w), Some(r)) => I2c::write_read(self, address, w, r),
            (Some(w), None) => I2c::write(self, address, w),
            (None, Some(r)) => I2c::read(self, address, r),
            (None, None) => Ok(()),
        }
    }
}
