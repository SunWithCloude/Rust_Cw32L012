//! ADC 模数转换器（§25）
//!
//! 12-bit SAR，最高 1M SPS。本模块提供**单通道单次阻塞转换**，
//! 以及两个内部自验证通道：内置温度传感器（IN14）与 1.2V 基准
//! （IN15，配合出厂校准可反推 VDDA）。
//!
//! # 通道映射（表 25-4，ADC1 视角）
//!
//! | IN | 信号     | IN  | 信号            |
//! |----|----------|-----|-----------------|
//! | 0..7 | PA0..PA7 | 12  | DAC_OUT2（内部） |
//! | 8  | PB0      | 13  | DAC_OUT1（内部） |
//! | 9  | PB1      | 14  | 温度传感器（内部）|
//! | 10 | PB10     | 15  | 1.2V BGR（内部） |
//! | 11 | PB2      |     |                 |
//!
//! （ADC2 的外部通道映射不同：IN0=PA05 … IN9=PA12，见手册表 25-4）
//!
//! # 用法
//!
//! ```ignore
//! let mut adc = Adc::new(p.adc1, &mut rcc, AdcClk::Div4);
//! let raw = adc.convert(AdcChannel::In0, SAMPLE_24)?;     // PA0 电压
//! let t   = adc.temperature_mc()?;                        // 毫摄氏度
//! let mv  = adc.vdda_mv()?;                               // VDDA 实测
//! ```
//!
//! # 内部通道时序要求
//!
//! - `TSEN` 使能后约 **30µs** 才能转换温度通道（本模块已内置等待）
//! - 内部通道输出阻抗高，采样周期建议 ≥ 166 ADCCLK

use crate::delay::Delay;
use crate::pac::{self, adc as ac, bgr as bg};
use crate::rcc::Rcc;

/// ADC 时钟分频（ADCCLK = PCLK / N）
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AdcClk {
    Div1 = 0,
    Div2 = 1,
    Div4 = 2,
    Div8 = 3,
}

/// 采样周期档位（表 25-2，单位 ADCCLK）
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SampleCycles {
    S6 = 0,
    S7 = 1,
    S9 = 2,
    S12 = 3,
    S18 = 4,
    S24 = 5,
    S30 = 6,
    S42 = 7,
    S54 = 8,
    S70 = 9,
    S102 = 10,
    S134 = 11,
    S166 = 12,
    S198 = 13,
    S262 = 14,
    S518 = 15,
}

/// 转换通道（表 25-4）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AdcChannel {
    /// 外部引脚 / 内部信号（0..=13，映射见模块文档）
    In(u8),
    /// 内置温度传感器（IN14，仅 ADC1）
    TempSensor,
    /// 1.2V 内核基准（IN15，仅 ADC1）
    Bgr1v2,
}

impl AdcChannel {
    fn code(self) -> u32 {
        match self {
            AdcChannel::In(n) => n as u32 & 0xF,
            AdcChannel::TempSensor => ac::CH_TS,
            AdcChannel::Bgr1v2 => ac::CH_BGR,
        }
    }
}

/// ADC 错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AdcError {
    /// 等待转换完成超时
    Timeout,
    /// 校准值异常（0/0xFFFF，出厂未编程）
    BadCalibration,
    /// 内部通道在 ADC2 上不可用
    UnsupportedChannel,
    /// 温度传感器输出异常（原始值 <100 counts ≈ <0.08V，正常 ~0.78V）。
    ///
    /// 某板实测（2026-10-21 终审，`ts_probe` 判定矩阵 + 受控 POR）：
    /// 仅 TSEN / BGREN+TSEN / ≥5s 稳定期 / 24s 预热 / BGR 软件完整重启 /
    /// ADC2 通道 0xE / Div8 极限采样全部为 0；真 POR（RESETFLAG 验证）
    /// 后 BGR 恢复理论值而 TS 仍为 0——软件侧手段穷尽，判定该芯片 TS
    /// 模拟块输出级无输出（硬件个体状态），非驱动问题。
    SensorDead,
}

