/* The HAL's F030 memory map has 4 KiB RAM. The physical F002A has 3 KiB. */
_stack_start = 0x20000c00;
ASSERT(__euninit + 768 <= _stack_start, "WS2812 target needs at least 768 bytes of stack");
ASSERT(__sidata + SIZEOF(.data) <= 0x08005000, "WS2812 target exceeds F002A flash");
