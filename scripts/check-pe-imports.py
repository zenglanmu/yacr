#!/usr/bin/env python3
"""Inspect a Windows PE executable's import tables (standard library only).

Cross-toolchain replacement for ``objdump -p``: the Windows release package is
built either with MinGW-w64 (GNU) or natively with MSVC, so the packaging script
must list imported DLLs on both a Linux host and a Windows runner.

Both the normal Import Directory and the Delay Import Directory are read (the
latter carries several Windows API-set DLLs that Rust links delay-loaded), and
names are de-duplicated preserving first-seen order -- matching the set
``objdump -p`` reports.

Usage:
    check-pe-imports.py EXE [EXE ...]     # one DLL name per line, machine=0x8664 required
    check-pe-imports.py --machine EXE     # print the COFF machine as hex

Exit status:
    0  every input is a readable PE32/PE32+ x86-64 image
    2  a file is unreadable, not PE, or not x86-64
    1  bad usage
"""
from __future__ import annotations

import struct
import sys

IMAGE_FILE_MACHINE_AMD64 = 0x8664
PE32_MAGIC = 0x10B
PE32_PLUS_MAGIC = 0x20B
DIRECTORY_IMPORT = 1
DIRECTORY_DELAY_IMPORT = 13


class PeError(Exception):
    pass


def _read(path: str) -> bytes:
    try:
        with open(path, "rb") as handle:
            return handle.read()
    except OSError as exc:  # surfaced, never swallowed
        raise PeError(f"{path}: cannot read ({exc})") from exc


def _u16(data: bytes, offset: int) -> int:
    return struct.unpack_from("<H", data, offset)[0]


def _u32(data: bytes, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def _u64(data: bytes, offset: int) -> int:
    return struct.unpack_from("<Q", data, offset)[0]


def _cstring(data: bytes, offset: int) -> str:
    end = data.find(b"\0", offset)
    if end == -1:
        raise PeError("unterminated string in import table")
    return data[offset:end].decode("ascii", "replace")


def parse(data: bytes) -> tuple[int, list[str]]:
    """Return (machine, de-duplicated imported DLL names) for a PE image."""
    if len(data) < 0x40 or data[:2] != b"MZ":
        raise PeError("not a PE image (missing MZ)")
    pe_offset = _u32(data, 0x3C)
    if pe_offset + 24 > len(data) or data[pe_offset : pe_offset + 4] != b"PE\0\0":
        raise PeError("not a PE image (missing PE signature)")
    coff = pe_offset + 4
    machine = _u16(data, coff)
    number_of_sections = _u16(data, coff + 2)
    size_of_optional = _u16(data, coff + 16)
    optional = coff + 20
    magic = _u16(data, optional)
    if magic == PE32_PLUS_MAGIC:
        directories = optional + 112
        image_base = _u64(data, optional + 24)
    elif magic == PE32_MAGIC:
        directories = optional + 96
        image_base = _u32(data, optional + 28)
    else:
        raise PeError(f"unknown optional-header magic 0x{magic:x}")

    sections = []
    section_start = optional + size_of_optional
    for index in range(number_of_sections):
        base = section_start + index * 40
        if base + 40 > len(data):
            raise PeError("truncated section table")
        virtual_size = _u32(data, base + 8)
        virtual_address = _u32(data, base + 12)
        raw_size = _u32(data, base + 16)
        raw_pointer = _u32(data, base + 20)
        sections.append((virtual_address, virtual_size or raw_size, raw_pointer))

    def rva_to_offset(rva: int) -> int:
        for virtual_address, size, raw_pointer in sections:
            if virtual_address <= rva < virtual_address + size:
                return rva - virtual_address + raw_pointer
        raise PeError(f"RVA 0x{rva:x} is not mapped by any section")

    names: list[str] = []
    seen: set[str] = set()

    def add(name: str) -> None:
        name = name.lower()
        if name and name not in seen:
            seen.add(name)
            names.append(name)

    def read_names(directory_index: int, is_delay: bool) -> None:
        entry = directories + directory_index * 8
        if entry + 8 > len(data):
            return
        rva = _u32(data, entry)
        size = _u32(data, entry + 4)
        if rva == 0 or size == 0:
            return
        offset = rva_to_offset(rva)
        step = 32 if is_delay else 20
        name_field = 1 if is_delay else 3
        while True:
            if offset + step > len(data):
                raise PeError("truncated import descriptor table")
            fields = struct.unpack_from("<IIIII", data, offset) if not is_delay else (
                struct.unpack_from("<IIIIIIII", data, offset)
            )
            if not any(fields):
                break
            attributes = fields[0]
            name_rva = fields[name_field]
            if is_delay and not (attributes & 1):
                # Legacy VA-based delay descriptor: name is an absolute VA.
                name_rva -= image_base
            add(_cstring(data, rva_to_offset(name_rva)))
            offset += step

    read_names(DIRECTORY_IMPORT, is_delay=False)
    read_names(DIRECTORY_DELAY_IMPORT, is_delay=True)
    return machine, names


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 1
    machine_only = argv[1] == "--machine"
    paths = argv[2:] if machine_only else argv[1:]
    if not paths:
        print("check-pe-imports: no input", file=sys.stderr)
        return 1
    try:
        for path in paths:
            machine, names = parse(_read(path))
            if machine != IMAGE_FILE_MACHINE_AMD64:
                raise PeError(f"{path}: machine 0x{machine:x} is not x86-64 (0x8664)")
            if machine_only:
                print(f"{path}: machine=0x{machine:x}")
            else:
                for name in names:
                    print(name)
    except PeError as exc:
        print(f"check-pe-imports: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
