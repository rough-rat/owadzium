.syntax unified
.cpu cortex-m0plus
.thumb
// SRAM execution keeps flash wait states out of the sampling interval.
.section .data.ws2812_capture,"ax",%progbits
.balign 4
.global ws2812_capture
.type ws2812_capture,%function

// Pack one sampled input bit into the current GRB byte.
// At 24 MHz, sample between the nominal 0.4 us and 0.8 us falling edges.
.macro sample offset
    .rept 7
    nop
    .endr
    ldr r5, [r1]
    lsrs r5, r5, #13
    adcs r6, r6
    .if \offset == 14
    // Commit the byte while the last pulse is still finishing. Doing this
    // after its falling edge would consume the next bit's short low interval.
    strb r6, [r0]
    adds r0, #1
    adds r7, #1
    subs r2, #1
    .endif
    movs r4, #32
.Lfall\@:
    ldr r3, [r1]
    lsls r3, r3, #19
    bpl .Llow\@
    subs r4, #1
    bne .Lfall\@
    b failed
.Llow\@:
.endm

.macro bit offset
    movs r4, #32
.Lrise\@:
    ldr r3, [r1]
    lsls r3, r3, #19
    bmi .Lsample\@
    subs r4, #1
    bne .Lrise\@
    b failed
.Lsample\@:
    sample \offset
.endm

.thumb_func
// r0: byte buffer, r1: GPIOA IDR, r2: buffer capacity in bytes.
// Returns captured bytes on end-of-data, 0 when idle, -1 on error/overflow.
// Byte unrolling removes per-bit pointer/count work from the shortest lows.
ws2812_capture:
    push {r4, r5, r6, r7, lr}
    movs r7, #0
    movs r6, #0
    movs r4, #255
    b first_rise
next_byte:
    movs r4, #32
first_rise:
    ldr r3, [r1]
    lsls r3, r3, #19
    bmi first_sample
    subs r4, #1
    bne first_rise
    mov r0, r7
    pop {r4, r5, r6, r7, pc}
first_sample:
    cmp r2, #0
    bne have_space
    b failed
have_space:
    // Compensate the capacity check in the first bit's sample delay.
    .rept 3
    nop
    .endr
    ldr r5, [r1]
    lsrs r5, r5, #13
    adcs r6, r6
    // Blank the local LEDs before a potentially long receive operation.
    ldr r3, =0x17f
    str r3, [r1, #0x18]
    movs r4, #32
first_fall:
    ldr r3, [r1]
    lsls r3, r3, #19
    bpl remaining_bits
    subs r4, #1
    bne first_fall
    b failed
remaining_bits:
    bit 2
    bit 4
    bit 6
    bit 8
    bit 10
    bit 12
    bit 14
    b next_byte
failed:
    movs r0, #0
    mvns r0, r0
    pop {r4, r5, r6, r7, pc}
.size ws2812_capture, .-ws2812_capture
.ltorg
