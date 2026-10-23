# Changelog

## 0.8.0（2026-10-23）—— RTC 实时时钟 / FLASH 自编程

- **RTC（`rtc`）**：BCD 日历（年月日周 + 时分秒，自动闰年、24 小时制）、
  时钟源 LSE/LSI/HSE/HSI 自动选择与等待、`DateTime` 合法性校验、
  双读稳定的 `datetime()`、两个闹钟（`Alarm` 支持时/分/秒独立屏蔽，
  按表 13-4 编码）、周期中断 0.5s~1 月、自动唤醒定时器、误差补偿寄存器、
  `KEY` 解锁/上锁自动配对、运行中写日期时间前等 `CR1.WAIT`。
- **FLASH 自编程（`flash`）**：页擦除（512B）+ 32 位字编程 + 擦除后整页写
  `write_page`、`PAGELOCK` 按 8 页分组解锁/回锁、`ICR` R1W0 错误清除、
  5 种 `ISR` 错误解码（PC 页 / 页锁 / 未擦除 / CACHEON / SDK 区）、
  用链接期 `_etext` 把代码区挡在擦写范围外。
- **rcc**：`enable_lse()`（等 LSE.STABLE，约 2s 超时）、`enable_rtc()`、
  `enable_flash_clock()` 已存在。
- **验证例程**：
  - `rtc_demo`：设 2026-10-23 23:59:50 → 12s 后读到 **2026-10-24 00:00:01、
    星期六**（BCD 进位 + 日期翻转 + 星期递增全对）；闹钟 A（当前秒 +5）
    **命中 1 次，命中时刻 00:00:06**；LSE 未起振自动退回 LSI（CLK=2）。
  - `flash_store`：页 127 存 `[magic, count, ~count]`，**连续两次复位后
    count 从 1→2→3**（掉电保持成立），擦除后回读 0xFFFFFFFF、编程回读
    `FLHC` + count + 反码全部一致。

### 实机踩坑（两个都会「静默失败」）

1. **必须使能 `AHBEN.FLASH`（bit1）**。`Rcc::new` 不会开它，缺这个时钟时
   `CR1/CR2/PAGELOCK` 的写入被硬件丢弃：擦除和编程都变成空操作，
   `ISR` 还不报错。`Flash::new()` 现在自动使能。
2. **MODE 必须「或」进现有 `CR1`**（`KEY | (CR1 | MODE)`），不能写裸的
   `KEY | MODE`。厂商 `FLASH_ErasePage` 也是 `CR1 | 0x02`；写裸值会导致
   编程静默失效（用 `flash_probe` 逐步骤对比定位）。
3. RTC 停表状态下 `CR1.WAIT` 不会清零，**停表后可直接写** DATE/TIME；
   照「运行中」流程去等 WAIT 会误报超时（`set_datetime` 已按运行状态分支）。

## 0.7.0（2026-10-23）—— DMA 控制器（4 通道）

- **DMA（`dma`）**：4 条独立通道（通道号即优先级，§8.5.4），8/16/32 位位宽，
  软件触发与**全部 40 种外设硬件触发**（`pac::dma::hs` 按表 8-8 建表：
  UART/SPI/ADC/DAC/TIM/I2C/CORDIC），BLOCK / BULK 两种传输模式，
  源/目的地址自增与完成重载，自动重传（`RESTART`，环形缓冲）。
  - 高层：`Dma::mem_to_mem(&src, &mut dst)` 一行完成存储器搬运（自动选位宽、
    通道 3，把 1/2 留给实时外设）
  - 外设流：`Config::periph_to_mem / mem_to_periph / mem_to_mem` + 官方步骤
    1~14 逐条对应；`Channel::configure/start/wait`、`remaining()`（读 CCNT）、
    `status()`（表 8-2 状态机解码）、`is_done/is_error`
  - `DmaError` 覆盖 5 种 `STATUS` 错误码 + 超时/宽度/长度/通道号
- **pac**：新增 `Dma` / `DmaChannel` 寄存器块（步进 0x20，总大小 0xA0）
  + 编译期布局断言；`dma` 位常量模块与 `dma::hs` 触发源全表；
  ADC 补 `IER_EOC/EOS/DMAEOC/DMAEOS`。
- **rcc**：新增 `enable_dma()` / `disable_dma()`（AHBEN[0]，带 0x5A5A 键）。
- **验证例程**：`dma_demo`（两相位，零接线）。
  - P1 存储器→存储器：64×u32 逐字比对，**64/64 一致，P1_BAD=0xFFFF_FFFF**
  - P2 ADC→DMA：DAC2 = 2048 → ADC1_IN12 连续转换，EOC 硬件触发搬 256 个
    16 位结果，**CPU 全程不参与**；MIN=2045 / MAX=2050 / MEAN=2047
    （DAC 码 2048，误差 ≤3 LSB）、REMAIN=0、STATUS=5（传输完成）
