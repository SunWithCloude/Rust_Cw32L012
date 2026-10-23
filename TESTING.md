# 测试备忘录（TESTING.md）

> HAL 实机验证手册。探针：ST-LINK V2（0483:3748），目标：CW32L012x8。
> 本文记录每个例程的**烧录命令、观察变量、预期读数、判定标准**，
> 探针重新插上后照单执行即可，无需重新推导。

## 通用操作

### 烧录 + 复位（在 HAL 仓库根目录执行）

```bash
PROBE="--probe 0483:3748 --protocol swd --chip CW32L012x8"
CFG='--chip-description-path C:/Users/Administrator/Downloads/CW32L0_Series.yaml'
probe-rs download $PROBE $CFG target/thumbv6m-none-eabi/debug/examples/<例程名>
probe-rs reset    $PROBE $CFG
```

### 观察静态变量（结果落在 SRAM）

1. 先定位符号地址（**每次重编译都会漂移，必须重新查**）：
   ```bash
   nm target/thumbv6m-none-eabi/debug/examples/<例程名> | grep -E " [bBdD] " | awk '{print $1, $NF}'
   ```
2. 读内存（**首选 `probe-rs read`——直连探针，不依赖 IDE**）：
   ```bash
   probe-rs read b8 0x20000000 128 $PROBE $CFG   # 读 128 字节 SRAM hexdump
   ```
   插件调试服务 `http://127.0.0.1:3678/api/memory?...` 仅在 IDE 调试
   会话开启时可用（忘了开 IDE 会 connect refused），作备选。
3. ⚠ **调试器直读 APB 外设寄存器不可信**（曾出现 SAMPLE/SQRCFR/RESULT1
   三寄存器同值 0x89D 的假读）。需要外设寄存器值时，在固件里
   `read_volatile` 快照到 SRAM 静态变量再读。

---

## 实测通过（2026-10-17 ~ 10-19，本板）

| 例程 | 验证内容 | 关键读数 |
|------|----------|----------|
| blink_pc13 | GPIO KEY 门控→ANALOG 清零→BSRR/TOG | PC13 500ms 方波 |
| tft_hw_spi | 硬件 SPI1@24MHz + 96MHz 变频 + ST7789 | 复位后 <3s 全屏彩条（软 SPI 时代 ~90s） |
| timer_blink | BTIM1 2Hz 中断 + wfi | PC13 翻转序列 0,0,1,0,0,1,1,0 |
| adc_internal | ADC 单次 + 内部通道 | VDDA=3282mV、TS=26.95℃（当时） |
| gtim_pwm | GTIM1 PWM 1kHz 呼吸 | REG=0x0F9F0000（ARR=3999/PSC=0）、CNT 铺满 0..3999 |
| dac_adc_loop | DAC→ADC 片上回环（DAC_OUT2→IN12） | 15 轮扫描，MAX_ERR=3 LSB |
| atim_deadtime | ATIM 互补 PWM + 死区 + 刹车 | REG=0x07CF0000、CCR=600（30%）、BDTR=0x94FF（MOE+BKE+OSSI+DTG=0xFF）、CNT 铺满 |
| wdt_demo | IWDT 超时复位 | BOOT_ID 增至 6 → 复位回 1 循环（停喂 ~1.5s 后咬人） |
| wwdt_demo | WWDT 窗口喂狗 | WW_CFG=0x27058、EARLY=51/FEEDS=50 同步（过早喂狗全部被拦截、正常喂狗全部成功、50 轮无复位）、LAST_CNT=0x64（>WINR=0x58 故拒绝） |
| low_power | Sleep + DeepSleep + WAKEUPCLK | SLEEPS=10（BTIM1 唤醒）、DS_WAKES=6（IWDT 2s 唤醒）、WDT_FEEDS=6、WAKE_CLK=0（唤醒后 SYSCLK=HSI ✓） |
| adc_sequence | 序列扫描 + DAC 联动 | IN12 精确跟随 DAC 设定（±1 LSB）、BGR=1488（理论 1489）✓ |

## 复测步骤（每项的判定细节）

### atim_deadtime（互补 PWM/死区/刹车）

