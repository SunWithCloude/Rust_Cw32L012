//! 实时时钟（§13）
//!
//! BCD 日历（年月日周 + 时分秒，自动闰年）、两个闹钟、周期中断（0.5s~1 月）、
//! 自动唤醒定时器（61µs~145h）、时间戳、RTCOUT、时钟误差补偿。
//!
//! # 时钟源（CR1.SOURCE）
//!
//! | 源 | 频率 | 说明 |
//! |---|---|---|
//! | LSE | 32768Hz | 外部晶体，最准；板上无晶振时 `enable_lse()` 返回 false |
//! | LSI | 32768Hz | 内部 RC，出厂已 TRIM（本板实测 32768Hz，误差 <1%） |
//! | HSE / HSI | 高速 | 精度差、功耗高，只做兜底 |
//!
//! 默认预分频 `PSC1=0 / PSC2=0x3FFF` 把 32768Hz 直接分成 **2Hz TICKCLK**
//! （日历每半秒走一步），调用 `Rtc::new` 后无需再配。
//!
//! # 寄存器锁定
//!
//! 除 `KEY` / `ICR` 外全部锁定，**每次写都要先 `0xCA`→`0x53` 解锁**。
//! 本模块在每次配置/写入时自动配对解锁与上锁；读日期时间寄存器还要等
//! `CR1.WAIT==0`（本模块已轮询）。
//!
//! # 用法
//!
//! ```ignore
//! let mut rtc = Rtc::new(p.rtc, &mut rcc, RtcClock::Lsi)?;   // 或 Lse
//! rtc.set_datetime(&DateTime { year: 26, month: 10, day: 23, weekday: 5,
//!                              hour: 23, minute: 59, second: 50 })?;
//! rtc.start();
//! // ... 12s 后
//! let now = rtc.datetime();     // 2026-10-24 00:00:02，星期 6
//! ```

use crate::pac::{self, rtc as rc};
use crate::rcc::Rcc;

/// RTC 时钟源
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtcClock {
    /// 外部 32.768kHz 晶体
    Lse,
    /// 内部 32.768kHz RC（出厂 TRIM）
    Lsi,
    /// 外部高速时钟
    Hse,
    /// 内部高速时钟
    Hsi,
}

impl RtcClock {
    fn code(self) -> u32 {
        match self {
            RtcClock::Lse => rc::SRC_LSE,
            RtcClock::Hse => rc::SRC_HSE,
            RtcClock::Lsi => rc::SRC_LSI,
            RtcClock::Hsi => rc::SRC_HSI,
        }
    }
}

/// 日期时间（BCD 语义，字段是**十进制值**，模块内部转 BCD）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DateTime {
    /// 年 0..=99（20xx）
    pub year: u8,
    /// 月 1..=12
    pub month: u8,
    /// 日 1..=31
    pub day: u8,
    /// 星期 0=周日、1..=6=周一到周六
    pub weekday: u8,
    /// 时 0..=23（24 小时制）
    pub hour: u8,
    /// 分 0..=59
    pub minute: u8,
    /// 秒 0..=59
    pub second: u8,
}

impl DateTime {
    /// 字段合法性（RTC 硬件也会拒写非法值，这里先挡一层）
    pub fn is_valid(&self) -> bool {
        self.year <= 99
            && (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.weekday <= 6
            && self.hour <= 23
            && self.minute <= 59
            && self.second <= 59
    }
}

/// 闹钟配置（`None` = 该字段屏蔽，即"与它无关"）
///
/// 例：`Alarm { hour: None, minute: None, second: Some(10), weekmask: 0x7F }`
/// 表示"每分钟的第 10 秒"。
#[derive(Debug, Clone, Copy)]
pub struct Alarm {
    /// 星期掩码：bit0=周日、bit1..=6=周一..周六；0x7F = 每天都响
    pub weekmask: u8,
    /// 时（None=屏蔽）
    pub hour: Option<u8>,
    /// 分（None=屏蔽）
    pub minute: Option<u8>,
    /// 秒（None=屏蔽）
    pub second: Option<u8>,
}

impl Alarm {
    /// 编码为 RTC_ALARMA/B 寄存器值（HOUREN 等三位：1 = 屏蔽）
    pub fn encode(&self) -> u32 {
        let mut v = ((self.weekmask as u32) & 0x7F) << rc::ALARM_WEEKMASK_POS;
        // 闹钟字段同样是 BCD（手册表 13-4 的 0x7F06 3000 → 06:30:00）
        match self.hour {
            Some(h) => v |= (to_bcd(h) & 0x3F) << rc::ALARM_HOUR_POS,
            None => v |= rc::ALARM_HOUREN,
        }
        match self.minute {
            Some(m) => v |= (to_bcd(m) & 0x7F) << rc::ALARM_MINUTE_POS,
            None => v |= rc::ALARM_MINUTEEN,
        }
        match self.second {
            Some(s) => v |= to_bcd(s) & rc::ALARM_SECOND_MASK,
            None => v |= rc::ALARM_SECONDEN,
        }
        v
    }
}

/// RTC 错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtcError {
    /// 时钟源未就绪（LSE 未起振 / LSI 未稳定）
    ClockNotReady,
    /// 日期时间字段非法
    BadDateTime,
    /// 等待 `CR1.WAIT` 清零超时（RTC 同步逻辑异常）
    Timeout,
}

