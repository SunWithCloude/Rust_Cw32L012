"""UART 对夹验证上位机脚本（CH340X USB-TTL ↔ CW32L012 PA02/PA03）。

流程：
1. 打开串口（默认 115200-8-N-1，raw 模式）
2. 通过 SWD 复位目标（固件重新发 banner）
3. 读 banner —— 验证 MCU → PC 方向
4. 发 0x00..0xFF 全字节 pattern，逐字节比对回显 —— 验证 PC → MCU → PC 双向
5. 用 probe-rs 读 SRAM 里的观测块 —— 验证分频值、收发计数、错误计数

用法：
    python tools/uart_link_test.py --port COM5 --obs 0x20000000
"""

import argparse
import subprocess
import sys
import time

import serial
from serial.tools import list_ports

BANNER = b"CW32L012 UART LINK OK\r\n"
PROBE = ["--probe", "0483:3748", "--protocol", "swd", "--chip", "CW32L012x8"]
MAGIC = 0x55505254


def pick_port(explicit: str | None) -> str:
    if explicit:
        return explicit
    ports = list(list_ports.comports())
    if len(ports) == 1:
        print(f"自动选择串口: {ports[0].device} ({ports[0].description})")
        return ports[0].device
    if not ports:
        sys.exit("没有找到任何串口：CH340X 是否插上？驱动是否装好？")
    sys.exit("有多个串口，请用 --port 指定：" + ", ".join(p.device for p in ports))


def probe_rs(*args: str) -> str:
    cmd = ["probe-rs", *args, *PROBE]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=40)
    if r.returncode != 0:
        sys.exit(f"probe-rs 失败: {' '.join(cmd)}\n{r.stderr.strip()}")
    return r.stdout


def read_obs(addr: int) -> list[int]:
    """读 7 个 u32（magic/brri/brrf/rx/tx/err/last）"""
    n = 7 * 4
    out = probe_rs("read", "b8", hex(addr), str(n))
    data = bytearray()
    for line in out.splitlines():
        if ":" not in line:
            continue
        _, _, payload = line.partition(":")
        for tok in payload.split():
            try:
                data.append(int(tok, 16))
            except ValueError:
                pass
    if len(data) < n:
        sys.exit(f"读取 SRAM 失败，只拿到 {len(data)} 字节")
    return [int.from_bytes(data[i * 4:i * 4 + 4], "little") for i in range(7)]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="串口名，例如 COM5（只有一个串口时可省略）")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--obs", help="观测块地址（16 进制），例如 0x20000000")
    ap.add_argument("--no-reset", action="store_true", help="不通过 SWD 复位目标")
    args = ap.parse_args()

    port = pick_port(args.port)
    results: list[tuple[str, bool, str]] = []

    ser = serial.Serial(port, args.baud, timeout=1.0, write_timeout=2.0)
    try:
        ser.reset_input_buffer()

        # ---- 1. 复位，抓 banner（MCU → PC）----
        if not args.no_reset:
            probe_rs("reset")
        ser.reset_input_buffer()

        deadline = time.time() + 5.0
        buf = bytearray()
        while time.time() < deadline and BANNER not in buf:
            chunk = ser.read(64)
            if chunk:
                buf += chunk
        got_banner = BANNER in buf
        results.append(("banner (MCU→PC)", got_banner,
                        repr(bytes(buf[:60])) if buf else "5s 内没收到任何字节"))

        # ---- 2. 全字节回显（PC → MCU → PC）----
        pattern = bytes(range(256))
        t0 = time.perf_counter()
        ser.write(pattern)
        ser.flush()
        echo = bytearray()
        deadline = time.time() + 3.0
        while len(echo) < len(pattern) and time.time() < deadline:
            chunk = ser.read(len(pattern) - len(echo))
            if chunk:
                echo += chunk
        dt = time.perf_counter() - t0

        ok_len = len(echo) == len(pattern)
        ok_data = bytes(echo) == pattern
        results.append(("echo 长度", ok_len, f"{len(echo)}/{len(pattern)} 字节"))
        results.append(("echo 内容逐字节一致", ok_data,
                        "全 256 个字节值（含 0x00/0x0D/0x0A）完全一致" if ok_data
                        else f"首个差异: {next((i for i, (a, b) in enumerate(zip(echo, pattern)) if a != b), '长度不足')}"))
        if ok_len:
            results.append(("往返吞吐", True,
                            f"256 B 往返 {dt*1000:.1f} ms（理论 ~44 ms @ {args.baud}，实际含缓冲）"))

        # ---- 3. SRAM 观测块 ----
        if args.obs:
            magic, brri, brrf, rx, tx, err, last = read_obs(int(args.obs, 16))
            results.append(("观测块魔数", magic == MAGIC, f"0x{magic:08X}（期望 0x{MAGIC:08X}）"))
            results.append(("波特率分频 BRRI", brri > 0, f"BRRI={brri} BRRF={brrf}"))
            results.append(("收字节数 rx", rx >= len(pattern), f"{rx}"))
            results.append(("回显字节数 tx", tx >= len(pattern), f"{tx}"))
            results.append(("接收错误 err", err == 0, f"{err} 次"))
            results.append(("最后一字节", last == pattern[-1], f"0x{last:02X}（期望 0x{pattern[-1]:02X}）"))
    finally:
        ser.close()

    print()
    width = max(len(n) for n, _, _ in results)
    all_ok = True
    for name, ok, detail in results:
        flag = "PASS" if ok else "FAIL"
        all_ok &= ok
        print(f"  [{flag}] {name:<{width}}  {detail}")
    print()
    print("结论:", "UART 对夹全部通过" if all_ok else "有项目失败，见上表")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