/// ADC（单通道单次阻塞转换 + 内部通道）
pub struct Adc<'p> {
    regs: &'p pac::Adc,
    bgr: &'static pac::Bgr,
    is_adc1: bool,
    clk: AdcClk,
    delay: Delay,
}

impl<'p> Adc<'p> {
    /// 初始化 ADC（使能 APBEN1.ADC 时钟 + 模块上电 + 模拟稳定等待）
    pub fn new(regs: &'p pac::Adc, rcc: &mut Rcc, clk: AdcClk) -> Self {
        rcc.enable_adc();
        regs.cr
            .modify(|v| (v & !ac::CR_CLK_MASK) | ac::CR_RESET | ((clk as u32) << ac::CR_CLK_POS));
        regs.cr.modify(|v| v | ac::CR_EN);
        // 模拟电路初始化（手册流程步骤 3：约 1µs）
        let mut delay = Delay::new(&rcc.clocks());
        delay.delay_us(2);
        let is_adc1 = regs as *const _ as usize == pac::ADC1_BASE;
        Adc {
            regs,
            bgr: unsafe { &*(pac::BGR_BASE as *const pac::Bgr) },
            is_adc1,
            clk,
            delay,
        }
    }

    /// 当前 ADCCLK（Hz）
    pub fn adcclk_hz(&self, pclk_hz: u32) -> u32 {
        pclk_hz / (1u32 << self.clk as u32)
    }

    /// 内部通道前置条件：确保 BGREN（及需要时的 TSEN）已使能并度过稳定期
    ///
    /// 实测要点（2026-10-21 `ts_probe` 判定矩阵）：
    /// - BGR_CR 复位默认 0x105——**BGREN 默认已开启**（厂商例程无需显式使能
    ///   的原因）；bit2/bit8 为硬件状态位（读数随运行出现，勿当错误）
    /// - TSEN 相对 BGREN 的使能顺序与 100ms 级稳定期对 TS 读数无影响
    ///   （本板 TS 读 0 的根因见 [`AdcError::SensorDead`]）；两级各等
    ///   40µs（手册 §25.9 30µs + 裕量）按保守实现保留
    fn ensure_internal_ready(&mut self, want_ts: bool) {
        let cr = self.bgr.cr.read();
        if cr & bg::CR_BGREN == 0 {
            self.bgr.cr.modify(|v| v | bg::CR_BGREN);
            self.delay.delay_us(40);
        }
        if want_ts && cr & bg::CR_TSEN == 0 {
            self.bgr.cr.modify(|v| v | bg::CR_BGREN | bg::CR_TSEN);
            self.delay.delay_us(40);
        }
    }

    /// 单通道单次转换（阻塞至 EOC，返回 0..4095）
    pub fn convert(&mut self, ch: AdcChannel, sample: SampleCycles) -> Result<u16, AdcError> {
        if !self.is_adc1 && matches!(ch, AdcChannel::TempSensor | AdcChannel::Bgr1v2) {
            return Err(AdcError::UnsupportedChannel);
        }
        // 内部通道前置条件：BGREN（IN15）/ TSEN+BGREN（IN14）
        match ch {
            AdcChannel::Bgr1v2 => self.ensure_internal_ready(false),
            AdcChannel::TempSensor => self.ensure_internal_ready(true),
            _ => {}
        }

        // 单通道：ENS=0（仅 SQRCH0），配置通道源与采样周期
        // （SQRCH0 字段位于 SQRCFR/SAMPLE 的 bit[3:0]）
        self.regs.start.write(0); // 停止并复位序列
        self.regs.sqrcfr.write(ch.code() << ac::SQRCH0_POS);
        self.regs.sample.write((sample as u32) << ac::SQRCH0_POS);
        self.regs.icr.write(ac::ICR_CLEAR_EOC_EOS);

        // 启动并等待 EOC（最长 (518+32) ADCCLK + 裕量 ≈ 1ms@12MHz）
        self.regs.start.write(ac::START);
        let mut n = 1 << 18;
        while self.regs.isr.read() & ac::ISR_EOC == 0 {
            if n == 0 {
                self.regs.start.write(0);
                return Err(AdcError::Timeout);
            }
            n -= 1;
            core::hint::spin_loop();
        }
        let r = (self.regs.result0.read() & 0xFFF) as u16;
        self.regs.icr.write(ac::ICR_CLEAR_EOC_EOS);
        Ok(r)
    }