/// RTC 驱动
pub struct Rtc<'p> {
    regs: &'p pac::Rtc,
}

/// 十进制 → BCD
#[inline]
const fn to_bcd(v: u8) -> u32 {
    ((v / 10) as u32) << 4 | (v % 10) as u32
}

/// BCD → 十进制
#[inline]
const fn from_bcd(v: u32) -> u8 {
    (((v >> 4) & 0xF) * 10 + (v & 0xF)) as u8
}

impl<'p> Rtc<'p> {
    /// 初始化：使能时钟源、开 RTC 时钟、配 24 小时制与 2Hz TICKCLK
    ///
    /// `clk` 选 `Lse` / `Lsi` 时会自动使能对应振荡器并等稳定，失败返回
    /// [`RtcError::ClockNotReady`]（此时应换 LSI 兜底）。
    pub fn new(regs: &'p pac::Rtc, rcc: &mut Rcc, clk: RtcClock) -> Result<Self, RtcError> {
        rcc.enable_rtc();
        let ok = match clk {
            RtcClock::Lse => rcc.enable_lse(),
            RtcClock::Lsi => rcc.enable_lsi(),
            _ => true,
        };
        if !ok {
            return Err(RtcError::ClockNotReady);
        }
        let me = Rtc { regs };
        // 24 小时制 + 1Hz 输出关（RTC1HZ 置 0），周期中断先关
        me.write_cr0(|v| (v & !(rc::CR0_INTERVAL_MASK | rc::CR0_RTC1HZ)) | rc::CR0_H24);
        me.write_cr1(|v| (v & !rc::CR1_SOURCE_MASK) | (clk.code() << rc::CR1_SOURCE_POS));
        // 32768Hz → TICKCLK 2Hz（复位默认值，显式写一遍更稳）
        me.regs.psc.write(rc::PSC_32768_TO_2HZ);
        Ok(me)
    }

    /// 解锁 → 执行 → 上锁（每次写寄存器都必须走这个）
    fn with_unlocked<R>(&self, f: impl FnOnce() -> R) -> R {
        self.regs.key.write(rc::KEY_UNLOCK1);
        self.regs.key.write(rc::KEY_UNLOCK2);
        let r = f();
        self.regs.key.write(rc::KEY_UNLOCK1);
        self.regs.key.write(0xFF); // 任意非 0x53 即重新上锁
        r
    }

    fn write_cr0(&self, f: impl FnOnce(u32) -> u32) {
        let v = self.regs.cr0.read();
        let nv = f(v);
        self.with_unlocked(|| self.regs.cr0.write(nv));
    }

    fn write_cr1(&self, f: impl FnOnce(u32) -> u32) {
        let v = self.regs.cr1.read();
        let nv = f(v);
        self.with_unlocked(|| self.regs.cr1.write(nv));
    }

    fn write_cr2(&self, f: impl FnOnce(u32) -> u32) {
        let v = self.regs.cr2.read();
        let nv = f(v);
        self.with_unlocked(|| self.regs.cr2.write(nv));
    }

