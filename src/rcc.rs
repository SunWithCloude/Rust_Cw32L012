//! 复位与时钟控制（SYSCTRL / RCC）
//!
//! 时钟树（加粗为复位默认状态）：
//!
//! ```text
//! HSIOSC 96MHz ──/24──> HSI 4MHz ──┐
//! HSE 晶体 ─────────────────────────┤
//! LSI ~32.8kHz ─────────────────────┼─> SysClk ──/1──> HCLK ──/1──> PCLK
//! LSE 32.768kHz ────────────────────┘   (HCLKPRS=2^n)  (PCLKPRS=2^n)
//! ```
//!
//! - SysClk 源切换与 HCLK/PCLK 分频写入 `CR0`；
//! - `AHBEN` / `APBEN1` / `APBEN2` 控制外设时钟门控（含配置时钟与工作时钟）；
//! - 以上寄存器写入均需 KEY=0x5A5A 写保护键（本模块已自动处理）。

use crate::gpio::PortId;
use crate::pac::{self, sysctrl as sc};

/// HSIOSC 标称频率（内部高频振荡器；实际精度取决于出厂 TRIM）
pub const HSIOSC_HZ: u32 = 96_000_000;

/// SysClk 时钟源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysClkSource {
    /// 内部高频时钟 HSI
    Hsi,
    /// 外部高频晶体 HSE
    Hse,
    /// PLL 输出（编码 0b010；本 HAL 未建模 PLL 寄存器 SYSCTRL_PLL@0x28，
    /// 使用前需通过 [`Rcc::sysctrl`] 逃生舱自行配置 PLL 并等待稳定）
    Pll,
    /// 内部低频时钟 LSI
    Lsi,
    /// 外部低频晶体 LSE
    Lse,
}

impl SysClkSource {
    /// CR0.SYSCLK[2:0] 编码
    pub const fn bits(self) -> u32 {
        match self {
            Self::Hsi => sc::SYSCLK_HSI,
            Self::Hse => sc::SYSCLK_HSE,
            Self::Pll => 0b010, // 官方 SYSCTRL_SYSCLKSRC_PLL（cw32l012_sysctrl.h）
            Self::Lsi => sc::SYSCLK_LSI,
            Self::Lse => sc::SYSCLK_LSE,
        }
    }
}

/// HSI 分频（HSI = HSIOSC / divisor）。
///
/// 编码依据手册 §4.7.4 查找表：0000=/32、0001~/1 … 1010=/10、1011=/12、
/// 1100=/16、1101=/20、1110=/24（复位默认）、1111=/28。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HsiDiv {
    /// HSI = HSIOSC/1
    Div1,
    /// HSI = HSIOSC/2
    Div2,
    /// HSI = HSIOSC/3
    Div3,
    /// HSI = HSIOSC/4
    Div4,
    /// HSI = HSIOSC/5
    Div5,
    /// HSI = HSIOSC/6
    Div6,
    /// HSI = HSIOSC/7
    Div7,
    /// HSI = HSIOSC/8
    Div8,
    /// HSI = HSIOSC/9
    Div9,
    /// HSI = HSIOSC/10
    Div10,
    /// HSI = HSIOSC/12
    Div12,
    /// HSI = HSIOSC/16
    Div16,
    /// HSI = HSIOSC/20
    Div20,
    /// HSI = HSIOSC/24（复位默认）
    Div24,
    /// HSI = HSIOSC/28
    Div28,
    /// HSI = HSIOSC/32
    Div32,
}

impl HsiDiv {
    /// 分频数
    pub const fn divisor(self) -> u32 {
        match self {
            Self::Div1 => 1,
            Self::Div2 => 2,
            Self::Div3 => 3,
            Self::Div4 => 4,
            Self::Div5 => 5,
            Self::Div6 => 6,
            Self::Div7 => 7,
            Self::Div8 => 8,
            Self::Div9 => 9,
            Self::Div10 => 10,
            Self::Div12 => 12,
            Self::Div16 => 16,
            Self::Div20 => 20,
            Self::Div24 => 24,
            Self::Div28 => 28,
            Self::Div32 => 32,
        }
    }

