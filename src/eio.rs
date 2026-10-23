//! `embedded-io` trait 实现（feature `embedded-io`）
//!
//! `embedded-hal` 1.0 里没有串口 trait（serial 从 0.2 的 `serial` 模块被移出），
//! 生态统一改用 [`embedded_io`]。实现 `Read` / `Write` 之后，UART 可以直接喂给
//! `embedded-io` 系列的协议栈与驱动：
//!
//! ```ignore
//! use embedded_io::Write;
//! writeln!(uart, "tick {}", n).unwrap();
//! ```
//!
//! 语义说明：
//! - [`Read::read`] **阻塞**到至少收到 1 个字节，返回实际读到的字节数
//!   （后续字节用非阻塞方式尽量填满，不会为了凑满缓冲区而等下去）；
//! - 接收错误（溢出 / 帧错 / 校验 / 噪声）会先于数据被检查，解码为
//!   [`UartError`]；
//! - [`Write::write`] 阻塞到全部写进发送寄存器，返回 `buf.len()`，因为本 UART
//!   没有发送 FIFO 深度信息、也不会部分失败。

use crate::uart::{Uart, UartError};
use embedded_io::{ErrorType, Read, Write};

impl ErrorType for Uart<'_> {
    type Error = UartError;
}

impl core::error::Error for UartError {}

impl embedded_io::Error for UartError {
    fn kind(&self) -> embedded_io::ErrorKind {
        use embedded_io::ErrorKind;
        match self {
            // 溢出意味着数据已经丢了，属于不可恢复的流错误
            UartError::Overrun => ErrorKind::Other,
            // 帧错 / 校验错 / 噪声都是这一字节不可信，接收器仍然可用
            UartError::Framing | UartError::Parity | UartError::Noise => ErrorKind::InvalidData,
        }
    }
}

impl Read for Uart<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        // 挂起的错误优先上报，否则 read_byte 会把它们悄悄吃掉
        if let Some(e) = self.take_error() {
            return Err(e);
        }
        buf[0] = self.read_byte();
        let mut n = 1;
        while n < buf.len() {
            match self.try_read_byte() {
                Some(b) => {
                    buf[n] = b;
                    n += 1;
                }
                None => break,
            }
        }
        Ok(n)
    }
}

impl Write for Uart<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.write_bytes(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Uart::flush(self);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 错误码映射必须稳定：溢出不能报成 InvalidData，否则上层会误以为可以续传
    #[test]
    fn error_kind_mapping() {
        assert_eq!(
            embedded_io::Error::kind(&UartError::Overrun),
            embedded_io::ErrorKind::Other
        );
        assert_eq!(
            embedded_io::Error::kind(&UartError::Framing),
            embedded_io::ErrorKind::InvalidData
        );
    }
}