    /// 等待计时寄存器可访问（`CR1.WAIT==0`）
    pub fn wait_ready(&self) -> Result<(), RtcError> {
        for _ in 0..500_000 {
            if self.regs.cr1.read() & rc::CR1_WAIT == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(RtcError::Timeout)
    }

    /// 启动计时
    pub fn start(&self) {
        self.write_cr0(|v| v | rc::CR0_START);
    }

    /// 停止计时（日期时间寄存器随即变为可写）
    pub fn stop(&self) {
        self.write_cr0(|v| v & !rc::CR0_START);
    }

    /// 是否正在计时
    pub fn is_running(&self) -> bool {
        self.regs.cr0.read() & rc::CR0_START != 0
    }

    /// 设置日期与时间（会自动先停表再启动，避免半写状态）
    pub fn set_datetime(&self, dt: &DateTime) -> Result<(), RtcError> {
        if !dt.is_valid() {
            return Err(RtcError::BadDateTime);
        }
        // 手册 §13.3.6：**运行中**写 DATE/TIME 才需要等 WAIT；停表后可直接写。
        // （实测：停表状态下 CR1.WAIT 不会清零，照手册等待会误报超时。）
        let was_running = self.is_running();
        if was_running {
            self.wait_ready()?;
            self.stop();
        }
        let date = ((dt.weekday as u32) << rc::DATE_WEEK_POS)
            | (to_bcd(dt.year) << rc::DATE_YEAR_POS)
            | (to_bcd(dt.month) << rc::DATE_MONTH_POS)
            | to_bcd(dt.day);
        let time = (to_bcd(dt.hour) << rc::TIME_HOUR_POS)
            | (to_bcd(dt.minute) << rc::TIME_MINUTE_POS)
            | to_bcd(dt.second);
        self.with_unlocked(|| {
            self.regs.date.write(date);
            self.regs.time.write(time);
        });
        if was_running {
            self.start();
        }
        Ok(())
    }

    /// 读当前日期时间（连读两次取稳定值，手册 §13.3.6 推荐做法）
    pub fn datetime(&self) -> DateTime {
        let mut last = self.read_once();
        for _ in 0..8 {
            let now = self.read_once();
            if now.0 == last.0 && now.1 == last.1 {
                return decode(now.0, now.1);
            }
            last = now;
        }
        decode(last.0, last.1)
    }

    fn read_once(&self) -> (u32, u32) {
        // 先 date 后 time：time 的读取触发同步，顺序反了可能跨秒
        let d = self.regs.date.read();
        let t = self.regs.time.read();
        (d, t)
    }

    /// 亚秒计数（0..=2×(PSC2+1)-1，本配置下 0..=32767；0.5s 粒度由 bit20 给出）
    pub fn subsecond(&self) -> u32 {
        self.regs.sscnt.read() & (rc::SSCNT_HALF | rc::SSCNT_LOW_MASK)
    }

    /// 配置闹钟 A（不自动使能，需再调 [`enable_alarm_a`](Self::enable_alarm_a)）
    pub fn set_alarm_a(&self, alarm: &Alarm) {
        let v = alarm.encode();
        self.with_unlocked(|| self.regs.alarma.write(v));
    }

    /// 配置闹钟 B
    pub fn set_alarm_b(&self, alarm: &Alarm) {
        let v = alarm.encode();
        self.with_unlocked(|| self.regs.alarmb.write(v));
    }

    /// 使能/关闭闹钟 A（`CR2.ALARMAEN`）
    pub fn enable_alarm_a(&self, on: bool) {
        self.write_cr2(|v| {
            if on {
                v | rc::CR2_ALARMAEN
            } else {
                v & !rc::CR2_ALARMAEN
            }
        });
    }

    /// 使能/关闭闹钟 B
    pub fn enable_alarm_b(&self, on: bool) {
        self.write_cr2(|v| {
            if on {
                v | rc::CR2_ALARMBEN
            } else {
                v & !rc::CR2_ALARMBEN
            }
        });
    }

    /// 周期中断间隔（表 13-5）：0=关、1=0.5s、2=1s、3=1min、4=1h、5=1d、6/7=1 月
    pub fn set_interval(&self, code: u8) {
        self.write_cr0(|v| (v & !rc::CR0_INTERVAL_MASK) | (code as u32 & rc::CR0_INTERVAL_MASK));
    }

    /// 自动唤醒定时器重载值（周期 = (arr+1)/唤醒时钟，§13.3.12）
    ///
    /// 唤醒时钟默认 RTCCLKD（32768Hz）再经 `CR2.AWTPRS` 分频（2/4/8/16）。
    pub fn set_wakeup(&self, arr: u16, div_sel: u8) {
        self.with_unlocked(|| self.regs.awtarr.write(arr as u32));
        self.write_cr2(|v| (v & !rc::CR2_AWTPRS_MASK) | (div_sel as u32 & rc::CR2_AWTPRS_MASK));
    }

    /// 使能/关闭自动唤醒定时器
    pub fn enable_wakeup(&self, on: bool) {
        self.write_cr2(|v| {
            if on {
                v | rc::CR2_AWTEN
            } else {
                v & !rc::CR2_AWTEN
            }
        });
    }

    /// 使能中断（掩码用 [`pac::rtc::IRQ_ALARMA`] 等）
    pub fn enable_interrupt(&self, mask: u32, on: bool) {
        let v = self.regs.ier.read();
        let nv = if on { v | mask } else { v & !mask };
        self.with_unlocked(|| self.regs.ier.write(nv));
    }

    /// 中断标志（`ISR`）
    pub fn flags(&self) -> u32 {
        self.regs.isr.read()
    }

    /// 清中断标志（`ICR` 是 R1W0：写 0 清除）
    pub fn clear_flags(&self, mask: u32) {
        self.regs.icr.write(!mask);
    }

    /// 寄存器块原始访问
    pub fn regs(&self) -> &'p pac::Rtc {
        self.regs
    }
}