    /// HSI.DIV[14:11] 寄存器编码
    pub const fn code(self) -> u32 {
        match self {
            Self::Div1 => 1,
            Self::Div2 => 2,
            Self::Div3 => 3,
            Self::Div4 => 4,
            Self::Div5 => 5,
            Self::Div6 => 6,
            Self::Div7 => 7,
            Self::Div8 => 8,
            Self::Div9 => 9,
            Self::Div10 => 10,
            Self::Div12 => 11,
            Self::Div16 => 12,
            Self::Div20 => 13,
            Self::Div24 => 14,
            Self::Div28 => 15,
            Self::Div32 => 0,
        }
    }

    /// 由寄存器编码解析（未定义编码返回 `None`）
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => Self::Div32,
            1 => Self::Div1,
            2 => Self::Div2,
            3 => Self::Div3,
            4 => Self::Div4,
            5 => Self::Div5,
            6 => Self::Div6,
            7 => Self::Div7,
            8 => Self::Div8,
            9 => Self::Div9,
            10 => Self::Div10,
            11 => Self::Div12,
            12 => Self::Div16,
            13 => Self::Div20,
            14 => Self::Div24,
            15 => Self::Div28,
            _ => return None,
        })
    }
}

/// HCLK 分频（HCLK = SysClk / divisor，CR0.HCLKPRS）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HclkDiv {
    /// HCLK = SysClk
    Div1,
    /// HCLK = SysClk/2
    Div2,
    /// HCLK = SysClk/4
    Div4,
    /// HCLK = SysClk/8
    Div8,
    /// HCLK = SysClk/16
    Div16,
    /// HCLK = SysClk/32
    Div32,
    /// HCLK = SysClk/64
    Div64,
    /// HCLK = SysClk/128
    Div128,
}

impl HclkDiv {
    /// CR0.HCLKPRS[7:5] 编码
    pub const fn bits(self) -> u32 {
        match self {
            Self::Div1 => 0,
            Self::Div2 => 1,
            Self::Div4 => 2,
            Self::Div8 => 3,
            Self::Div16 => 4,
            Self::Div32 => 5,
            Self::Div64 => 6,
            Self::Div128 => 7,
        }
    }

    /// 分频数
    pub const fn divisor(self) -> u32 {
        1 << self.bits()
    }
}

/// PCLK 分频（PCLK = HCLK / divisor，CR0.PCLKPRS）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PclkDiv {
    /// PCLK = HCLK
    Div1,
    /// PCLK = HCLK/2
    Div2,
    /// PCLK = HCLK/4
    Div4,
    /// PCLK = HCLK/8
    Div8,
}

impl PclkDiv {
    /// CR0.PCLKPRS[4:3] 编码
    pub const fn bits(self) -> u32 {
        match self {
            Self::Div1 => 0,
            Self::Div2 => 1,
            Self::Div4 => 2,
            Self::Div8 => 3,
        }
    }

    /// 分频数
    pub const fn divisor(self) -> u32 {
        1 << self.bits()
    }
}

/// 各时钟域当前频率快照
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clocks {
    /// 系统时钟 SysClk
    pub sysclk_hz: u32,
    /// AHB 总线时钟 HCLK（SysTick 计数时钟源）
    pub hclk_hz: u32,
    /// APB 外设时钟 PCLK（UCLK=PCLK 时为 UART 波特率时钟源）
    pub pclk_hz: u32,
}

/// RCC 操作句柄（持有 SYSCTRL 与 FLASHCTRL 寄存器块引用）
pub struct Rcc {
    regs: &'static pac::Sysctrl,
    flash: &'static pac::Flashctrl,
    /// SysClk 频率记录（复位默认 4MHz；由本模块 API 维护）
    sysclk_hz: u32,
    /// HCLK 分频记录（复位默认 /1）
    hclk_div: u32,
}