- 单测 70 项全绿（新增 4 项：位宽编码、CSR 位域、CNT/TRIG 编码、REPEAT 钳位）。

## 0.6.0（2026-10-23）—— LPTIM 低功耗定时器 / HALLTIM 霍尔定时器 / VC 比较器 / OPA 运放

- **LPTIM 低功耗定时器（`lptim`）**：PCLK/LSE/LSI 三种计数时钟、DIV1..DIV128、
  ARRM/CMPM 中断（IRQ19，**深度休眠可唤醒**，表 3-3）；`CNTSTART`/`SNGSTART`
  连续/单次启动、ARST 异步复位、双读一致的 `count()`。
  **踩坑修复**：LSI 复位默认关闭（§4.5.5），`Lptim::new(Lsi)` 现在自动
  调用 `Rcc::enable_lsi()`（加载出厂 TRIM @0x0010_07C2 + 轮询 STABLE），
  否则计数器完全不动作；`clk_ready()` 暴露稳定状态。
- **HALLTIM 霍尔接口定时器（`halltim`）**：24bit 递增计数、PCLK/2/4/8 分频、
  三路捕获（硬件把 CNT 存入 WIDTH 并清零）、CCR PWM 输出、两级滤波、
  TRGO 触发源（含三通道异或）、`SOFTCAP` 软件捕获，状态寄存器 CHxS/CHxF。
- **VC1–VC4 模拟比较器（`vc`）**：正端 4 路 / 负端 2 路 + 内部 DAC/分压参考、
  20mV 迟滞、极性反相、窗口比较、16 档数字滤波（PCLK/LSI）、
  高/上升沿/下降沿三类中断（IRQ14/15，深度休眠可唤醒）、BLANK 窗口触发。
  分压参考 `V = VIN×(1+DIV)/8`（`ref_voltage_mv` 纯函数 + 单测）。
- **OPA1/OPA2 运放（`opa`）**：外部放大 / 内部 PGA（×2..×32）/ 跟随器三种
  模式，偏置 1..8µA 可调，输入引脚独立导通控制；电平校准与触发校准
  （`CALPERIOD` 8..262144 个 OPACLK），`is_calibrating()` 查询。
- **rcc**：新增 `enable_lsi()`（TRIM+稳定等待）、`enable_halltim/enable_vc/enable_opa`。
- **验证例程**：`lptim_deepsleep`（LSI 自校准 + Sleep/DeepSleep 唤醒）、
  `halltim_capture`（PWM + 软件捕获自检）、`analog_loop`
  （DAC→OPA 跟随器→PB00→VC1 零接线模拟回环，二分找翻转点）。
### 实机验证中发现并修复的三个真 bug

1. **OPA 时钟门控位写错**：原按 `APBEN1[9]` 使能，实为 **`APBEN2[9]`**（§4.7.13）。
   症状：`OPA1_CR` 读回始终为复位值 `0xE000`，所有 OPA 配置静默失效。
2. **缺 `Pin::into_analog()`**：模拟外设要求 `GPIOx_ANALOG.PINy=1`，而
   `Pin::new` 会**清**该位；PB00 未配模拟时被复位默认的数字输出驱动器
   （DIR=0/ODR=0）压在 0V，OPA 输出被短路，比较器恒低。新增 API 后
   `analog_loop` 立刻通过。
3. **`delay_us` 严重偏长**：每次调用都做 `u64` 除法（M0+ 无硬件除法，
   走软件例程）且 SysTick 启停固定开销大——实测 4MHz、未优化构建下
   `delay_us(100)` 实际 **410µs（+310%）**、`delay_us(1000)` 1328µs。
   修复：① `Delay` 预计算每 µs/每 ms 节拍数，常规频率零除法；
   ② 短延时（< 20000 周期）改用 **3 周期/轮的周期忙等**（`subs`+`bne`），
   避开 SysTick 启停开销；③ dev profile 开 `opt-level = "s"`（-O0 下
   函数调用开销可达数百周期）。修复后：100µs→95~101µs、1000µs→1011µs、
   10000µs→10052µs（±5% 内）。

此 bug 还解释了排查中"LSI 实测 36864Hz（+12%）"的假象——5 秒校准窗口
因延时偏慢实际走了 5.8 秒；修复后 **LSI = 32768Hz（标称 32800，误差 0.1%）**。

### 实机验证（2026-10-23，CW32L012 + ST-LINK）

