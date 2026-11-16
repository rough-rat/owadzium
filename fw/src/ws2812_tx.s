.syntax unified
.cpu cortex-m0plus
.thumb
.section .data.ws2812_forward,"ax",%progbits
.balign 4
.global ws2812_forward
.type ws2812_forward,%function

.macro update
.Lwait\@:
    ldr r4, [r2, #0x10]
    lsls r4, r4, #31
    bpl .Lwait\@
    str r5, [r2, #0x10]
.endm

.macro transmit_bit
    update
    lsls r3, r3, #1
    movs r4, #10
    bcc .Lzero\@
    movs r4, #19
.Lzero\@:
    // CCR1 preload takes effect at the next update, giving jitter-free
    // 416.7/791.7 ns high pulses and exactly 1.25 us bit periods.
    str r4, [r2, #0x34]
.endm

.thumb_func
// r0: packed GRB bytes, r1: length, r2: TIM1 base. Interrupts must be disabled.
// TIM1 CH1N runs PWM1 with inverted complementary polarity on PA7.
ws2812_forward:
    push {r4, r5, lr}
    movs r5, #0
    cmp r1, #0
    beq tx_done
    str r5, [r2, #0x10]
tx_byte:
    ldrb r3, [r0]
    lsls r3, r3, #24
    transmit_bit
    transmit_bit
    transmit_bit
    transmit_bit
    transmit_bit
    transmit_bit
    transmit_bit
    transmit_bit
    adds r0, #1
    subs r1, #1
    beq tx_finish
    b tx_byte
tx_finish:
    // Start the final bit, preload idle low, then wait for it to finish.
    update
    str r5, [r2, #0x34]
    update
tx_done:
    pop {r4, r5, pc}
.size ws2812_forward, .-ws2812_forward