impl Rcc {
    /// 创建 RCC 句柄。
    ///
    /// 复位后时钟状态：SysClk = HSI = HSIOSC/24 = 4MHz，HCLK = PCLK = SysClk，
    /// FLASH 读等待周期 WAIT=0（复位值），与手册默认状态一致。
    pub fn new(sysctrl: &'static pac::Sysctrl, flashctrl: &'static pac::Flashctrl) -> Self {
        let mut rcc = Rcc {
            regs: sysctrl,
            flash: flashctrl,
            sysclk_hz: HSIOSC_HZ / 24, // 复位默认 HSI.DIV=1110(/24)
            hclk_div: 1,
        };
        // 加载工厂 TRIM 校准（手册 §4.4.2；此时 HCLK=4MHz，频率微调
        // 不跨 FLASH 等待档位，取指安全）
        rcc.load_factory_hsi_trim();
        rcc
    }

    /// 当前 SysClk 频率（本模块维护的快照）
    pub fn sysclk_hz(&self) -> u32 {
        self.sysclk_hz
    }

    /// 当前 HCLK 频率（SysClk / HCLK 分频）
    pub fn hclk_hz(&self) -> u32 {
        self.sysclk_hz / self.hclk_div
    }

    /// HSI 振荡器是否已稳定（HSI.STABLE）
    pub fn hsi_stable(&self) -> bool {
        self.regs.hsi.read() & sc::HSI_STABLE != 0
    }

    /// 修改 HSI 分频。SysClk 随之更新为 HSIOSC/div（假定 SysClk 源为 HSI）。
    ///
    /// 严格按手册 §4.5.2「HSI 时钟不同频率间切换流程」实现：
    /// 1. 先按当前/目标中**较高** HCLK 频率提升 FLASH 读等待周期（防止变频
    ///    瞬间取指出错）；
    /// 2. 保持 TRIM 不变，仅修改 HSI.DIV；
    /// 3. 按新频率回落 FLASH 读等待周期，并在 WAIT≠0 时使能 CACHE/FETCH
    ///    （手册建议）。
    ///
    /// HSI 分频切换无需等待稳定（手册明确）。满速运行（>72MHz）请确认
    /// 电压范围支持（见数据手册）。
    pub fn set_hsi_div(&mut self, div: HsiDiv) -> &mut Self {
        let new_hz = HSIOSC_HZ / div.divisor();
        let new_hclk = new_hz / self.hclk_div;
        // 步骤 1：按较高频率先提升 FLASH 等待周期
        self.set_flash_latency_for_hclk(new_hclk.max(self.hclk_hz()));
        // 步骤 2：仅改 DIV，保持 TRIM（读-改-写天然保留）
        self.regs
            .hsi
            .modify(|v| (v & !sc::HSI_DIV_MASK) | (div.code() << sc::HSI_DIV_POS));
        self.sysclk_hz = new_hz;
        // 步骤 4：按新频率配置 FLASH 等待周期（可为零则回落）
        self.set_flash_latency_for_hclk(new_hclk);
        self
    }

    /// 设置 HCLK 分频（HCLK = SysClk / div）。
    ///
    /// 自动同步 FLASH 读等待周期（HCLK 变高时先提升等待再分频，
    /// 变低时先分频再回落等待）。
    pub fn set_hclk_div(&mut self, div: HclkDiv) -> &mut Self {
        // 先取旧 HCLK 快照（self.hclk_div 赋值后 hclk_hz() 即为新值）
        let old_hclk = self.hclk_hz();
        let new_hclk = self.sysclk_hz / div.divisor();
        if new_hclk > old_hclk {
            // 频率升高：先加等待周期
            self.set_flash_latency_for_hclk(new_hclk);
        }
        self.regs
            .cr0
            .modify(|v| (v & !sc::HCLKPRS_MASK) | (div.bits() << sc::HCLKPRS_POS) | sc::KEY);
        self.hclk_div = div.divisor();
        if new_hclk < old_hclk {
            // 频率降低：再回落等待周期
            self.set_flash_latency_for_hclk(new_hclk);
        }
        self
    }

    /// 便捷方法：SysClk 满速运行（HSI = HSIOSC/1 = 96MHz，HCLK=PCLK=96MHz）。
    ///
    /// 完成动作：FLASH 等待周期提升至 3 并使能 CACHE/FETCH → HSI.DIV=1。
    /// 前提：供电电压范围支持 96MHz（见数据手册），否则请用
    /// [`set_hsi_div`](Self::set_hsi_div) 选择较低频率。
    pub fn run_hsi_at_96mhz(&mut self) -> &mut Self {
        self.set_hsi_div(HsiDiv::Div1)
    }

