//! TS 复测第三轮：把"没试过的角度"一次性扫完
//!
//! 前两轮（`ts_probe` / `ts_probe2`）已排除：使能顺序、稳定期、BGR 软件重启、
//! ADC2、ADCCLK、采样档、真 POR、厂商例程 1:1 复刻。本轮只测**从未测过**的
//! 六条路径，一次烧录约 61s 跑完：
//!
//! - **A `BGR_CR` 未定义位扫描**：POR 后该寄存器读回 0x105（bit2/bit8 硬件
//!   自发置位）、软件重启后 0x3、60s 后 0xf。若 bit2/3/8 是"TS 偏置 trim
//!   加载/就绪"之类的隐藏位，显式置位可能把偏置拉起来。8 组掩码 × TS。
//! - **B 连续转换（CONT=1）**：TS 偏置电流可能被转换周期门控，单次模式只
//!   给 40µs 偏置时间，连续模式给 100ms。取 1ms/10ms/100ms 三个时间点。
//! - **C ADC EN 断电重启**：EN=0 → 5ms → EN=1（模拟块完整断电再上电）×3。
//! - **D 其它模拟块"顺带启动 BGR"**：手册注"BGREN、ADCEN、TSEN、VC1/2/3/4、
//!   OPA1/2 使能都会自动启动 BGR"——用 VC1 / OPA1 的使能去撞 BGR 状态机。
//! - **E 采样电容放电曲线**：先用 BGR(S166) 把采样电容充到 1.2V，再用 9 档
//!   采样时间读 TS，得到 RC 衰减曲线 → 反推 TS 节点电压与等效阻抗（比上一轮
//!   单点 S6=83 更完整）。
//! - **F 60s 预热（厂商配置 Div1/S166）**：上一轮是 Div8 跑的，这轮换成与
//!   厂商例程完全一致的 ADCCLK=4MHz + S166（41.5µs），看是否只是"档位巧合"。
//!
//! 另加两个对照：
//! - **G** 末尾 S166 ×3（与 A 首项对照，看是否随时间变化）
//! - **H** 关掉 TSEN 只留 BGREN（无偏置基线），看 TS 节点"无偏置"读数
//!
//! 所有观察量用 `write_volatile` 写入（v0.6 坑：只写不读的 static 会被优化删除）。

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};

use cw32l012_hal::{
    adc::{Adc, AdcClk},
    delay::Delay,
    gpio::{Level, Pin, PortId},
    pac,
    rcc::Rcc,
    Peripherals,
};

/// 写单个观察量
macro_rules! put {
    ($p:expr, $v:expr) => {
        write_volatile($p, $v)
    };
}

/// 按索引写数组观察量
macro_rules! put_at {
    ($arr:ident, $t:ty, $i:expr, $v:expr) => {
        write_volatile((addr_of_mut!($arr) as *mut $t).add($i), $v)
    };
}

/// 开机 BGR_CR 快照
static mut BOOT_CR: u32 = 0;
/// 开机 ADC1_CR 快照
static mut ADC_CR0: u32 = 0;
/// A：8 组 BGR_CR 掩码
static mut A_MASK: [u32; 8] = [0; 8];
/// A：掩码写回读值
static mut A_RB: [u32; 8] = [0; 8];
/// A：对应 TS 读数
static mut A_TS: [u16; 8] = [0; 8];
/// B：CONT 模式 1ms/10ms/100ms
static mut B_TS: [u16; 3] = [0; 3];
/// C：ADC EN 断电重启 ×3
static mut C_TS: [u16; 3] = [0; 3];
/// D：VC1→TS、OPA1→TS、撤掉后再 TS
static mut D_TS: [u16; 3] = [0; 3];
/// E：放电曲线（采样档 0/2/3/4/5/7/9/11/15）
static mut E_TS: [u16; 9] = [0; 9];
/// F：60s 预热 TS（每 5s）
static mut F_TS: [u16; 12] = [0; 12];
/// F：60s 预热 BGR（每 5s）
static mut F_BGR: [u16; 12] = [0; 12];
/// G：末尾 S166 ×3
static mut G_TS: [u16; 3] = [0; 3];
/// H：TSEN=0 无偏置基线 ×2
static mut H_TS: [u16; 2] = [0; 2];
/// 结束时的 BGR_CR
static mut END_CR: u32 = 0;
/// 结束时的 TS / BGR 单次读数
static mut END_TS: u16 = 0;
static mut END_BGR: u16 = 0;

/// 绕过驱动前置条件的原始单次转换（SQRCH0 直接落通道号，SAMPLE 落采样档）
#[inline]
fn raw(adc: &mut Adc, ch: u32, sample: u32) -> u16 {
    use cw32l012_hal::pac::adc as ac;
    let r = adc.regs();
    r.start.write(0);
    r.sqrcfr.write(ch);
    r.sample.write(sample);
    r.icr.write(ac::ICR_CLEAR_EOC_EOS);
    r.start.write(ac::START);
    let mut n = 1u32 << 22;
    while r.isr.read() & ac::ISR_EOC == 0 {
        if n == 0 {
            return 0xFFFF;
        }
        n -= 1;
        core::hint::spin_loop();
    }
    let v = (r.result0.read() & 0xFFF) as u16;
    r.icr.write(ac::ICR_CLEAR_EOC_EOS);
    v
}

