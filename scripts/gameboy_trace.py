#!/usr/bin/env python3
"""The lockstep gate's reference side: gameboy-python, traced slice by slice.

Run on the reference's interpreter (external/venv). A slice is 70,224 T of board time (one
frame's worth; ``gb.t`` counts dots, so a double-speed slice holds twice the M-cycles): the
board steps (``gb.cpu.step()``, which is ``GameBoy.step`` on CGB hardware) until ``gb.t``
reaches the slice's end. Each slice's line holds the CPU state at its end, the board time,
the timer, PPU and DMA position, the serial output length, a CRC-32 of every CPU bus M-cycle of
the slice (packed ``t << 26 | kind << 24 | address << 8 | value``, little-endian u64; kind 0
read, 1 write, 2 internal; the value of a read or write of an APU register, $FF10-$FF3F and
$FF76-$FF77, is zeroed with ``--mask-apu``: used before gb-apu-rust was adopted, when the Rust board's
APU was a placeholder) and a CRC-32 of the last finished picture
(``ppu.frame``'s bytes). ``target/release/gameboy-trace`` writes the same lines;
``scripts/lockstep.py`` compares them.

    gameboy_trace.py ROM --slices N [--machine KEY] [--boot skip|real|PATH]
                     [--input FILE] [--detail SLICE] [--mask-apu] --out FILE

ROM may be a .zip (its first member is read in place). ``--input``: lines ``SLICE NAME...``;
before that slice the pressed buttons become exactly NAME... (``-`` for none), released ones
first, each change through ``joypad.set_pressed`` in the order a, b, select, start, right,
left, up, down. ``--audio``: the APU captures its samples (``apu.capture``, 44,100 Hz) and each
line ends with ``snd=CRC/COUNT`` of the slice's ``take_samples()`` (little-endian i16).
``--detail SLICE``: also write every bus M-cycle of that slice, one per line.
The MBC3 clock runs on board time (``gb.t / 4194304``).
"""

from __future__ import annotations

import argparse
import struct
import zipfile
import zlib
from array import array
from pathlib import Path

from gameboy_python.board import GameBoy
from gameboy_python.boot import find_boot_rom, read_image
from gameboy_python.cartridge import load_cartridge
from gameboy_python.machines import MACHINES

SLICE_T = 70224
NAMES = ("a", "b", "select", "start", "right", "left", "up", "down")
MASK = 0xFFFFFFFFFFFFFFFF


def read_rom(path: Path) -> bytes:
    if path.suffix.lower() == ".zip":
        with zipfile.ZipFile(path) as archive:
            return archive.read(archive.infolist()[0])
    return path.read_bytes()


def is_apu(address: int) -> bool:
    return 0xFF10 <= address <= 0xFF3F or 0xFF76 <= address <= 0xFF77


def read_inputs(path: Path | None) -> dict[int, list[str]]:
    out: dict[int, list[str]] = {}
    if path is None:
        return out
    for line in path.read_text().splitlines():
        words = line.split("#")[0].split()
        if words:
            out[int(words[0])] = [w for w in words[1:] if w != "-"]
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("rom", type=Path)
    parser.add_argument("--slices", type=int, required=True)
    parser.add_argument("--machine", default="dmg")
    parser.add_argument("--boot", default="skip")
    parser.add_argument("--input", type=Path)
    parser.add_argument("--detail", type=int, default=-1)
    parser.add_argument("--mask-apu", action="store_true")
    parser.add_argument("--audio", action="store_true")
    parser.add_argument("--save-at", type=int, default=-1, help="ignored: the Rust board's round trip")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    machine = MACHINES[args.machine]
    if args.boot == "skip":
        boot = None
    elif args.boot == "real":
        boot, _label = find_boot_rom(machine.boot_rom)
        if boot is None:
            raise SystemExit("no boot ROM")
    else:
        boot = read_image(Path(args.boot))
    holder: list[GameBoy] = []
    cart = load_cartridge(
        read_rom(args.rom), clock=lambda: holder[0].t / 4194304 if holder else 0.0
    )
    gb = GameBoy(cart, machine=machine, boot_rom=boot)
    holder.append(gb)
    gb.apu.capture = args.audio
    cpu = gb.cpu
    log = array("Q")
    apu_values = not args.mask_apu
    read, write, idle = cpu.read_byte, cpu.write_byte, cpu.idle

    def traced_read(address: int) -> int:
        value = read(address)
        shown = value
        if not apu_values and is_apu(address):
            shown = 0
        log.append(((gb.t << 26) | (address << 8) | (shown & 0xFF)) & MASK)
        return value

    def traced_write(address: int, value: int) -> None:
        write(address, value)
        shown = 0 if not apu_values and is_apu(address) else value
        log.append(((gb.t << 26) | (1 << 24) | (address << 8) | shown) & MASK)

    def traced_idle(address: int) -> None:
        idle(address)
        log.append(((gb.t << 26) | (2 << 24) | (address << 8)) & MASK)

    cpu.read_byte, cpu.write_byte, cpu.idle = traced_read, traced_write, traced_idle
    inputs = read_inputs(args.input)
    pressed: set[str] = set()
    step = cpu.step
    with args.out.open("w") as out:
        for index in range(args.slices):
            if index in inputs:
                wanted = set(inputs[index])
                for name in NAMES:
                    if name in pressed and name not in wanted:
                        gb.joypad.set_pressed(name, False)
                for name in NAMES:
                    if name in wanted and name not in pressed:
                        gb.joypad.set_pressed(name, True)
                pressed = wanted
            end = (index + 1) * SLICE_T
            while gb.t < end:
                step()
            ppu, timer = gb.ppu, gb.timer
            bus = zlib.crc32(log.tobytes()) if log else 0
            frame = zlib.crc32(bytes(ppu.frame))
            out.write(
                f"{index} t={gb.t} pc={cpu.pc:04x} sp={cpu.sp:04x} "
                f"af={cpu.a:02x}{cpu.f:02x} bc={cpu.b:02x}{cpu.c:02x} "
                f"de={cpu.d:02x}{cpu.e:02x} hl={cpu.h:02x}{cpu.l:02x} "
                f"ie={cpu.ie:02x} if={cpu.if_:02x} ime={int(cpu.ime)} "
                f"halt={int(cpu.halted)} stop={int(cpu.stopped)} "
                f"div={timer.counter:04x} tima={timer.tima:02x} "
                f"ly={ppu.ly} dot={ppu.dot} mode={ppu.mode}/{ppu.read_mode} "
                f"lcdc={ppu.lcdc:02x} dma={int(gb.dma.active)} "
                f"ser={len(gb.serial.output)} frames={ppu.frames} "
                f"bus={bus:08x}/{len(log)} pic={frame:08x}"
            )
            if args.audio:
                samples = gb.apu.take_samples().tobytes()
                out.write(f" snd={zlib.crc32(samples):08x}/{len(samples) // 2}")
            out.write("\n")
            if index == args.detail:
                for record in log:
                    t, kind = record >> 26, (record >> 24) & 3
                    out.write(f"  {t} {'rwi'[kind]} {(record >> 8) & 0xFFFF:04x} {record & 0xFF:02x}\n")
            del log[:]
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
