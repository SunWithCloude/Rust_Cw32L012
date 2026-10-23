//! RTC 自测：跨日走时 + 闹钟（零接线）
//!
//! - **P1 走时/跨日**：设为 2026-10-23（周五）23:59:50，启动后等 12s，
//!   应读到 **2026-10-24 00:00:02（周六）**——一次验证 BCD 进位、24 小时制、
//!   日期翻转与星期递增。
//! - **P2 闹钟 A**：设"每分钟第 25 秒"响（hour/minute 屏蔽），等到整分 25 秒，
//!   查 `ISR.ALARMA`。
//!
//! 时钟源优先 LSE（外部 32.768kHz 晶体），2s 内不起振则退回 LSI
//! （内部 RC，出厂已 TRIM，实测 32768Hz）。
//!
//! 观察量：`CLK`（1=LSE、2=LSI）、`DT_BEFORE/AFTER`、`ALARM_HITS`、
//! `ALARM_AT`、`ERR`。

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    pac,
    rcc::Rcc,
    rtc::{Alarm, DateTime, Rtc, RtcClock},
    Peripherals,
};

macro_rules! put {
    ($p:expr, $v:expr) => {
        unsafe { write_volatile($p, $v) }
    };
}

/// 实际使用的时钟源：1=LSE、2=LSI、0=失败
static mut CLK: u32 = 0;
/// 设定值（year<<24 | month<<16 | day<<8 | weekday）
static mut DT_BEFORE: u32 = 0;
/// 12s 后读到的日期
static mut DT_AFTER: u32 = 0;
/// 12s 后读到的时间（hour<<16 | minute<<8 | second）
static mut TM_AFTER: u32 = 0;
/// 闹钟命中次数
static mut ALARM_HITS: u32 = 0;
/// 命中时的时间
static mut ALARM_AT: u32 = 0;
/// 错误码（0 = 无）
static mut ERR: u32 = 0;
/// 启动前后 `ISR` 快照
static mut ISR0: u32 = 0;
static mut ISR1: u32 = 0;

fn pack_date(d: &DateTime) -> u32 {
    ((d.year as u32) << 24) | ((d.month as u32) << 16) | ((d.day as u32) << 8) | d.weekday as u32
}

fn pack_time(d: &DateTime) -> u32 {
    ((d.hour as u32) << 16) | ((d.minute as u32) << 8) | d.second as u32
}

#[no_mangle]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let mut delay = Delay::new(&rcc.clocks());
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // ---- 选时钟源：LSE 优先，失败退 LSI ----
    let mut err = 0u32;
    let rtc = match Rtc::new(p.rtc, &mut rcc, RtcClock::Lse) {
        Ok(r) => {
            put!(addr_of_mut!(CLK), 1u32);
            r
        }
        Err(_) => match Rtc::new(p.rtc, &mut rcc, RtcClock::Lsi) {
            Ok(r) => {
                put!(addr_of_mut!(CLK), 2u32);
                r
            }
            Err(_) => {
                put!(addr_of_mut!(CLK), 0u32);
                loop {
                    core::hint::spin_loop();
                }
            }
        },
    };

    // ---- P1：设 23:59:50，等 12s 跨日 ----
    let t0 = DateTime {
        year: 26,
        month: 10,
        day: 23,
        weekday: 5,
        hour: 23,
        minute: 59,
        second: 50,
    };
    if rtc.set_datetime(&t0).is_err() {
        err = 1;
    }
    put!(addr_of_mut!(DT_BEFORE), pack_date(&t0));
    rtc.start();
    put!(addr_of_mut!(ISR0), rtc.flags());

    led.toggle();
    delay.delay_ms(12_000);
    let t1 = rtc.datetime();
    put!(addr_of_mut!(DT_AFTER), pack_date(&t1));
    put!(addr_of_mut!(TM_AFTER), pack_time(&t1));
    put!(addr_of_mut!(ISR1), rtc.flags());

    // ---- P2：闹钟 A = 当前秒数 +5（分钟/小时屏蔽 → 每分钟该秒触发）----
    let target = (t1.second + 5) % 60;
    rtc.set_alarm_a(&Alarm {
        weekmask: 0x7F,
        hour: None,
        minute: None,
        second: Some(target),
    });
    rtc.enable_alarm_a(true);
    rtc.clear_flags(pac::rtc::IRQ_ALARMA);

    let mut hits = 0u32;
    let mut at = 0u32;
    for _ in 0..1200 {
        // 最多等 12s
        let f = rtc.flags();
        if f & pac::rtc::IRQ_ALARMA != 0 {
            hits += 1;
            let d = rtc.datetime();
            at = pack_time(&d);
            rtc.clear_flags(pac::rtc::IRQ_ALARMA);
            break;
        }
        delay.delay_ms(10);
    }
    put!(addr_of_mut!(ALARM_HITS), hits);
    put!(addr_of_mut!(ALARM_AT), at);
    put!(addr_of_mut!(ERR), err);

    loop {
        delay.delay_ms(500);
        led.toggle();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
