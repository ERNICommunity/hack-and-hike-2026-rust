ENTRY(_start)
SECTIONS {
  . = 0x00400000;
  .text : { *(.literal .literal.*) *(.text .text.*) }
  . = 0x00500000;
  .data : { . = 0x10; *(.data .data.*) }   /* g at 0x00500010, low byte 0x10 */
}
