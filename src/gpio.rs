//! GPIO（§9）
//!
//! 方向约定（手册 §9.6.1，注意与 STM32 相反）：
//! `DIR=0` 输出，`DIR=1` 输入。
//!
//! 引脚可用性（按 DIR/ANALOG 复位值推断，实际以封装为准）：
//! - GPIOA：PA0..PA8
//! - GPIOB：PB0..PB7
//! - GPIOC：PC13..PC15
//! - GPIOF：PF0、PF1、PF3、PF6、PF7
//!
//! 下拉电阻（UM V1.4 §9.6.3）：PDR 寄存器存在（@0x0C），但**硬件上仅 PF3
//! 具备实际下拉电阻**，且 PF3 复位默认使能下拉（PDR 复位值 0x0000_0008）。
//! `Pull::Down` 仅在 PF3 上配置真实下拉，其余引脚按浮空处理。
//!
//! 常用复用（已逐行对照手册表 9-2，更多组合见手册）：
//!
//! | UART  | 组合 A                       | 组合 B                       |
//! |-------|------------------------------|------------------------------|
//! | UART1 | PA02=TXD / PA03=RXD（AF1）   | PA09=TXD / PA10=RXD（AF1）   |
//! |       | PB03=TXD / PB00=RXD（AF1）   |                              |
//! | UART2 | PA02=TXD / PA03=RXD（AF2）   | PA07=TXD / PA06=RXD（AF1）   |
//! |       | PB00=TXD / PB01=RXD（AF2）   |                              |
//! | UART3 | PA04=TXD / PA05=RXD（AF1）   | PA01=TXD / PA00=RXD（AF2）   |
//! |       | PA08=TXD（AF1）/ PA09=RXD（AF2） | PA13=TXD / PA14=RXD（AF2）|
//! |       | PB02=TXD / PB05=RXD（AF1）   |                              |
//!
//! 注：UART1/UART2 可共用 PA02/PA03 物理引脚（AF1/AF2 二选一）；
//! 同一物理引脚同一时刻只能归属一个 UART。

use crate::pac;
use crate::rcc::Rcc;

/// GPIO 端口号
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortId {
    /// 端口 A
    A,
    /// 端口 B
    B,
    /// 端口 C
    C,
    /// 端口 F
    F,
}

impl PortId {
    /// 端口寄存器块基地址
    pub const fn base(self) -> usize {
        match self {
            Self::A => pac::GPIOA_BASE,
            Self::B => pac::GPIOB_BASE,
            Self::C => pac::GPIOC_BASE,
            Self::F => pac::GPIOF_BASE,
        }
    }
}

/// 输出电平
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Level {
    /// 低电平
    Low,
    /// 高电平
    High,
}

/// 输入上下拉配置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pull {
    /// 浮空
    Floating,
    /// 上拉
    Up,
    /// 下拉（UM V1.4 §9.6.3：PDR 存在但**硬件上仅 PF3 有实际下拉电阻**；
    /// 其余引脚按浮空处理，需要确定低电平请外接下拉电阻）
    Down,
}

/// 复用功能编号（手册表 9-1：每脚 4bit，0000=GPIO，0001=AF1 … 1111=AF15）
///
/// 注：表 9-2 目前仅定义 AF0..AF9 的外设功能，AF10..15 编码保留。
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Af {
    Af1 = 1,
    Af2 = 2,
    Af3 = 3,
    Af4 = 4,
    Af5 = 5,
    Af6 = 6,
    Af7 = 7,
    Af8 = 8,
    Af9 = 9,
    Af10 = 10,
    Af11 = 11,
    Af12 = 12,
    Af13 = 13,
    Af14 = 14,
    Af15 = 15,
}

/// 尚未配置方向的引脚。
///
/// 调用方需保证同一引脚只创建一个 [`Pin`]（本 HAL 不做引脚级独占检查）。
pub struct Pin<'p> {
    regs: &'p pac::Gpio,
    port: PortId,
    pin: u8,
}