- 静态：`REG`=ARR<<16|PSC → 2kHz@4MHz 应为 0x07CF0000；`CCR`=600；
  `BDTR` 应为 0x94FF（bit15 MOE=1、bit12 BKE=1、bit10 OSSI=1、DTG=0xFF
  → 1008 t_DTS ≈ 252µs ≥ 请求 250µs）；CNT_MIN=0、CNT_MAX=1999。
- **坑（已修）**：刹车输入 PA06 必须 `into_af_input(Af::Af7)`（AF 复用
  输入 + 上拉）。普通 GPIO 输入或悬空会让 BRK 采样为低 → 刹车锁存
  → MOE 被硬件清零 → 引脚无波形（BDTR 读回 0x14FF 的教训）。
- 示波器（可选）：PA08/PA15 互补 2kHz、翻转沿间隔 ≥250µs 互不重叠；
  PA06 拉低 → 两路立即熄灭；`re_enable_output()` 恢复。

### wdt_demo（IWDT 复位循环）

- 采样 BOOT_ID @0x20000000（1.2s 间隔 ×10）：应见 递增 → 突然回 1
  （喂满 5 次后停喂，IWDT 1.5s 超时复位）→ 再递增。PC13：快闪 5 下
  + 灭 ~1.5s 循环。

### wwdt_demo（窗口看门狗）

- 静态：WW_CFG=0x0002_7058（PRS=2、WCNT=0x70、WINR=0x58 → 窗口
  [98.3, 200.7]ms）；EARLY≈FEEDS 同步递增；程序永不复位（计数远超
  200ms 超时）。LAST_CNT 记录的是**过早喂狗时刻**的计数值（50ms 处
  ≈0x64，>WINR 0x58 → 正确拒绝）。
- 对照实验（可选）：把 `feed()` 换成 `feed_forced()` → 过早喂狗真实
  写入 → 芯片复位循环（PC13 常灭）。

### low_power（休眠/深度休眠）

- 烧录复位后等待 ≥21s（阶段 1 = 10×500ms Sleep + 阶段 2 = 6×2s
  DeepSleep），读四个静态变量（见上表）。
- 电流计（可选）：DeepSleep 阶段平均电流应显著低于 Sleep 阶段
  （高速时钟关闭）。WAKE_CLK=0 证明 SYSCTRL_CR2.WAKEUPCLK 生效。

### dac_adc_loop / adc_sequence / gtim_pwm

见上表读数；adc_sequence 的 IN12 应每轮精确跟随 DAC_SET
（512,1024,…,3584 循环，±1 LSB）。

---


### lptim_deepsleep（LPTIM + LSI 自校准 + 深度休眠唤醒）— ✅ 2026-10-23 通过

```bash
cargo build --target thumbv6m-none-eabi --example lptim_deepsleep
# 烧录/复位见上文；等 22s 后读
probe-rs read b8 0x20000000 20 --probe 0483:3748 --protocol swd \
  --chip CW32L012x8 --chip-description-path "C:/Users/Administrator/Downloads/CW32L0_Series.yaml"
```

| 符号 | 偏移 | 实测 | 判定 |
|---|---|---|---|
| SLEEP_WAKES | 0x00 | **3** | =3（LPTIM ARRM 唤醒 Sleep） |
| PHASE | 0x04 | **2** | 三阶段全部跑完 |
| LSI_HZ | 0x08 | **32768** | 标称 32800，误差 0.1%（出厂 TRIM 生效） |
| DS_WAKES | 0x0C | **15** | ≥3 即三段 DeepSleep 均被唤醒；之后在收尾 wfi 里继续计数 |
| CAL_TICKS | 0x10 | **40** | 5s × 32768 / 4096 = 40 |

**踩坑 ①（已修）**：LSI 复位默认关闭（§4.5.5），首版 `Lptim::new(Lsi)`
未使能 → `CAL_TICKS=0` 且卡在首个 `wfi`。现由 `Rcc::enable_lsi()`
（读 TRIM@0x0010_07C2 → `CR1.LSIEN` → 轮询 `LSI.STABLE[15]`）自动处理。

**踩坑 ②（已修）**：早期读到 `LSI_HZ≈36864`（+12%）并非振荡器不准，
而是 `delay_ms` 偏慢导致 5s 窗口实际 5.8s。延时修复后回到 32768。
→ 任何"用延时当时间基准"的测量都要先确认延时精度（见下节）。

