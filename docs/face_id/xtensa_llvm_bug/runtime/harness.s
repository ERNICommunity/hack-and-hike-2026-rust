# Runs low_byte() under qemu-xtensa (Linux user mode) and exits with its
# result as the process status. g lives at 0x00500010 (see link.ld), so
# the correct result is 0x10 = 16; its first byte is 0xAB = 171.
    .text
    .global _start
    .align 4
_start:
    movi    a0, 0
    call8   low_byte        # windowed call: the result comes back in a10
    mov     a6, a10         # exit(status): the first argument is in a6
    movi    a2, 118         # __NR_exit on xtensa-linux
    syscall

    .data
    .global g
    .align 4
g:  .byte 0xAB, 0xCD, 0xEF, 0x01, 0, 0, 0, 0