impl<'p> Pin<'p> {
    /// 创建引脚并自动使能所在端口时钟。`pin` 取 0..=15。
    pub fn new(regs: &'p pac::Gpio, port: PortId, pin: u8, rcc: &mut Rcc) -> Self {
        assert!(pin < 16, "引脚号必须为 0..=15（Got {pin}）");
        rcc.enable_gpio(port);
        Pin { regs, port, pin }
    }

    /// 端口号
    pub const fn port(&self) -> PortId {
        self.port
    }

    /// 引脚号
    pub const fn pin(&self) -> u8 {
        self.pin
    }

    /// 该引脚的位掩码
    #[inline]
    fn mask(&self) -> u32 {
        1u32 << self.pin
    }

    /// 复用寄存器选择：`(是否 AFRH, 位偏移)`
    #[inline]
    fn af_pos(&self) -> (bool, u32) {
        if self.pin < 8 {
            (false, 4 * self.pin as u32)
        } else {
            (true, 4 * (self.pin as u32 - 8))
        }
    }

    /// 写入复用功能编码
    fn set_af(&self, af: u8) {
        let (high, pos) = self.af_pos();
        if high {
            self.regs
                .afrh
                .modify(|v| (v & !(0xF << pos)) | ((af as u32) << pos));
        } else {
            self.regs
                .afrl
                .modify(|v| (v & !(0xF << pos)) | ((af as u32) << pos));
        }
    }

    /// 清除开漏 / 上拉 / 下拉 / 模拟模式，复用功能清为 GPIO
    ///
    /// 下拉说明（UM V1.4 §9.6.3）：PDR 复位值 0x0000_0008，即 **PF3 复位
    /// 默认使能下拉**——本函数统一清掉，避免遗留意外下拉；需要时经
    /// [`Pull::Down`]（仅 PF3 生效）重新配置。
    fn clear_extras(&self) {
        let m = self.mask();
        self.regs.opendrain.modify(|v| v & !m);
        self.regs.pur.modify(|v| v & !m);
        self.regs.pdr.modify(|v| v & !m);
        self.regs.analog.modify(|v| v & !m);
        self.set_af(0);
    }

    /// 配置为**模拟引脚**（`GPIOx_ANALOG.PINy=1`）
    ///
    /// ADC 输入、VC 比较器输入、OPA 输入/输出、DAC 输出都要求引脚处于模拟
    /// 模式（手册 §27.3.2 / §29.3.2 明确要求 `GPIOx_ANALOG.PINy=1`）。
    /// [`Pin::new`] 会清掉 ANALOG 位，因此模拟用途必须显式调用本方法——
    /// 否则数字输出驱动器会把模拟节点拉死（实测：PB00 未配模拟时 OPA
    /// 输出被 DIR=0/ODR=0 的数字驱动压到 0V，比较器恒输出低）。
    ///
    /// 同时关闭上/下拉并把方向置为输入，避免附加负载。
    pub fn into_analog(self) -> Pin<'p> {
        let m = self.mask();
        self.regs.pur.modify(|v| v & !m);
        self.regs.pdr.modify(|v| v & !m);
        self.regs.dir.modify(|v| v | m); // 1 = 输入，关断输出驱动
        self.regs.analog.modify(|v| v | m); // 1 = 模拟
        self
    }

    /// 配置为推挽输出。先写电平再切方向，避免切换瞬间毛刺。
    pub fn into_output(self, initial: Level) -> Output<'p> {
        let m = self.mask();
        match initial {
            Level::High => self.regs.bsrr.write(m),
            Level::Low => self.regs.bsrr.write(m << 16),
        }
        self.clear_extras();
        self.regs.dir.modify(|v| v & !m); // 0 = 输出
        Output { pin: self }
    }

    /// 配置为开漏输出（ODCR 置 1）。
    ///
    /// 典型用途：I²C 总线、多设备线与、电平转换。高电平由外部上拉
    /// 电阻提供，`set_high()` 只是释放引脚（关断输出驱动）。
    pub fn into_output_open_drain(self, initial: Level) -> Output<'p> {
        let m = self.mask();
        match initial {
            Level::High => self.regs.bsrr.write(m),
            Level::Low => self.regs.bsrr.write(m << 16),
        }
        self.clear_extras();
        self.regs.opendrain.modify(|v| v | m); // 1 = 开漏
        self.regs.dir.modify(|v| v & !m); // 0 = 输出
        Output { pin: self }
    }

    /// 配置为输入（可选上拉 / 下拉）
    ///
    /// 上拉：任意引脚可用（PUR）。
    /// 下拉：**仅 PF3 具备实际下拉电阻**（UM V1.4 §9.6.3）；其余引脚
    /// 按浮空处理——需要确定低电平请外接下拉电阻。
    pub fn into_input(self, pull: Pull) -> Input<'p> {
        let m = self.mask();
        self.clear_extras();
        match pull {
            Pull::Floating => {}
            Pull::Up => self.regs.pur.modify(|v| v | m),
            Pull::Down => {
                if self.supports_pull_down() {
                    self.regs.pdr.modify(|v| v | m);
                }
            }
        }
        self.regs.dir.modify(|v| v | m); // 1 = 输入
        Input { pin: self }
    }

    /// 该引脚是否具备实际下拉电阻（仅 PF3，UM V1.4 §9.6.3）
    pub const fn supports_pull_down(&self) -> bool {
        matches!(self.port, PortId::F) && self.pin == 3
    }

    /// 配置为复用功能输出（外设驱动引脚，如 UART TXD）
    pub fn into_af_output(self, af: Af) -> AfPin<'p> {
        let m = self.mask();
        self.clear_extras();
        self.set_af(af as u8);
        self.regs.dir.modify(|v| v & !m); // 0 = 输出
        AfPin { pin: self }
    }

    /// 配置为复用功能输入（如 UART RXD）
    ///
    /// 按手册表 21-1 推荐启用内部上拉（数字、上拉输入、复用），
    /// 避免对端未接或总线空闲时引脚悬空引入噪声帧。
    pub fn into_af_input(self, af: Af) -> AfPin<'p> {
        let m = self.mask();
        self.clear_extras();
        self.set_af(af as u8);
        self.regs.pur.modify(|v| v | m); // 上拉（表 21-1 推荐）
        self.regs.dir.modify(|v| v | m); // 1 = 输入
        AfPin { pin: self }
    }
}