    /// 序列扫描转换（ENS 模式，2..=8 通道，一次启动按序全部转换）
    ///
    /// * `chs` — 通道列表（依序落入 SQRCH0..SQRCH{N-1}）
    /// * `samples` — 每通道采样周期（与 `chs` 等长）
    /// * `out` — 结果缓冲（长度 ≥ chs.len()，按通道顺序填充 0..4095）
    ///
    /// 等待序列完成标志 EOS（而非逐通道 EOC），全程阻塞。
    /// 内部通道前置条件（BGREN/TSEN）自动处理；ADC2 上不可含内部通道。
    pub fn convert_sequence(
        &mut self,
        chs: &[AdcChannel],
        samples: &[SampleCycles],
        out: &mut [u16],
    ) -> Result<(), AdcError> {
        let n = chs.len();
        if n == 0 || n > 8 || samples.len() != n || out.len() < n {
            return Err(AdcError::UnsupportedChannel);
        }
        let mut sqrcfr = 0u32;
        let mut sample = 0u32;
        for (i, ch) in chs.iter().enumerate() {
            if !self.is_adc1 && matches!(ch, AdcChannel::TempSensor | AdcChannel::Bgr1v2) {
                return Err(AdcError::UnsupportedChannel);
            }
            sqrcfr |= ch.code() << (4 * i);
            sample |= (samples[i] as u32) << (4 * i);
        }
        // 内部通道前置条件（两级分步使能，见 ensure_internal_ready 文档）
        let any_bgr = chs.iter().any(|c| matches!(c, AdcChannel::Bgr1v2));
        let any_ts = chs.iter().any(|c| matches!(c, AdcChannel::TempSensor));
        self.ensure_internal_ready(any_ts);
        let _ = any_bgr;

        self.regs.start.write(0); // 复位序列
        self.regs.sqrcfr.write(sqrcfr);
        self.regs.sample.write(sample);
        self.regs.icr.write(ac::ICR_CLEAR_EOC_EOS);
        self.regs
            .cr
            .modify(|v| (v & !ac::CR_ENS_MASK) | ((n as u32 - 1) << ac::CR_ENS_POS));

        self.regs.start.write(ac::START);
        let mut spins = 1 << 19;
        while self.regs.isr.read() & ac::ISR_EOS == 0 {
            if spins == 0 {
                self.regs.start.write(0);
                return Err(AdcError::Timeout);
            }
            spins -= 1;
            core::hint::spin_loop();
        }
        let results = [
            self.regs.result0.read(),
            self.regs.result1.read(),
            self.regs.result2.read(),
            self.regs.result3.read(),
            self.regs.result4.read(),
            self.regs.result5.read(),
            self.regs.result6.read(),
            self.regs.result7.read(),
        ];
        for (slot, r) in out.iter_mut().zip(results.iter()) {
            *slot = (r & 0xFFF) as u16;
        }
        self.regs.icr.write(ac::ICR_CLEAR_EOC_EOS);
        // 恢复单通道模式（ENS=0），避免污染后续 convert()
        self.regs.cr.modify(|v| v & !ac::CR_ENS_MASK);
        Ok(())
    }