    /// 设置 PCLK 分频（PCLK = HCLK / div）
    pub fn set_pclk_div(&mut self, div: PclkDiv) -> &mut Self {
        self.regs
            .cr0
            .modify(|v| (v & !sc::PCLKPRS_MASK) | (div.bits() << sc::PCLKPRS_POS) | sc::KEY);
        self
    }

    /// 切换 SysClk 源（假定目标时钟已由用户配置并稳定）。
    ///
    /// 使用 HSE/LSI/LSE 时，请随后调用 [`set_sysclk_hz`](Self::set_sysclk_hz)
    /// 登记实际频率，否则 [`clocks`](Self::clocks) 结果失真。
    ///
    /// 注意（手册 §4.5 步骤 4）：切换到**更高**频率时钟源前，应先调用
    /// [`set_flash_wait`](Self::set_flash_wait) 按新旧频率较高者提升
    /// FLASH 等待周期；降频方向无需提前处理（登记频率时自动回落）。
    pub fn set_sysclk_source(&mut self, src: SysClkSource) -> &mut Self {
        self.regs.cr0.modify(|v| (v & !0x7) | src.bits() | sc::KEY);
        if src == SysClkSource::Hsi {
            let code = (self.regs.hsi.read() >> sc::HSI_DIV_POS) & 0xF;
            if let Some(d) = HsiDiv::from_code(code) {
                self.sysclk_hz = HSIOSC_HZ / d.divisor();
            }
        }
        self
    }

    /// 手动登记 SysClk 频率（使用 HSE/LSI/LSE 等外部时钟时）。
    ///
    /// 登记时按新 HCLK 自动收敛 FLASH 读等待周期：降频立即回落，
    /// 提频则提升到新频率所需值（但按手册 §4.5，提频应在切换源
    /// **之前**完成等待周期提升，见 [`set_sysclk_source`] 注记）。
    pub fn set_sysclk_hz(&mut self, hz: u32) -> &mut Self {
        self.sysclk_hz = hz;
        self.set_flash_latency_for_hclk(self.hclk_hz());
        self
    }

    /// 读取当前时钟频率（HCLK/PCLK 分频实时读取 CR0）
    pub fn clocks(&self) -> Clocks {
        let cr0 = self.regs.cr0.read();
        let hclk_div = 1u32 << ((cr0 >> sc::HCLKPRS_POS) & 0x7);
        let pclk_div = 1u32 << ((cr0 >> sc::PCLKPRS_POS) & 0x3);
        let hclk = self.sysclk_hz / hclk_div;
        Clocks {
            sysclk_hz: self.sysclk_hz,
            hclk_hz: hclk,
            pclk_hz: hclk / pclk_div,
        }
    }

    // ================= 外设时钟门控 =================

    /// 使能 GPIO 端口时钟（AHBEN）
    pub fn enable_gpio(&mut self, port: PortId) -> &mut Self {
        let bit = match port {
            PortId::A => sc::AHBEN_GPIOA,
            PortId::B => sc::AHBEN_GPIOB,
            PortId::C => sc::AHBEN_GPIOC,
            PortId::F => sc::AHBEN_GPIOF,
        };
        self.set_bits(&self.regs.ahben, bit);
        self
    }

    /// 禁用 GPIO 端口时钟
    pub fn disable_gpio(&mut self, port: PortId) -> &mut Self {
        let bit = match port {
            PortId::A => sc::AHBEN_GPIOA,
            PortId::B => sc::AHBEN_GPIOB,
            PortId::C => sc::AHBEN_GPIOC,
            PortId::F => sc::AHBEN_GPIOF,
        };
        self.clear_bits(&self.regs.ahben, bit);
        self
    }