/// 数字输出引脚
pub struct Output<'p> {
    pin: Pin<'p>,
}

impl<'p> Output<'p> {
    /// 输出高电平（BSRR 低 16 位置位，原子操作）
    #[inline]
    pub fn set_high(&mut self) {
        self.pin.regs.bsrr.write(self.pin.mask());
    }

    /// 输出低电平（BSRR 高 16 位清零，原子操作）
    #[inline]
    pub fn set_low(&mut self) {
        self.pin.regs.bsrr.write(self.pin.mask() << 16);
    }

    /// 设置输出电平
    #[inline]
    pub fn set_level(&mut self, level: Level) {
        match level {
            Level::High => self.set_high(),
            Level::Low => self.set_low(),
        }
    }

    /// 翻转输出（TOG 寄存器，原子操作）
    #[inline]
    pub fn toggle(&mut self) {
        self.pin.regs.tog.write(self.pin.mask());
    }

    /// 读回 ODR 设定值（true = 高）
    #[inline]
    pub fn is_set_high(&self) -> bool {
        self.pin.regs.odr.read() & self.pin.mask() != 0
    }

    /// 读回 ODR 设定值（true = 低）
    #[inline]
    pub fn is_set_low(&self) -> bool {
        !self.is_set_high()
    }

    /// 读引脚实际电平（IDR，true = 高）。
    ///
    /// 开漏模式下读 IDR 而非 ODR，即可检测总线实际状态
    /// （如 I²C 仲裁、从设备应答位）。
    #[inline]
    pub fn pin_is_high(&self) -> bool {
        self.pin.regs.idr.read() & self.pin.mask() != 0
    }

    /// 切换为开漏输出（ODCR 置 1）
    #[inline]
    pub fn enable_open_drain(&mut self) {
        self.pin.regs.opendrain.modify(|v| v | self.pin.mask());
    }

    /// 切换回推挽输出（ODCR 清 0）
    #[inline]
    pub fn disable_open_drain(&mut self) {
        self.pin.regs.opendrain.modify(|v| v & !self.pin.mask());
    }