- `lptim_deepsleep`：LSI 自校准 32768Hz、Sleep 唤醒 ×3、DeepSleep 唤醒 ×15（持续计数）
- `halltim_capture`：2ms 计数 4097、二次捕获 WIDTH=4110（捕获延迟 13 计数）、
  捕获后 CNT=20、ARR=19999/CCR=6000 精确
- `analog_loop`：DAC→OPA 跟随器→PB00→VC1 翻转点 **2035**（理论 2048，误差 0.6%），
  比较器低/高端输出 0/1 正确

- 66 项主机侧单元测试全绿；交叉编译零警告。

## 0.5.0（2026-10-19）—— ATIM 高级定时器 / WWDT 窗口看门狗 / 低功耗模式

- **ATIM 高级定时器（`atim`）**：互补 PWM（CHy+CHyN）+ **死区**（CW32 自有
  DTG 编码：0xx×1 / 10x×2 / 110×8 / 111×16 t_DTS，`encode_dtg` 向上取整
  保证实际死区永不短于请求值，单测含 96MHz 全程回代）+ **刹车**（BRK
  使能/极性/滤波，`re_enable_output` 恢复）；万分比占空比 API 与 GTIM
  同风格。**实机验证**：2kHz/30%/250µs 死区，BDTR 回读 0x94FF、CNT 铺
  满；踩坑记录：刹车输入必须 AF 复用输入（悬空/普通输入会锁存刹车清
  MOE，见 TESTING.md）。
- **WWDT 窗口看门狗（`wwdt`）**：窗口内喂狗、过早/过晚都复位的三条件
  语义；`WindowConfig::for_feed_interval` 按 PCLK 自动求解 PRS/WCNT/WINR
  （T_max≥请求、T_min≈请求/2，ns 级 u64 全程防精度丢失，手册 §20.3.4
  官方例值回归）；`feed()` 先读计数值、窗口外拒绝（返回 TooEarly 而非
  复位芯片）。**实机验证**：EARLY=51/FEEDS=50 同步递增、50 轮零复位。
- **低功耗（`pwr`）**：`sleep()`（WFI+SLEEPDEEP=0）/ `deep_sleep(fast)`
  （FLASH 忙检测 + SYSCTRL_CR2.WAKEUPCLK 快速唤醒时钟 + 唤醒后自动清
  SLEEPDEEP）/ `wait_for_event()`（WFE）。**实机验证**：Sleep×10
  （BTIM1 唤醒）+ DeepSleep×6（IWDT 中断唤醒），WAKEUPCLK 后
  SYSCLK=HSI ✓。
- **IWDT 中断模式**：`Iwdt::start_interrupt`（溢出走中断不复位、
  DeepSleep 不暂停）——低功耗唤醒源。
- **ADC 温度传感器失效保护**：`AdcError::SensorDead`——TS 原始值
  <100 counts（≈<0.08V，正常 ~0.78V）时拒绝计算，杜绝"-271℃"假数据
  （本板个体异常的实机教训，诊断过程见 TESTING.md）。
- pac 新增 Atim（0x74，含 ICR@0x70 特殊排布断言）/Wwdt（0x0C）寄存器块
  与位常量；rcc 补 enable_atim/enable_wwdt。
- **实机验证全绿**：gtim_pwm、atim_deadtime、dac_adc_loop（回环误差
  3 LSB）、adc_sequence（IN12/BGR）、wdt_demo（复位循环观测）、
  wwdt_demo（窗口语义）、low_power（双阶段唤醒）——流程与判定标准
  全部沉淀到 **TESTING.md**（测试备忘录）。

## 0.4.0（2026-10-18）—— DAC / 看门狗 / GTIM PWM / ADC 序列扫描

- **DAC（`dac`）**：双通道 12-bit，无触发直写模式（写 DHR → 1 APCLK 转入
  DOR）；`set_ch1/ch2/set_both` + DOR 回读；引脚输出使能独立控制
  （PA4/PA5 与 SPI1/OPA 冲突警告内置于文档）。**内部互联回环自测**：
  DAC_OUT2 直连 ADC1_IN12，`examples/dac_adc_loop.rs` 零接线验证精度。
- **IWDT 看门狗（`wdt`）**：键值机完整封装（0xCCCC 启动 / 0xAAAA 喂狗 /
  0x5555 解锁 / 0x5A5A+0xA5A5 停止）；`start(TimeoutMs)` 自动选
  PRS/ARR（**整体向上取整，实际超时永不短于请求值**，5 项单测含边界
  0.8ms~209.6s 与抽查回代）；独立 RC10K 时钟域，主时钟崩溃仍可复位。
