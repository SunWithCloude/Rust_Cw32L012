"""静态校验：反汇编 ecosystem 例程，确认 PRIMASK 相关指令真的被生成。

硬件此刻未连接，先用 objdump 等价手段证明内联汇编编译正确：
在 main 的机器码里找 mrs/cpsid/cpsie 三条指令，并打印上下文。
"""
import sys

from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB, CS_MODE_MCLASS
from elftools.elf.elffile import ELFFile

PATH = sys.argv[1] if len(sys.argv) > 1 else 'target/thumbv6m-none-eabi/release/examples/ecosystem'

with open(PATH, 'rb') as f:
    elf = ELFFile(f)
    symtab = elf.get_section_by_name('.symtab')
    main = next(s for s in symtab.iter_symbols() if s.name == 'main')
    addr, size = main['st_value'] & ~1, main['st_size']  # Thumb 符号最低位是状态位，要去掉
    sec = elf.get_section(main['st_shndx'])
    off = addr - sec['sh_addr']
    code = sec.data()[off:off + size]

md = Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_MCLASS)
md.detail = True

want = {'mrs': 'MRS（读 PRIMASK）', 'cpsid': 'CPSID（关中断）', 'cpsie': 'CPSIE（开中断）'}
found = {k: 0 for k in want}
lines = []
for insn in md.disasm(code, addr):
    text = f'{insn.address:08x}: {insn.mnemonic:<8} {insn.op_str}'
    lines.append(text)
    key = insn.mnemonic.lower().split('.')[0]
    if key in found:
        found[key] += 1

print(f'main @0x{addr:x}，{size} 字节，共 {len(lines)} 条指令')
for k, label in want.items():
    print(f'  {label:<16} 出现 {found[k]} 次')

print('\n--- 含 PRIMASK/CPS 的指令及上下文 ---')
for i, ln in enumerate(lines):
    m = ln.split(':', 1)[1].strip().split()[0].split('.')[0]
    if m in want:
        for j in range(max(0, i - 2), min(len(lines), i + 3)):
            print(('  >> ' if j == i else '     ') + lines[j])
        print()

ok = found['mrs'] >= 1 and found['cpsid'] >= 1 and found['cpsie'] >= 1
print('结论:', 'OK —— 三条指令均存在' if ok else 'FAIL —— 缺少指令')
sys.exit(0 if ok else 1)