    /// 回退为未定向引脚
    pub fn into_pin(self) -> Pin<'p> {
        self.pin
    }
}

/// 数字输入引脚
pub struct Input<'p> {
    pin: Pin<'p>,
}

impl<'p> Input<'p> {
    /// 读输入电平（true = 高）
    #[inline]
    pub fn is_high(&self) -> bool {
        self.pin.regs.idr.read() & self.pin.mask() != 0
    }

    /// 读输入电平（true = 低）
    #[inline]
    pub fn is_low(&self) -> bool {
        !self.is_high()
    }

    /// 使能外部中断（选择触发沿）
    ///
    /// 注意：这只打开 GPIO 层的沿检测；NVIC 层还需
    /// `runtime::Interrupt::GpioA/B/C/F.enable()`（按端口），
    /// ISR 内需 [`Input::clear_interrupt`] 清标志。
    pub fn enable_interrupt(&mut self, edge: Edge) {
        let m = self.pin.mask();
        match edge {
            Edge::Rising => self.pin.regs.riseie.modify(|v| v | m),
            Edge::Falling => self.pin.regs.fallie.modify(|v| v | m),
            Edge::Both => {
                self.pin.regs.riseie.modify(|v| v | m);
                self.pin.regs.fallie.modify(|v| v | m);
            }
        }
    }

    /// 禁用本脚外部中断（清除沿使能，不动其它引脚）
    pub fn disable_interrupt(&mut self) {
        let m = !self.pin.mask();
        self.pin.regs.riseie.modify(|v| v & m);
        self.pin.regs.fallie.modify(|v| v & m);
    }

    /// 中断标志是否挂起（沿已触发待处理）
    #[inline]
    pub fn interrupt_pending(&self) -> bool {
        self.pin.regs.isr.read() & self.pin.mask() != 0
    }

    /// 清除本脚中断标志（R1W0：写 0 清除）
    #[inline]
    pub fn clear_interrupt(&mut self) {
        self.pin.regs.icr.write(!(1u32 << self.pin.pin));
    }

    /// 使能中断数字滤波（抗毛刺）：滤除宽度小于一个滤波时钟周期的脉冲
    ///
    /// `clk` 为端口级滤波时钟源（本端口所有已使能滤波的引脚共用，
    /// FILTER[18:16]）；本脚使能位置 FILTER[PIN]。DeepSleep 下需选
    /// LSI/RC10K/LPTIM 源（手册 §9.6.12 注）。
    pub fn enable_interrupt_filter(&mut self, clk: FilterClock) {
        // 端口级时钟选择 + 本脚使能位
        self.pin
            .regs
            .filter
            .modify(|v| (v & !(0x7 << 16)) | ((clk as u32) << 16) | self.pin.mask());
    }

    /// 禁用本脚中断滤波
    pub fn disable_interrupt_filter(&mut self) {
        self.pin.regs.filter.modify(|v| v & !self.pin.mask());
    }

    /// 回退为未定向引脚
    pub fn into_pin(self) -> Pin<'p> {
        self.pin
    }
}

/// GPIO 外部中断触发沿（§9.6.8/9.6.9）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// 上升沿
    Rising,
    /// 下降沿
    Falling,
    /// 双沿
    Both,
}

/// 中断数字滤波时钟源（§9.6.12 FILTER[18:16]，每 8 脚共享一个选择）
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FilterClock {
    HclkDiv2 = 0,
    HclkDiv4 = 1,
    HclkDiv8 = 2,
    Btim1Ovf = 3,
    Lsi = 5,
    Rc10k = 6,
    LptimOvf = 7,
}

/// 已配置为复用功能的引脚（所有权令牌，通常交给 UART 等外设持有）
pub struct AfPin<'p> {
    pin: Pin<'p>,
}

impl<'p> AfPin<'p> {
    /// 端口号
    pub const fn port(&self) -> PortId {
        self.pin.port
    }

    /// 引脚号
    pub const fn pin(&self) -> u8 {
        self.pin.pin
    }

    /// 回退为未定向引脚
    pub fn into_pin(self) -> Pin<'p> {
        self.pin
    }
}