### halltim_capture（HALLTIM 24bit 计数 + PWM + 双次软件捕获）— ✅ 2026-10-23 通过

配置：PCLK/2 = 2MHz，ARR=19999（100Hz PWM），占空比 30%（CCR=6000）。

| 符号 | 偏移 | 实测 | 判定 |
|---|---|---|---|
| W1 | 0x00 | 153 | 上一轮捕获后残留（应 <300） |
| W2 | 0x04 | **4110** | ≈CNT_MID（差 13 = 捕获延迟 ≈6µs） |
| ARR | 0x08 | **19999** | 精确 |
| CCR | 0x0C | **6000** | 精确 |
| ROUNDS | 0x10 | 递增 | 固件活着 |
| CNT_MID | 0x14 | **4097** | 2ms @2MHz ≈4000（+2.4% 延时余量） |
| CNT_AFTER | 0x18 | **20** | 捕获沿硬件清零 ≈0 |

判定要点：`W2 ≈ CNT_MID` 且 `CNT_AFTER ≈ 0` —— 只有硬件捕获路径能把
"当前 CNT 存进 WIDTH 并清零计数器"，这组关系是最强自检信号。
⚠ 偏移随构建变化，跑前先 `nm <elf> | grep 符号名`。

### analog_loop（DAC → OPA 跟随器 → PB00 → VC1）— ✅ 2026-10-23 通过

链路：`DAC_OUT1` 内部 → OPA1 正端 INP4 → 跟随器输出 `OPA1_OUT=PB00` →
`VC1_CH3=PB00`（同一焊盘）→ 与内部 `VDDA×4/8` 比较。二分扫描 DAC 找翻转点。

| 符号 | 偏移 | 实测 | 判定 |
|---|---|---|---|
| CROSS | 0x00（u16） | **2035** | 理论 2048（VDDA/2），误差 0.6% |
| RUNS | 0x04（u32） | 递增 | 固件活着 |
| LOW_FLV | 0x08（u8） | **0** | DAC=0 < 1.65V |
| HIGH_FLV | 0x09（u8） | **1** | DAC=4095 > 1.65V |

**两个必踩的坑（已修）**：
1. **OPA 时钟门控在 `APBEN2[9]`**（不是 APBEN1）。写错时 `OPA1_CR` 读回
   恒为复位值 `0xE000`，配置静默失效。
2. **PB00 必须 `into_analog()`**：模拟外设要求 `GPIOx_ANALOG.PINy=1`。
   不配时（复位值 0 = 数字）DIR=0/ODR=0 的输出驱动器把节点压在 0V，
   OPA 输出被短路 → 比较器恒低、`CROSS=0xFFFF`。

### clk_check（延时/时钟精度）— ✅ 2026-10-23 通过

以 BTIM1 1kHz（PCLK 硬件分频）为基准，测 `delay_ms/delay_us`：

| 指标 | 修复前 | 修复后 |
|---|---|---|
| `delay_ms(1000)` | 1000 节拍 ✔ | 1000 节拍 ✔ |
| `1000×delay_us(1000)` | 1328µs/次（+33%） | **1011µs/次（+1.1%）** |
| `5000×delay_us(100)` | 410µs/次（+310%） | **100.8µs/次（+0.8%）** |
| `500×delay_us(10000)` | 10336µs/次 | **10052µs/次（+0.5%）** |
| PCLK 实测 | — | 4.024MHz（HSI 工厂 TRIM 后 +0.6%） |

### dma_demo（DMA 存储器 / ADC 硬件触发）— ✅ 2026-10-23 通过

零接线两相位：P1 存储器→存储器 64×u32；P2 DAC2=2048 → ADC1_IN12 连续转换，
EOC 硬件触发把 256 个 16 位结果搬进 `BUF`（CPU 全程不参与）。固件末尾自旋，
随时可读。

烧录后 2s 读三个区间：`0x20000200`（DST[64]）、`0x20000300`（统计量，28 字节）、
`0x20000000`（BUF[256]）。