fn decode(date: u32, time: u32) -> DateTime {
    DateTime {
        year: from_bcd((date & rc::DATE_YEAR_MASK) >> rc::DATE_YEAR_POS),
        month: from_bcd((date & rc::DATE_MONTH_MASK) >> rc::DATE_MONTH_POS),
        day: from_bcd(date & rc::DATE_DAY_MASK),
        weekday: ((date & rc::DATE_WEEK_MASK) >> rc::DATE_WEEK_POS) as u8,
        hour: from_bcd((time & rc::TIME_HOUR_MASK) >> rc::TIME_HOUR_POS),
        minute: from_bcd((time & rc::TIME_MINUTE_MASK) >> rc::TIME_MINUTE_POS),
        second: from_bcd(time & rc::TIME_SECOND_MASK),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcd_roundtrip() {
        for v in 0u8..=99 {
            assert_eq!(from_bcd(to_bcd(v)), v);
        }
        assert_eq!(to_bcd(59), 0x59);
        assert_eq!(to_bcd(23), 0x23);
        assert_eq!(to_bcd(9), 0x09);
    }

    #[test]
    fn datetime_encode_decode() {
        // 2026-10-23 周五 23:59:50
        let dt = DateTime {
            year: 26,
            month: 10,
            day: 23,
            weekday: 5,
            hour: 23,
            minute: 59,
            second: 50,
        };
        assert!(dt.is_valid());
        let date = (5 << rc::DATE_WEEK_POS)
            | (to_bcd(26) << rc::DATE_YEAR_POS)
            | (to_bcd(10) << rc::DATE_MONTH_POS)
            | to_bcd(23);
        let time =
            (to_bcd(23) << rc::TIME_HOUR_POS) | (to_bcd(59) << rc::TIME_MINUTE_POS) | to_bcd(50);
        assert_eq!(date, 0x0526_1023);
        assert_eq!(time, 0x0023_5950);
        assert_eq!(decode(date, time), dt);
    }

    #[test]
    fn invalid_datetime_rejected() {
        let base = DateTime {
            year: 26,
            month: 10,
            day: 23,
            weekday: 5,
            hour: 12,
            minute: 0,
            second: 0,
        };
        assert!(!DateTime { month: 13, ..base }.is_valid());
        assert!(!DateTime { day: 0, ..base }.is_valid());
        assert!(!DateTime { hour: 24, ..base }.is_valid());
        assert!(!DateTime { weekday: 7, ..base }.is_valid());
    }

    #[test]
    fn alarm_encoding_matches_manual_examples() {
        // 表 13-4 逐条：每天早上 6:30:00 → 0x7F06 3000
        let daily = Alarm {
            weekmask: 0x7F,
            hour: Some(6),
            minute: Some(30),
            second: Some(0),
        };
        assert_eq!(daily.encode(), 0x7F06_3000);
        // 每小时 50 分 → 0x7F80 5000
        let hourly = Alarm {
            weekmask: 0x7F,
            hour: None,
            minute: Some(50),
            second: Some(0),
        };
        assert_eq!(hourly.encode(), 0x7F80_5000);
        // 每分钟第 10 秒 → 0x7F80 8010（10 的 BCD 是 0x10，不是 0x0A）
        let each_min = Alarm {
            weekmask: 0x7F,
            hour: None,
            minute: None,
            second: Some(10),
        };
        assert_eq!(each_min.encode(), 0x7F80_8010);
    }

    #[test]
    fn alarm_all_none_only_matches_weekday() {
        let a = Alarm {
            weekmask: 0x01,
            hour: None,
            minute: None,
            second: None,
        };
        // weekmask[24]=0x01 + HOUREN[23] + MINUTEEN[15] + SECONDEN[7]
        assert_eq!(a.encode(), 0x0180_8080);
    }
}
