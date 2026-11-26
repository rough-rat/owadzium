# Providence Badge Firmware

Rust firmware for the PY32F002A-based Providence Badge.

## Current Features

- Drives two 4-pin charlieplex LED banks:
  - pink hour ring on `PA0..PA3`
  - white half-step ring on `PA4`, `PA5`, `PA6`, `PA8`
- Implements Stop-mode sleep with PB0 wake:
  - PB0 is expected to have an external pullup and button-to-ground
  - firmware wakes for about 1 hour
  - pressing PB0 while awake returns to sleep
- Uses low-power Stop configuration:
  - low-power regulator enabled
  - 1.0 V Stop-mode voltage scaling
  - reduced SRAM retention voltage
- Provides a 5 second flash rescue window after reset.
  - Hold PB0 during reset to keep the firmware awake for flashing/debug.
- Includes a layout/SVG-based LED index workflow:
  - `../layout.json` is the source layout
  - `../led-layout.svg` marks LED positions, colors, and indexes
- Current animation test:
  - rotating 4-pink-LED cross for about 2 seconds
  - white LEDs with layout indexes `14`, `15`, `20`, `21` for 2 seconds
  - 6 seconds dark

## Build

```sh
cargo +nightly build --release
```

## Flash

```sh
./flash.sh
```

If the target is hard to attach to, wake it with PB0 or hold PB0 during reset. As a last resort:

```sh
CONNECT_UNDER_RESET=1 ./flash.sh
```

## Logs

```sh
probe-rs attach --chip PY32F030x8 target/thumbv6m-none-eabi/release/blobbadge-fw
```

## WS2812 receiver target

The separate `ws2812` binary receives an 800 kHz, MSB-first GRB stream on
`PA12`. Eight RGB pixels control the eight fixed-color LEDs. Each LED uses
`max(R, G, B)` as its brightness, so any primary color can light it. Pixel
order follows the existing firmware banks:

| Pixel | Pin | Physical color |
| --- | --- | --- |
| 0 | PA4 | green |
| 1 | PA6 | green |
| 2 | PA5 | green |
| 3 | PA8 | green |
| 4 | PA0 | pink |
| 5 | PA2 | pink |
| 6 | PA1 | pink |
| 7 | PA3 | pink |

```sh
cargo build --release --bin ws2812
BIN=ws2812 ./flash.sh
```

The original firmware remains the default run/flash target. PB0 toggles
the receiver's LEDs off/on with 40 ms debounce; reception continues while off.
The receiver starts enabled, holds the last valid frame when the source stops,
and does not enter low-power modes. A low interval of at least 50 us latches a
complete frame. Frames shorter than eight pixels, partial RGB pixels, and
overlength frames leave the displayed frame unchanged and are not forwarded.

`PA7` is the data output: connect it to the next board's `PA12`, with a common
ground. Each board consumes exactly the first eight RGB pixels and forwards
the remaining GRB bytes unchanged, even when its local LEDs are disabled.
An eight-pixel input produces no output pulses. PA7 is low between frames.

Forwarding is buffered: it starts after the input reset interval, rather than
streaming concurrently with reception. A TIM1 PWM channel regenerates an
800 kHz signal with 416.7/791.7 ns high pulses. The buffer holds up to 128 pixels
(16 eight-LED boards). Allow an idle interval after each input frame at least as
long as its forwarded tail plus 100 us; input arriving during transmission is
not captured. The test source's 20 ms interval accommodates the full buffer.
Each hop adds the time needed to receive its input frame and detect reset.

Reception runs from SRAM at 24 MHz with interrupts disabled. Software
pulse-density modulation drives the LEDs, with maximum brightness scaled to
approximately 25% duty for this resistorless board. The LEDs are blanked during
packet reception and forwarding. The idle loop periodically services
the button and LEDs, so a frame beginning during that short service interval
can be discarded; use a continuously refreshed stream. This is intended for
nominal 800 kHz timing, not all WS2812 variants or their full timing tolerances.
The target linker script places the stack within the F002A's physical 3 KiB RAM.

### Implementation