| 指标 | 地址 | 实测 | 判定 |
|---|---|---|---|
| `P1_OK` | 0x300 | **64** | 应 64 |
| `P1_BAD` | 0x304 | **0xFFFF_FFFF** | 无失配 |
| `P1_ERR` | 0x308 | 0 | — |
| `P2_ERR` | 0x30A | 0 | — |
| `P2_MAX` / `P2_MIN` | 0x30C / 0x30E | **2050 / 2045** | DAC 码 2048，±3 LSB |
| `P2_OUT` | 0x310 | 0 | 无 >16 LSB 离群点 |
| `P2_MEAN` | 0x314 | **2047** | ≈2048 |
| `P2_FIRST` | 0x316 | 2048 | — |
| `P2_REMAIN` | 0x318 | **0** | 搬完 |
| `P2_STATUS` | 0x31A | **5** | 表 8-2「传输完成」 |

DST 前几字应为 `A5A5_0000, A5A4_0101, A5A7_0202, A5A6_0303…`
（`0xA5A5_0000 ^ i*0x0101_0101`），实测逐字一致。

**五个坑（都写进 `dma.rs` 文档了）**：

1. **`ICR` 是 R1W0**（复位值 0xFFFF_FFFF）：写 1 无动作、**写 0 才清**。
   清通道 1 的 TC/TE 要 `icr.write(!(1<<0 | 1<<1))`，不是写 1。
2. **`CNT.REPEAT` 每次传输后被硬件清 0**（§8.5.2），所以每轮传输前都要重新
   `configure()`，不能只改 `CNT`。
3. **软件触发 `SOFTSRC` 完成后自动清零**，再传一次必须重新写。
4. **地址自增量由位宽决定**（8/16/32bit → 1/2/4 字节）。ADC 多通道结果寄存器
   步进是 4 字节，所以要么配 32bit，要么用 `REPEAT` 整块搬。
5. **源/目的地址不能是 DMA / FLASH / RAM 控制器自己的寄存器**（§8.8.6 注）。

另：ADC 要出 DMA 请求必须置 `IER.DMAEOC`（单次）/ `IER.DMAEOS`（序列），
且 `HARDSRC` 选 `ADC1_SINGLE`（0x0D）/ `ADC1_SEQ`（0x0C）——两者别配错。

### rtc_demo（RTC 跨日走时 + 闹钟）— ✅ 2026-10-23 通过

固件先试 LSE（外部 32.768k 晶体），2s 不起振则退回 LSI。本板 **LSE 未起振
（CLK=2 → LSI）**，说明 StartKit 没焊 32.768k 晶体或负载电容不匹配。

| 指标 | 地址 | 实测 | 判定 |
|---|---|---|---|
| `CLK` | 0x04 | **2** | LSE 失败 → LSI |
| `DT_BEFORE` | 0x20 | 0x1A0A1705 | 26-10-23 周五 |
| `DT_AFTER` | 0x18 | **0x1A0A1806** | 26-10-24 **周六**（跨日+星期递增） |
| `TM_AFTER` | 0x1C | 00:00:01 | 23:59:50 起跑 12s |
| `ALARM_HITS` | 0x00 | **1** | 闹钟 A 命中 |
| `ALARM_AT` | 0x14 | **00:00:06** | 设定值 = 读回秒数 +5，命中时刻一致 |
| `ERR` | 0x08 | 0 | 无错误 |

`DT_BEFORE/AFTER` 打包格式：`year<<24 | month<<16 | day<<8 | weekday`；
`TM_AFTER`：`hour<<16 | minute<<8 | second`。

坑：**停表状态下 `CR1.WAIT` 不清零**。手册 §13.3.6 的"等 WAIT"流程只适用于
**运行中**写 DATE/TIME；停表后可直接写。照运行中流程做会一直等到超时
（首版 `set_datetime` 因此返回 Err，但时间其实已写入——`ERR=1` 而日期正确）。

另：12s 读到 00:00:01 而非 :02，差 1s。LSI 已加载出厂 TRIM（0x0010_07C2=0x51），
未单独标定长期走时精度；需要准点请接 LSE。

### flash_store（FLASH 自编程 / 掉电保持）— ✅ 2026-10-23 通过

第 127 页（`0xFE00`，512B）存 `[magic "FLHC", count, ~count]`，每次复位 +1。
`_etext` 实测 0x0EF8，页 127 距代码区很远，`erase_page` 的代码区护栏不会误拦。