    /// 测量 VDDA（经 1.2V 基准通道 + 出厂校准，返回 mV）
    ///
    /// 公式（手册 §25.8 步骤 13）：`VDD = cal × 4095 / RESULT0`，
    /// cal 为 0x0010_07D2 处的 16-bit 工厂校准值（单位 mV）。
    pub fn vdda_mv(&mut self) -> Result<u32, AdcError> {
        let cal = unsafe { (pac::calib::VDDA_ADDR as *const u16).read_volatile() };
        if cal == 0 || cal == 0xFFFF {
            return Err(AdcError::BadCalibration);
        }
        let raw = self.convert(AdcChannel::Bgr1v2, SampleCycles::S166)?;
        if raw == 0 {
            return Err(AdcError::BadCalibration);
        }
        Ok((cal as u32 * 4095) / raw as u32)
    }

    /// 测量环境温度（返回毫摄氏度 m℃，经 VDDA 自校准）
    ///
    /// 公式（手册 §25.9）：
    /// `T = T0×0.5 + 0.095765×(Vref×AdcValue − 4.095×Trim)`
    /// 其中 T0@0x0010_07CD（0.5℃ 单位）、Trim@0x0010_07CE（16bit）。
    pub fn temperature_mc(&mut self) -> Result<i32, AdcError> {
        let vdda = self.vdda_mv()?;
        self.temperature_mc_with_vref(vdda)
    }

    /// 指定 Vref（mV）测量温度（避免重复测 VDDA）
    pub fn temperature_mc_with_vref(&mut self, vref_mv: u32) -> Result<i32, AdcError> {
        let t0 = unsafe { (pac::calib::TS_T0_ADDR as *const u8).read_volatile() } as i64;
        let trim = unsafe { (pac::calib::TS_TRIM_ADDR as *const u16).read_volatile() } as i64;
        if trim == 0 || trim == 0xFFFF {
            return Err(AdcError::BadCalibration);
        }
        let adc = self.convert(AdcChannel::TempSensor, SampleCycles::S262)? as i64;
        // 传感器失效保护：正常室温下 TS 输出 ≈0.6~0.9V（raw 750~1100），
        // 读到 <100 counts（≈<0.08V）说明传感器无输出——拒绝计算，
        // 避免输出"-271℃（绝对零度）"式假数据
        if adc < 100 {
            return Err(AdcError::SensorDead);
        }
        // t_mC = T0×500 + 95765×(Vref_mv×adc − 4095×trim) / 1e6
        // （0.095765 °C/V × diff/1000 V × 1000 m℃ = 95765×diff/1e6 m℃）
        let diff = vref_mv as i64 * adc - 4095 * trim;
        let t_mc = t0 * 500 + 95_765 * diff / 1_000_000;
        Ok(t_mc as i32)
    }

    /// 寄存器块原始访问
    pub fn regs(&mut self) -> &'p pac::Adc {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_codes() {
        assert_eq!(AdcChannel::In(0).code(), 0);
        assert_eq!(AdcChannel::In(11).code(), 11);
        assert_eq!(AdcChannel::TempSensor.code(), 0xE);
        assert_eq!(AdcChannel::Bgr1v2.code(), 0xF);
    }

    #[test]
    fn temperature_formula_matches_manual_example() {
        // 手册 §25.9 示例：Vref=3.3V adc=0x3CD trim=0x311 T0=0x32 → 24.65℃
        let t0 = 0x32 as i64;
        let trim = 0x311 as i64;
        let adc = 0x3CD as i64;
        let diff = 3300i64 * adc - 4095 * trim;
        let t_mc = t0 * 500 + 95_765 * diff / 1_000_000;
        assert!(
            (24_650 - t_mc).abs() < 5,
            "手册示例应得 24.65℃，实际 {t_mc} m℃"
        );
    }

    #[test]
    fn vdda_formula_sanity() {
        // Vref=3.3V 时测 1.2V 基准：raw ≈ 1200/3300×4095 ≈ 1489
        // VDD = cal×4095/raw（cal 为 1.2V 基准的出厂校准值，mV，≈1200）
        let raw = (1200u64 * 4095) / 3300;
        let vdd = 1200u64 * 4095 / raw;
        assert!((vdd as i64 - 3300).abs() <= 5, "回代应 ≈3300mV，实际 {vdd}");
    }
}