- **GTIM PWM（`gtim`）**：GTIM1-4 × 4 通道，边沿对齐 PWM 模式 1 +
  预装载 + ARPE；频率/占空比（万分比）API，PSC/ARR 自动配对；
  CCR/ARR/PSC/CNT 全可回读。`examples/gtim_pwm.rs` 1kHz 呼吸占空比。
- **ADC 序列扫描**：`convert_sequence` 2..=8 通道一次启动（ENS=N-1、
  每通道独立采样周期、等待 EOS），内部通道前置条件自动处理；
  `examples/adc_sequence.rs` [DAC回环, 温度, 基准] 三通道联测。
- 时钟门控补全：DAC（APBEN2[10]）、IWDT（APBEN2[4]）、GTIM1-4（APBEN1）。
- pac 新增 Dac/Iwdt/Gtim 寄存器块（布局断言 0x38/0x28/0x68）与位常量。
- 实机验证进行中（探针掉线待复插）：dac_adc_loop / wdt_demo / gtim_pwm /
  adc_sequence 四例程编译零警告，上电验证命令已备好。

## 0.3.0（2026-10-18）—— 外设矩阵扩展：SPI / I2C / BTIM / ADC + embedded-hal 1.0

新外设驱动（全部按《CW32L012 用户手册》V1.4 逐位建模，寄存器块带编译期布局断言）：

- **SPI（`spi`）**：SPI1/2/3 阻塞式主机。8-bit 帧、四种 CPOL/CPHA 模式、
  MSB/LSB、单工单发/单收（屏幕等只写器件零开销）、`SCK=PCLK/(2×(N+1))`
  波特计算（含最接近档自动选择）。实机验证：ST7789 @24MHz，整屏绘制
  从软 SPI 时代的 ~90s 缩到 <3s（复位后 3s 内 init+清屏+彩条全部完成）。
- **I2C（`i2c`）**：I2C1/2 阻塞式主机，7-bit 寻址，100k/400k 档时序按
  I2C 规范推导并与手册表 23-4 官方值对齐（±1 TCLKD）；NACK/仲裁丢失/
  总线卡死全带超时与结构化错误；`scan()` 总线扫描。21 项主机单测
  （含表 23-4 数值回归）。
- **BTIM（`btim`）**：BTIM1-3 周期/单次定时，PSC/ARR 自动配对（ARR 最大化
  量化精度），更新中断 + `runtime::Interrupt` 联动。实机验证：2Hz 中断
  翻转 PC13（`wfi` 睡眠、硬件唤醒）。
- **ADC（`adc`）**：ADC1/2 单次阻塞转换；内置温度传感器（IN14）与 1.2V
  基准（IN15）通道，内置 30µs TSEN 稳定等待、出厂校准读取与手册公式
  的整数实现（`vdda_mv()` / `temperature_mc()`）。实机验证：VDDA=3282mV、
  温度 26.9℃（手册公式示例回归测试 ±5m℃）。修复 SQRCH0 采样周期字段
  错位（bit0 而非 bit4）。
- **GPIO 外部中断 API**：`Input::enable_interrupt(Edge)` /
  `clear_interrupt` / 数字滤波（FILTER，端口级时钟源选择）。
- **时钟门控补全**：`rcc` 新增 SPI1/2/3、I2C1/2、BTIM、ADC 使能（KEY 保护写）。

生态兼容（feature `ehal-1`，默认开启，可关回零依赖）：

- **embedded-hal 1.0 trait**：`DelayNs`（含纳秒级双路径延时）、
  digital（InputPin/OutputPin/StatefulOutputPin，`toggle` 覆写为 TOG 原子
  翻转；Output 可回读 IDR）、`SpiBus` + `SpiDevice`（事务封装、CS 时序、
  `Operation::DelayNs`）、`i2c::I2c`。

## 0.2.4（2026-10-17）—— 对照 UM V1.4 的裁决修正

- `pac`：补回 `GPIOx_PDR @ 0x0C`（V1.2 无此寄存器；V1.4 §9.6.3 确认存在，
  硬件上仅 PF3 有实际下拉）。`Pull::Down` 在 PF3 生效，其余引脚文档化浮空；
  `clear_extras` 一并清 PDR（修复 PF3 复位默认下拉的隐藏行为）。
- UART 引脚表三处矛盾统一为表 9-2 完整组合（PA04/05 AF1、PA01/00 AF2 等）。
- 数据来源 V1.2 → V1.4；"未上板"过时声明更新。
- 实机验证：HAL 全路径 blink_pc13 上板（向量表 MSP=0x20002000、PC13 翻转）。

## 0.2.3 及更早

见 README 路线图与历史交付说明：PAC/时钟/GPIO/UART/SysTick/运行时全量
实现、FLASH 等待周期安全切换（v0.2）、UART 错误自愈（v0.2）。