| 指标 | 地址 | 首次烧录 | 复位 1 | 复位 2 |
|---|---|---|---|---|
| `OK` | 0x00 | 1 | 1 | 1 |
| `OLD_MAGIC` | 0x28 | 0x1234_5678* | 0x4348_4C46 | 0x4348_4C46 |
| `OLD_COUNT` | 0x24 | 0xFFFF_FFFF | 1 | 2 |
| `NEW_COUNT` | 0x20 | 1 | **2** | **3** |
| `ERASED` | 0x0C | 0xFFFF_FFFF | — | — |
| `RB_MAGIC` | 0x18 | 0x4348_4C46 | 同左 | 同左 |
| `RB_CHECK` | 0x10 | 0xFFFF_FFFE | 0xFFFF_FFFD | 0xFFFF_FFFC |
| `SECURITY` | 0x1C | 0 | 0 | 0 |

\* 首次那轮的 0x1234_5678 是调试探针 `flash_probe` 留下的，正好反证"读旧值"
读的是真 Flash。

**两个静默失败坑（都不报错，最难查）**：

1. **必须先使能 `AHBEN.FLASH`（bit1）**。`Rcc::new` 不开这个时钟，缺它时
   `CR1/CR2/PAGELOCK` 的写全被丢弃——擦除/编程变成空操作，`ISR` 一片干净。
   `Flash::new(regs, &mut rcc)` 现在自动开。
2. **MODE 要「或」进现有 CR1**：`KEY | (CR1_old | MODE)`。写裸的 `KEY | MODE`
   会让编程静默失效（擦除有时还"看起来"成功）。厂商 `FLASH_ErasePage` 也是
   `CR1 | 0x02`。定位方法：写 `flash_probe` 逐相位 dump CR1/CR2/PAGELOCK/ISR
   + 读回值，四相位（擦除前/擦除后/32 位写/8 位写）一比就出来了。

其他要点：`ICR` 同样是 R1W0（写 0 清）；`write_page` = 擦除 + 从页首写；
擦除触发用**字节写**（厂商写法），32 位写虽然也能触发但没必要冒险。

### ecosystem（critical-section / PWM trait / embedded-io）— ⏳ 待上机

零接线。验证 v0.9 的三条生态对接通路，结果全部落在 SRAM 起始处（32 字节）。

```bash
cargo build --target thumbv6m-none-eabi --release --example ecosystem \
  --features critical-section-impl,embedded-io
probe-rs download --probe 0483:3748 --protocol swd --chip CW32L012x8 \
  target/thumbv6m-none-eabi/release/examples/ecosystem
probe-rs reset  --probe 0483:3748 --protocol swd --chip CW32L012x8
sleep 2 && probe-rs read b8 0x20000000 32 --probe 0483:3748 --protocol swd --chip CW32L012x8
```

符号布局（`nm | grep ' [bB] '` 实测）：

| 地址 | 符号 | 类型 | 预期 |
|---|---|---|---|
| 0x20000000 | `IO_WRITTEN` | u16 | **16**（"cw32 eio ok\r\n" 长度） |
| 0x20000002 | `PWM_MAX` | u16 | **10000** |
| 0x20000004 | `CS_AFTER` | u32 | **0** |
| 0x20000008 | `CS_BEFORE` | u32 | **0** |
| 0x2000000C | `CS_INSIDE` | u32 | **1** |
| 0x20000010 | `CS_NESTED` | u32 | **1**（嵌套里仍屏蔽） |
| 0x20000014 | `PWM_25PCT` | u16 | **≈2500**（ARR=3999 → CCR=1000） |
| 0x20000016 | `PWM_CLAMP` | u16 | **10000**（`u16::MAX` 被夹紧） |
| 0x20000018 | `STAGE` | u32 | **3**（走完三步） |

判定要点：

- **`CS_BEFORE=0 / CS_INSIDE=1 / CS_AFTER=0`** 才说明 `mrs PRIMASK` + `cpsid i`
  真的生效且恢复正确。若 `CS_AFTER=1`，说明 `release` 没把中断开回来（会表现为
  `wfi` 永远睡死）；若 `CS_INSIDE=0`，说明汇编没生效、`critical-section` 只是摆设。
- **`CS_NESTED` 必须也是 1**：内层拿到旧值 1，退出时不该 `cpsie i`。这是
  “嵌套安全”的唯一直接证据。
- `PWM_25PCT` 有 ±2 的量化误差（`CCR = (ARR+1)*2500/10000 = 1000`，
  反算 `1000*10000/4000 = 2500`），不必卡死等值。