#[no_mangle]
fn main() -> ! {
    use cw32l012_hal::pac::adc as ac;

    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let mut delay = Delay::new(&rcc.clocks());
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);
    let mut adc = Adc::new(p.adc1, &mut rcc, AdcClk::Div1); // ADCCLK = PCLK = 4MHz（厂商配置）
                                                            // OPA1 输出 = PB00（= VC1_CH3），复位是数字输入会把输出压 0 → 先放模拟态
    let _pb00 = Pin::new(p.gpiob, PortId::B, 0, &mut rcc).into_analog();

    let bgr = pac::BGR_BASE as *mut u32;

    // SAFETY: 单核独占，按相位顺序执行；BGR/VC/OPA 均为纯配置写
    unsafe {
        put!(addr_of_mut!(BOOT_CR), bgr.read_volatile());
        put!(addr_of_mut!(ADC_CR0), adc.regs().cr.read());

        // ---- A：BGR_CR 未定义位扫描（TSEN|BGREN 恒置）----
        let masks: [u32; 8] = [0x003, 0x007, 0x00B, 0x00F, 0x103, 0x107, 0x10B, 0x10F];
        for (i, m) in masks.iter().enumerate() {
            bgr.write_volatile(*m);
            delay.delay_ms(2);
            put_at!(A_MASK, u32, i, *m);
            put_at!(A_RB, u32, i, bgr.read_volatile());
            put_at!(A_TS, u16, i, raw(&mut adc, 0xE, 12)); // S166 @4MHz = 41.5µs
        }
        // 恢复到 POR 观察值（bit0/2/8）+ TSEN
        bgr.write_volatile(0x107);
        delay.delay_ms(2);

        // ---- B：连续转换模式 ----
        adc.regs().cr.modify(|v| v | ac::CR_CONT);
        adc.regs().start.write(ac::START);
        delay.delay_ms(1);
        put_at!(B_TS, u16, 0, (adc.regs().result0.read() & 0xFFF) as u16);
        delay.delay_ms(10);
        put_at!(B_TS, u16, 1, (adc.regs().result0.read() & 0xFFF) as u16);
        delay.delay_ms(100);
        put_at!(B_TS, u16, 2, (adc.regs().result0.read() & 0xFFF) as u16);
        adc.regs().cr.modify(|v| v & !ac::CR_CONT);
        adc.regs().start.write(0);

        // ---- C：ADC EN 断电重启 ×3 ----
        for i in 0..3 {
            adc.regs().cr.modify(|v| v & !ac::CR_EN);
            delay.delay_ms(5);
            adc.regs().cr.modify(|v| v | ac::CR_EN);
            delay.delay_us(200);
            put_at!(C_TS, u16, i, raw(&mut adc, 0xE, 12));
        }

        // ---- D：VC1 / OPA1 使能（自动启动 BGR）----
        rcc.enable_vc();
        let vc1_cr0 = (pac::VC1_BASE + 0x04) as *mut u32;
        let vc1_cr0_old = vc1_cr0.read_volatile();
        vc1_cr0.write_volatile(vc1_cr0_old | 1); // CR0[0] = EN
        delay.delay_ms(2);
        put_at!(D_TS, u16, 0, raw(&mut adc, 0xE, 12));
        vc1_cr0.write_volatile(vc1_cr0_old & !1);

        rcc.enable_opa();
        let opa1_cr = pac::OPA1_BASE as *mut u32;
        let opa1_cr_old = opa1_cr.read_volatile();
        opa1_cr.write_volatile((opa1_cr_old & !0x6) | 0x7); // 跟随器 + EN
        delay.delay_ms(2);
        put_at!(D_TS, u16, 1, raw(&mut adc, 0xE, 12));
        opa1_cr.write_volatile(opa1_cr_old);
        delay.delay_ms(2);
        put_at!(D_TS, u16, 2, raw(&mut adc, 0xE, 12));

        // ---- E：采样电容放电曲线（BGR 预充 → TS 各档采样）----
        let samples: [u32; 9] = [0, 2, 3, 4, 5, 7, 9, 11, 15];
        for (i, s) in samples.iter().enumerate() {
            let _ = raw(&mut adc, 0xF, 15); // 预充：S518 读 BGR ≈1.2V
            put_at!(E_TS, u16, i, raw(&mut adc, 0xE, *s));
        }

        // ---- F：60s 预热（Div1/S166，厂商配置）----
        for i in 0..12 {
            put_at!(F_TS, u16, i, raw(&mut adc, 0xE, 12));
            put_at!(F_BGR, u16, i, raw(&mut adc, 0xF, 12));
            led.toggle();
            delay.delay_ms(5000);
        }

        // ---- G：末尾 S166 ×3 ----
        for i in 0..3 {
            put_at!(G_TS, u16, i, raw(&mut adc, 0xE, 12));
        }

        // ---- H：TSEN=0 无偏置基线 ----
        for i in 0..2 {
            bgr.write_volatile(0x001); // 只留 BGREN
            delay.delay_ms(2);
            put_at!(H_TS, u16, i, raw(&mut adc, 0xE, 12));
        }

        // ---- 收尾：恢复 TSEN|BGREN，记录终值 ----
        bgr.write_volatile(0x107);
        delay.delay_ms(2);
        put!(addr_of_mut!(END_CR), bgr.read_volatile());
        put!(addr_of_mut!(END_TS), raw(&mut adc, 0xE, 12));
        put!(addr_of_mut!(END_BGR), raw(&mut adc, 0xF, 12));
    }

    // 末尾**不能**用 wfi：WFI 会让 SWD 掉线，只能靠 reset 恢复（而 reset 会把
    // 61s 的采样从头再跑一遍）。这里空转，保证随时可读 SRAM。
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