    /// UART1 时钟使能（APBEN1[3]）
    pub fn enable_uart1(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_UART1);
        self
    }

    /// UART2 时钟使能（APBEN1[4]）
    pub fn enable_uart2(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_UART2);
        self
    }

    /// UART3 时钟使能（APBEN1[8]）
    pub fn enable_uart3(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_UART3);
        self
    }

    /// SPI1 时钟使能（APBEN1[2]）
    pub fn enable_spi1(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_SPI1);
        self
    }

    /// SPI2 时钟使能（APBEN1[13]）
    pub fn enable_spi2(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_SPI2);
        self
    }

    /// SPI3 时钟使能（APBEN1[14]）
    pub fn enable_spi3(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_SPI3);
        self
    }

    /// 禁用 SPI1 时钟
    pub fn disable_spi1(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben1, sc::APBEN1_SPI1);
        self
    }

    /// 禁用 SPI2 时钟
    pub fn disable_spi2(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben1, sc::APBEN1_SPI2);
        self
    }

    /// 禁用 SPI3 时钟
    pub fn disable_spi3(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben1, sc::APBEN1_SPI3);
        self
    }

    /// I2C1 时钟使能（APBEN2[6]）
    pub fn enable_i2c1(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_I2C1);
        self
    }

    /// I2C2 时钟使能（APBEN2[11]）
    pub fn enable_i2c2(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_I2C2);
        self
    }

    /// 禁用 I2C1 时钟
    pub fn disable_i2c1(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben2, sc::APBEN2_I2C1);
        self
    }

    /// 禁用 I2C2 时钟
    pub fn disable_i2c2(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben2, sc::APBEN2_I2C2);
        self
    }

    /// BTIM1-3 时钟使能（APBEN2[2]，三实例共用一位）
    pub fn enable_btim(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_BTIM);
        self
    }

    /// ADC 时钟使能（APBEN1[0]）
    pub fn enable_adc(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_ADC);
        self
    }

    /// 禁用 ADC 时钟
    pub fn disable_adc(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben1, sc::APBEN1_ADC);
        self
    }

    /// 使能 DMA 控制器时钟（**AHBEN[0]**，§4.7.12）
    pub fn enable_dma(&mut self) -> &mut Self {
        self.set_bits(&self.regs.ahben, sc::AHBEN_DMA);
        self
    }

    /// 禁用 DMA 控制器时钟
    pub fn disable_dma(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.ahben, sc::AHBEN_DMA);
        self
    }

    /// 禁用 BTIM1-3 时钟
    pub fn disable_btim(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben2, sc::APBEN2_BTIM);
        self
    }

    /// DAC 时钟使能（APBEN2[10]）
    pub fn enable_dac(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_DAC);
        self
    }

    /// 禁用 DAC 时钟
    pub fn disable_dac(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben2, sc::APBEN2_DAC);
        self
    }

    /// IWDT 时钟使能（APBEN2[4]；IWDT 计数时钟为独立 RC10K，不受此门控）
    pub fn enable_iwdt(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_IWDT);
        self
    }

    /// LPTIM 时钟使能（APBEN2[7]；LSI/LSE 计数时钟独立于该门控）
    pub fn enable_lptim(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_LPTIM);
        self
    }

    /// HALLTIM 时钟使能（APBEN2[12]）
    pub fn enable_halltim(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_HALLTIM);
        self
    }

    /// VC 比较器配置时钟使能（APBEN1[1]）
    pub fn enable_vc(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_VC);
        self
    }

    /// OPA 运放配置时钟使能（**APBEN2[9]**，§4.7.13）
    pub fn enable_opa(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_OPA);
        self
    }

    /// 使能 LSI 内部低速振荡器（32.8kHz）并等待稳定，返回是否成功
    ///
    /// **LSI 复位默认关闭**（手册 §4.5.5）——LPTIM / RTC 等以 LSI 为计数时钟
    /// 的外设必须先调用本函数，否则计数器完全不动作（表现为"定时器不响"）。
    ///
    /// 流程：加载出厂 TRIM（@0x0010_07C2，±1% 精度）→ CR1.LSIEN=1 →
    /// 轮询 LSI.STABLE。已在运行时直接等待稳定并返回 `true`。
    ///
    /// 返回 `false` 表示约 100ms 内未等到 STABLE（晶振异常），此时 LSI 时钟
    /// 不可信，不应据此配置定时参数。
    pub fn enable_lsi(&mut self) -> bool {
        // 出厂 TRIM（9bit 有效范围）；未编程/异常值时保留硬件默认 TRIM
        let raw = unsafe { core::ptr::read_volatile(pac::calib::LSI_TRIM_ADDR as *const u16) };
        if raw != 0 && raw != 0xFFFF && raw <= sc::LSI_TRIM_MASK as u16 {
            self.regs
                .lsi
                .modify(|v| (v & !sc::LSI_TRIM_MASK) | raw as u32);
        }
        self.set_bits(&self.regs.cr1, sc::CR1_LSIEN);
        // STABLE 典型建立时间 <1ms；给足 ~100ms（复位默认 4MHz 下忙等）
        for _ in 0..200_000 {
            if self.regs.lsi.read() & sc::LSI_STABLE != 0 {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    /// 使能 LSE 外部 32.768kHz 晶体并等待稳定（CR1[4] + LSE.STABLE，§4.5.4）
    ///
    /// 返回 `false` 表示约 2s 内未起振（板上无晶振 / 负载电容不匹配）。
    /// **先 POR 已使能过的情况下不要重复调用**：LSE 的 DRIVER/PINLOCK 只能
    /// 被上电复位，重复初始化会打断正在运行的晶体。
    pub fn enable_lse(&mut self) -> bool {
        if self.regs.lse.read() & sc::LSE_STABLE != 0 {
            return true;
        }
        self.set_bits(&self.regs.cr1, sc::CR1_LSEEN);
        // 起振典型 200~500ms，给足 ~2s（4MHz 下 100 万轮忙等）
        for _ in 0..1_000_000 {
            if self.regs.lse.read() & sc::LSE_STABLE != 0 {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    /// 关闭 LSE（LSELOCK=0 时有效）
    pub fn disable_lse(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.cr1, sc::CR1_LSEEN);
        self
    }

    /// RTC 时钟使能（APBEN2[1]）
    pub fn enable_rtc(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_RTC);
        self
    }

    /// RTC 时钟禁用
    pub fn disable_rtc(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben2, sc::APBEN2_RTC);
        self
    }

    /// WWDT 时钟使能（APBEN2[5]）
    pub fn enable_wwdt(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben2, sc::APBEN2_WWDT);
        self
    }

    /// ATIM 时钟使能（APBEN1[5]）
    pub fn enable_atim(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_ATIM);
        self
    }

    /// GTIM1 时钟使能（APBEN1[6]）
    pub fn enable_gtim1(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_GTIM1);
        self
    }

    /// GTIM2 时钟使能（APBEN1[7]）
    pub fn enable_gtim2(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_GTIM2);
        self
    }

    /// GTIM3 时钟使能（APBEN1[11]）
    pub fn enable_gtim3(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_GTIM3);
        self
    }

    /// GTIM4 时钟使能（APBEN1[12]）
    pub fn enable_gtim4(&mut self) -> &mut Self {
        self.set_bits(&self.regs.apben1, sc::APBEN1_GTIM4);
        self
    }

    /// 禁用 UART1 时钟
    pub fn disable_uart1(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben1, sc::APBEN1_UART1);
        self
    }

    /// 禁用 UART2 时钟
    pub fn disable_uart2(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben1, sc::APBEN1_UART2);
        self
    }

    /// 禁用 UART3 时钟
    pub fn disable_uart3(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.apben1, sc::APBEN1_UART3);
        self
    }

    /// 原始 APB1 时钟使能位（手册 §4.7.12；用于本 HAL 未封装的 APB1 外设）
    pub fn enable_apb1_bits(&mut self, bits: u32) -> &mut Self {
        self.set_bits(&self.regs.apben1, bits);
        self
    }

    /// 原始 APB1 时钟禁用位
    pub fn disable_apb1_bits(&mut self, bits: u32) -> &mut Self {
        self.clear_bits(&self.regs.apben1, bits);
        self
    }

    /// 原始 APB2 时钟使能位（手册 §4.7.13）
    pub fn enable_apb2_bits(&mut self, bits: u32) -> &mut Self {
        self.set_bits(&self.regs.apben2, bits);
        self
    }

    /// 原始 APB2 时钟禁用位
    pub fn disable_apb2_bits(&mut self, bits: u32) -> &mut Self {
        self.clear_bits(&self.regs.apben2, bits);
        self
    }

    /// 原始 AHB 时钟使能位（手册 §4.7.11）
    pub fn enable_ahb_bits(&mut self, bits: u32) -> &mut Self {
        self.set_bits(&self.regs.ahben, bits);
        self
    }

    /// 原始 AHB 时钟禁用位
    pub fn disable_ahb_bits(&mut self, bits: u32) -> &mut Self {
        self.clear_bits(&self.regs.ahben, bits);
        self
    }

    /// SYSCTRL 原始寄存器访问（逃生舱）
    pub fn sysctrl(&mut self) -> &'static pac::Sysctrl {
        self.regs
    }

    /// FLASHCTRL 原始寄存器访问（逃生舱）
    pub fn flashctrl(&mut self) -> &'static pac::Flashctrl {
        self.flash
    }

    /// FLASH 模块配置时钟使能（AHBEN.FLASH）。
    ///
    /// 手册 §7.4 要求：配置 FLASH 读等待周期前必须先使能该时钟。
    pub fn enable_flash_clock(&mut self) -> &mut Self {
        self.set_bits(&self.regs.ahben, sc::AHBEN_FLASH);
        self
    }

    /// FLASH 模块配置时钟禁用
    pub fn disable_flash_clock(&mut self) -> &mut Self {
        self.clear_bits(&self.regs.ahben, sc::AHBEN_FLASH);
        self
    }

    /// 从 FLASH 工厂校准区加载 HSIOSC 96MHz TRIM 校准值（手册 §4.4.2）。
    ///
    /// - 校准值地址 0x0010_07C0（2 字节）；芯片出厂已针对每颗芯片实测；
    /// - 值无效（0x0000 / 0xFFFF）或超出 11bit 范围时跳过并返回 `None`；
    /// - 已加载则返回校准值。
    ///
    /// 未加载时 HSIOSC 频率误差可达 ±5%（90~100MHz），直接影响高波特率
    /// UART 与延时精度。[`Rcc::new`] 已自动调用一次；通常无需手动调用。
    pub fn load_factory_hsi_trim(&mut self) -> Option<u16> {
        const FACTORY_TRIM_ADDR: u32 = 0x0010_07C0;
        let raw = unsafe { core::ptr::read_volatile(FACTORY_TRIM_ADDR as *const u16) };
        if raw == 0 || raw == 0xFFFF || raw > 0x7FF {
            return None;
        }
        self.regs
            .hsi
            .modify(|v| (v & !sc::HSI_TRIM_MASK) | (raw as u32 & sc::HSI_TRIM_MASK));
        Some(raw)
    }

    /// 按频率计算 FLASH 读等待周期编码（表 7-1）：
    /// 0=≤24MHz、1=≤48MHz、2=≤72MHz、3=≤96MHz
    fn flash_wait_code(hclk_hz: u32) -> u32 {
        if hclk_hz <= 24_000_000 {
            0
        } else if hclk_hz <= 48_000_000 {
            1
        } else if hclk_hz <= 72_000_000 {
            2
        } else {
            3
        }
    }

    /// 配置 FLASH 读等待周期（手册 §7.4 表 7-1）。
    ///
    /// - 同步写入 `FLASH_CR2.WAIT` 与 `SYSCTRL_CR2.FLASHWAIT`（二者功能相同，
    ///   手册 §4.7.3 / §7.10.2 注记；双写保证生效）；
    /// - CACHE / FETCH 随 WAIT 确定性联动：`WAIT≠0` 时置位（手册建议，
    ///   补偿取指性能损失），`WAIT=0` 时清零（回到复位等效配置）；
    /// - 自动先使能 FLASH 配置时钟（AHBEN.FLASH，手册 §7.4 要求）。
    pub fn set_flash_wait(&mut self, wait: u32) -> &mut Self {
        debug_assert!(wait <= 3, "WAIT 编码仅 0..=3（表 7-1）");
        self.enable_flash_clock();
        // CACHE/FETCH 确定性重写（仅保留 CACHEINVALID 等其余位）
        let keep = self.flash.cr2.read()
            & !(crate::pac::flash::CR2_WAIT_MASK
                | crate::pac::flash::CR2_CACHE
                | crate::pac::flash::CR2_FETCH);
        let cache_fetch = if wait != 0 {
            crate::pac::flash::CR2_CACHE | crate::pac::flash::CR2_FETCH
        } else {
            0
        };
        self.flash
            .cr2
            .write(crate::pac::flash::KEY | keep | cache_fetch | wait);
        // SYSCTRL_CR2.FLASHWAIT 镜像（KEY 保护）
        self.regs
            .cr2
            .modify(|v| (v & !sc::CR2_FLASHWAIT_MASK) | (wait << sc::CR2_FLASHWAIT_POS) | sc::KEY);
        self
    }

    /// 按目标 HCLK 频率自动配置 FLASH 读等待周期（内部使用）
    fn set_flash_latency_for_hclk(&mut self, hclk_hz: u32) {
        let wait = Self::flash_wait_code(hclk_hz);
        // 与当前编码相同则跳过（避免多余 KEY 写入）
        let current = (self.regs.cr2.read() & sc::CR2_FLASHWAIT_MASK) >> sc::CR2_FLASHWAIT_POS;
        if current != wait {
            self.set_flash_wait(wait);
        }
    }

    // ---- 内部：KEY 保护的位置位/清零 ----

    /// 置位（KEY 保护寄存器；KEY 位读为 0，低 16 位为状态）
    fn set_bits(&self, reg: &pac::Rw<u32>, bits: u32) {
        let v = reg.read() & 0xFFFF;
        reg.write(sc::KEY | v | bits);
    }

    /// 清零（KEY 保护寄存器）
    fn clear_bits(&self, reg: &pac::Rw<u32>, bits: u32) {
        let v = reg.read() & 0xFFFF;
        reg.write(sc::KEY | (v & !bits));
    }
}
#[cfg(test)]
mod tests {
    use super::{HsiDiv, Rcc};