- `IO_WRITTEN` 只是 trait 返回值，PA02 上确实有波形但没接东西；
  要看真实字节请用 USB-TTL 接 PA02 收。

## 已知问题（复测时注意）

### ⚠ TS 内置温度传感器（本板个体异常，代码已证无错）

- 现象：TS 通道（0xE）转换结果 0~7 counts（≈0V），单次/序列/厂商
  例程 1:1 复刻（Div1、S166、TSEN 先于 EN、双通道）全部为 0；
  同路径 BGR（0xF）✓、IN12（0xC）✓。
- v0.3.0（2026-10-18 凌晨）同板实测 TS≈980 → 26.9℃，寄存器路径自
  当时起无变化（转录重放 diff 核对）。
- **终审（2026-10-21 `ts_probe` 判定矩阵，一烧录跑完所有变量）**：
  | 相位 | 条件 | 结果 |
  |---|---|---|
  | A | 仅 TSEN（厂商风格，100ms 后）×3 | 0,0,0 |
  | B | 补 BGREN（100ms 后）×3 | 0,0,0 |
  | C | TSEN 使能 ≥5s | 0 |
  | 曲线 | 每秒一次 ×24s（S518 最长采样） | 全 0 |
  | S6 | 最短采样（6 周期）对照 | **83** |
  | BGR | 同节奏对照 ×24 | 1545~1571 ✓ |
- S6=83 是决定性证据：前次 BGR 转换（≈1.2V）残留在采样电容的电荷，
  在 3µs 短采样里被 TS 输出放电至 67mV——**TS 输出 ≈0V 但仍呈高阻
  负载特征**。排除使能顺序、稳定期、上电预热、采样时间、ADC 故障。
- **POR 终审（2026-10-21，受控实验）**：ts_probe 固件先清
  SYSCTRL_RESETFLAG 再采数；用户真断电 10s 重插后 RF_BOOT=0x1（干净
  POR，bit0 独置、无软复位残留）。结果：**BGR 恢复理论值 1492**
  （断电前漂至 1522~1548，证明 POR 确实重置了模拟偏置），**TS 仍为 0**。
  至此软件侧手段全部穷尽：使能顺序（5 变体）、稳定期、BGR 软件重启
  （2 变体）、ADC2/0xE、ADCCLK 2MHz/500kHz、采样 S6~S518+Div8 极限、
  90s+ 预热、POR——全部无效。**终审：该芯片 TS 模拟块输出级无输出**
  （硬件个体状态；官方样品亦可能存在 QC 漏检个体，或曾受未知电应力）。
- 附带发现：**BGR_CR 复位默认 0x105——BGREN 默认已开启**（厂商例程
  无需显式使能的原因）；bit2/3/8 为硬件状态位（POR 后 0x105，软件
  重启 BGR 后变 0x3，60s 转换后 0xf——随启动路径变化，勿当错误）。
- 已采取的措施：`AdcError::SensorDead`——`temperature_mc*()` 在
  raw<100 counts 时返回错误，杜绝"-271℃"假数据。
- 复测时若 TS 恢复（>750 counts ≈ 室温）：adc_sequence 三通道应全绿；
  adc_internal 的 TEMP_MC 应回到 ~25000-30000 m℃；再跑一次 ts_probe
  看 A/B/C/曲线是否整体抬升。

- **第三轮（2026-10-23 `ts_probe3`，六条从未测过的路径，全灭）**：
  | 相位 | 新角度 | 结果 |
  |---|---|---|
  | A | `BGR_CR` 未定义位 bit2/3/8 显式置位（8 组掩码，含 POR 值 0x105） | 写回值 100% 保持，**TS 全 0** |
  | B | 连续转换 CONT=1，1ms/10ms/100ms 三点 | 0,0,0 |
  | C | ADC EN 断电 5ms 再上电 ×3 | 0,0,0 |
  | D | VC1 使能→TS、OPA1 使能（跟随器）→TS、撤销→TS | 0,0,0 |
  | E | BGR 预充 + 9 档采样放电曲线 | 120→58→28→7→1→0（S6→S42） |
  | F | 60s 预热，改用厂商配置 Div1/S166 | TS 12 点全 0；BGR 1495~1505 稳定 |
  | G/H | 末尾复测 / 关 TSEN 无偏置基线 | 0,0,0 / 0,0 |