- `src/ws2812.rs` owns frame validation, button debounce, LED modulation, and
  forwarding. A 384-byte buffer holds the complete incoming GRB frame.
- `src/ws2812_rx.s` samples PA12 and packs bits into bytes in SRAM. Its waits
  are bounded so disconnected or stuck inputs return control to the main loop.
- `src/ws2812_tx.s` feeds TIM1 CCR1 preload one bit ahead. The timer drives
  PA7 through AF2 (`TIM1_CH1N`); its output stays low after the final bit.
- `src/ws2812_decode.rs` maps the first eight RGB pixels to local brightness.
  `ws2812.x` restricts the stack and image to the F002A's actual memory limits.

For two boards, send sixteen pixels: board one displays pixels 0–7 and its
PA7 sends pixels 8–15 to board two. Board two consumes those eight pixels and
produces no further data. This also works when either board's LEDs are off.

### ESP32 test source

`tools/ws2812-source` contains a PlatformIO project for a classic ESP32 dev board.
Connect **GPIO18 to PA12** and **GND to GND**. The signal is 3.3 V; use a
compatible badge supply. Change `WS2812_PIN` in `platformio.ini` for another pin.
For return-data verification, connect **PA7 to ESP32 GPIO19**. For a physical
chain, PA7 instead connects to the next board's PA12.

```sh
pio run -d tools/ws2812-source
pio run -d tools/ws2812-source -t upload --upload-port /dev/ttyUSB0
pio device monitor -d tools/ws2812-source --port /dev/ttyUSB0
```

The RMT peripheral sends sixteen GRB pixels approximately 50 times per second,
with 400/800 ns high pulses and a 300 us reset. The automatic sequence cycles
through chase, ramp, off, on, individual RGB components, and fixed test values.
At 115200 baud, send `a` for automatic, `0` for off, `1` for on, `c` for chase,
`r`, `g`, or `b` for a primary component, or `t` for brightness values
`[0, 1, 128, 255, 51, 254, 170, 3]` on the first board. The second group uses
these values in reverse order, with varying RGB components. Send `8` for an
eight-pixel source (output should stay silent), or `d` to restore sixteen.
The GPIO19 RMT receiver checks that exactly the last eight pixels return,
printing byte data, high-pulse widths in 25 ns ticks, and PASS/FAIL at 115200 baud.

For SWD diagnostics, the exported `WS2812_STATUS` array contains the latched
frame count, rejected capture count, enabled flag, eight brightness values,
last input byte count, and last forwarded byte count (all `u32`). Indexes are
`0`, `1`, `2`, `3..10`, `11`, and `12`, respectively. Find its address and read
thirteen words without halting the CPU:

```sh
cargo nm --release --bin ws2812 -- --defined-only | rg WS2812_STATUS
probe-rs read --chip PY32F002Ax5 b32 <symbol-address> 13
```

For sixteen input pixels, the final two words should be `00000030 00000018`
(48 received bytes and 24 forwarded bytes). These counters confirm the
firmware path; the GPIO19 return connection checks the actual output bytes.

The pure decoder can also be checked on the host:

```sh
rustc --edition=2021 --test src/ws2812_decode.rs -o /tmp/ws2812-decode-test
/tmp/ws2812-decode-test
```

Hardware validation: both MCUs were flashed and the receiver's SWD brightness
values matched off, full-on, red-only, green-only, blue-only, and the fixed
eight-value test pattern at nominal 400/800 ns timing. With sixteen input pixels,
all six checks reported 48 input bytes and 24 forwarded bytes. The MCU's PA7
edge detector confirmed no pulses for eight-pixel input and output activity for
sixteen-pixel input. External PA7 waveform/byte verification and a physical
multi-board chain test remain unperformed. Occasional rejected frames were
observed during continuous refresh; the last complete frame remains displayed.

## LED brigtness notes

The schematic of the bug badge is direct IO-LED connection. No resistor in series.
This isn't done according to best practices, but we can get away with this, as the
circuit is very simple. Also, in theory, PWMing LEDs gives us a bit more optimal
usage of the battery juice.

2 bottom LED pairs should be green, 2 top should be pink.

Green: ~2V
Pink: ~2.8V