    #[test]
    fn hsi_div_lookup_roundtrip() {
        // 手册 §4.7.4 全 16 个编码往返：code <-> 变体 <-> divisor
        let table: [(u32, HsiDiv, u32); 16] = [
            (0, HsiDiv::Div32, 32),
            (1, HsiDiv::Div1, 1),
            (2, HsiDiv::Div2, 2),
            (3, HsiDiv::Div3, 3),
            (4, HsiDiv::Div4, 4),
            (5, HsiDiv::Div5, 5),
            (6, HsiDiv::Div6, 6),
            (7, HsiDiv::Div7, 7),
            (8, HsiDiv::Div8, 8),
            (9, HsiDiv::Div9, 9),
            (10, HsiDiv::Div10, 10),
            (11, HsiDiv::Div12, 12),
            (12, HsiDiv::Div16, 16),
            (13, HsiDiv::Div20, 20),
            (14, HsiDiv::Div24, 24), // 复位默认
            (15, HsiDiv::Div28, 28),
        ];
        for (code, div, d) in table {
            assert_eq!(div.code(), code, "code of {div:?}");
            assert_eq!(div.divisor(), d, "divisor of {div:?}");
            assert_eq!(HsiDiv::from_code(code), Some(div), "from_code {code}");
        }
    }

    #[test]
    fn flash_wait_tiers() {
        // 表 7-1：≤24M→0，≤48M→1，≤72M→2，≤96M→3，0Hz→0
        assert_eq!(Rcc::flash_wait_code(0), 0);
        assert_eq!(Rcc::flash_wait_code(24_000_000), 0);
        assert_eq!(Rcc::flash_wait_code(24_000_001), 1);
        assert_eq!(Rcc::flash_wait_code(48_000_000), 1);
        assert_eq!(Rcc::flash_wait_code(48_000_001), 2);
        assert_eq!(Rcc::flash_wait_code(72_000_000), 2);
        assert_eq!(Rcc::flash_wait_code(72_000_001), 3);
        assert_eq!(Rcc::flash_wait_code(96_000_000), 3);
    }
}