- **E 的物理含义**（比第二轮单点 S6=83 更硬）：每次转换开始时采样电容
  已放空，开关闭合的电荷注入在 S6 窗末还剩 **120 counts ≈ 97mV**，随后
  按 **τ≈1.03µs** 泄放到 0（ln(120/58)、ln(58/28) 两段算出同一个 τ）。
  三点拟合一致：**节点 = 0V + 约 100~250kΩ 的泄放通路**。若节点真有
  0.78V，采样只会让读数往上爬，绝不会归零。
- **H 是最锋利的一刀**：TSEN=0 与 TSEN=1 的读数完全一样（都是 0）。
  说明 TSEN 位对节点电压**没有任何可观测影响**——偏置电流根本没到传感器。
- 本轮同时确认 `BGR_CR` bit2/3/8 **可写且能保持**（写 0x10F 读回 0x10F），
  所以"隐藏位被硬件锁死"的假设也排除。
- **同日第二次上电完全复现**：A/B/C/D 仍全 0；E = 117→57→29→7→2→0
  （τ 一致）；F_TS 12 点全 0，F_BGR 1543~1559；G/H 全 0；END_CR=0x107。
  这一轮 VDDA 掉到 ≈3.17V（上轮 3.28V），**BGR 跟着电源走、TS 一动不动**，
  再次说明 TS 与供电无关。
- 至此结论不变且更硬：**该芯片 TS 模拟块（偏置/输出级）无输出**，
  软件侧无解。同一份固件换一颗 CW32L012 即可一锤定音。

### ts_probe3（TS 第三轮·六角度扫描）

- 一次烧录 61s（F 相位占 60s）跑完 A~H 八个相位，观察量全部走
  `write_volatile`。用于 TS 复查 / 换板建基线。
- SRAM 布局（0x2000_0000 起，`nm` 实测）：`A_RB[8×u32]`@0x00、
  `A_TS`@0x20、`B_TS`@0x30、`C_TS`@0x36、`D_TS`@0x3C、`E_TS[9]`@0x42、
  `F_TS[12]`@0x54、`G_TS`@0x6C、`H_TS`@0x72、`F_BGR[12]`@0x76、
  `A_MASK[8×u32]`@0x90、`END_CR`@0xB0、`END_TS`@0xB4、`ADC_CR0`@0xB8、
  `BOOT_CR`@0xBC、`END_BGR`@0xC0。
- 读法：`probe-rs read b8 0x20000000 196`（hexdump 按行 `split(':')`
  后拼字节，u16 小端）。
- **坑：固件末尾别用 `wfi` 收尾**。本次在 WFI 之后首次读取连续报
  `JtagGetIdcodeError`（连 `--connect-under-reset` 也超时），靠一次
  `probe-rs reset` 才重新拿到 AP——但 reset 会把 61s 的采样从头再跑一遍。
  建议末尾空转（`loop { spin_loop() }`），或趁 F 相位跑着的时候读
  （数据是边跑边写进 SRAM 的，中途读到的就是阶段快照）。

### ts_probe（TS 上电特性判定矩阵）

- 一次烧录 30s 跑完 A/B/C/曲线/S6 五个相位，静态变量含 TS_CURVE、
  BGR_CURVE 等八组（见例程文档）。用于复测 TS 是否恢复、或换了板子
  之后建立基线。BGR_CURVE 全程应 ≈cal×4095/VDDA（±2%）。
- 注意 `probe-rs read b8` 输出带地址前缀 hexdump，解析时按行 split
  ':' 后按字节顺序拼数组（node/python 均可）。

### 调试器 APB 直读假读

见"通用操作"第 3 条——凡是要看外设寄存器，一律走 SRAM 快照。

### 静态量被优化掉（v0.6 新增坑）

开了 `opt-level` 后，只写不读的 `static mut` 会被整段删除：符号消失或
地址错位，读到的就是别的数据（曾把 `DS_WAKES` 读成 `LSI_HZ=15`）。
**观察用的静态量必须用 `write_volatile(addr_of_mut!(X), v)` 写。**

### 静态变量地址漂移

重编译后 nm 重新定位（曾把 TS_RAW 误当 TEMP_MC 读出 -271℃ 假象）。
