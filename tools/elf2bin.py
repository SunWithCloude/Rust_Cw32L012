#!/usr/bin/env python3
"""ELF32 -> flat BIN / Intel HEX 转换器（用于 CW32L012 固件）。

按 PT_LOAD 程序头的物理地址（p_paddr，Flash 范围）拼接装载镜像；
忽略 SRAM 段（.bss 等 NOBITS）。

用法：python3 elf2bin.py input.elf output.bin [output.hex]
"""
import struct
import sys


def parse_elf(path):
    with open(path, 'rb') as f:
        data = f.read()
    if data[:4] != b'\x7fELF':
        raise ValueError('not an ELF file')
    ei_class, ei_data = data[4], data[5]
    if ei_class != 1 or ei_data != 1:
        raise ValueError('require ELF32 little-endian')
    # ELF header
    e_phoff = struct.unpack_from('<I', data, 0x1C)[0]
    e_phentsize = struct.unpack_from('<H', data, 0x2A)[0]
    e_phnum = struct.unpack_from('<H', data, 0x2C)[0]
    loads = []
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
        p_type, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz, p_flags, p_align = \
            struct.unpack_from('<8I', data, off)
        if p_type == 1 and p_filesz > 0:  # PT_LOAD 且有文件内容
            loads.append((p_paddr, p_offset, p_filesz, p_vaddr, p_memsz))
    return data, loads


def to_bin(data, loads, flash_limit=0x2000_0000):
    segs = sorted([l for l in loads if l[0] < flash_limit])
    if not segs:
        raise ValueError('no flash load segments')
    end = max(a + z for a, _, z, _, _ in segs)
    img = bytearray(end)
    for paddr, off, filesz, _, _ in segs:
        img[paddr:paddr + filesz] = data[off:off + filesz]
    return bytes(img)


def to_ihex(img, base=0):
    lines = []
    for i in range(0, len(img), 16):
        chunk = img[i:i + 16]
        addr = base + i
        if addr > 0xFFFF:
            ext = 0x0100 | (addr >> 16)
            cs = (-(0x02 + 2 + (ext & 0xFF) + (ext >> 8))) & 0xFF
            lines.append(f':02000002{ext:04X}{cs:02X}')
        rec = f':{len(chunk):02X}{addr & 0xFFFF:04X}00' + chunk.hex().upper()
        cs = sum(int(rec[j:j+2], 16) for j in range(1, len(rec), 2))
        rec += f'{(-cs) & 0xFF:02X}'
        lines.append(rec)
    lines.append(':00000001FF')
    return '\n'.join(lines) + '\n'


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 1
    data, loads = parse_elf(sys.argv[1])
    img = to_bin(data, loads)
    with open(sys.argv[2], 'wb') as f:
        f.write(img)
    print(f'{sys.argv[2]}: {len(img)} bytes')
    if len(sys.argv) > 3:
        with open(sys.argv[3], 'w') as f:
            f.write(to_ihex(img))
        print(f'{sys.argv[3]}: {len(img)} bytes (IHEX)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
