// The low byte of a global's address.
//
//   clang --target=xtensa-esp-elf -mcpu=esp32s3 -O2 -S low_byte.c -o -
//
// Espressif clang 21.1.3: fatal error: error in backend: Cannot select:
//   i32 = XtensaISD::PCREL_WRAPPER TargetConstantPool:i32<@g ...>
// Espressif clang 22.1.4: l32r a8, .LCPI0_0 / l8ui a2, a8, 0, which
//   returns the first byte stored at g instead of the address's low byte.
// GCC (xtensa-esp-elf 15.2.0): l32r a8, <g> / extui a2, a8, 0, 8.
// LLVM 20 (llc from esp-20.1.1, same IR): movi a8, 255 / l32r a9, ... / and.

extern char g[8];

unsigned low_byte(void) { return (unsigned)g & 0xff; }
