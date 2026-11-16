#![no_std]
#![no_main]

use cortex_m::peripheral::syst::SystClkSource;
use cortex_m_rt::entry;
use py32_hal::gpio::{Input, Level, Output, Pull, Speed};
use py32_hal::{pac, rcc::Sysclk, time::Hertz};
use {defmt_rtt as _, panic_halt as _};

mod ws2812_decode;
use ws2812_decode::packed_brightness;

core::arch::global_asm!(include_str!("ws2812_rx.s"), options(raw));
core::arch::global_asm!(include_str!("ws2812_tx.s"), options(raw));
extern "C" {
    fn ws2812_capture(bytes: *mut u8, idr: *const u32, capacity: u32) -> u32;
    fn ws2812_forward(bytes: *const u8, length: u32, timer: *mut ()) -> ();
}

const CLOCK: u32 = 24_000_000;
const LED_PINS: [u8; 8] = [2, 6, 3, 8, 0, 4, 1, 5];
const LED_MASK: u32 = 0x17f;
const RESET_TICKS: u32 = CLOCK / 1_000_000 * 50;
const LOCAL_BYTES: usize = 8 * 3;
const MAX_FRAME_BYTES: usize = 128 * 3;

// Read over SWD without stopping reception: frames, incomplete captures,
// enabled flag, eight latched brightness values, input and forwarded byte counts.
#[no_mangle]
static mut WS2812_STATUS: [u32; 13] = [0; 13];

fn status(index: usize, value: u32) {
    unsafe {
        core::ptr::addr_of_mut!(WS2812_STATUS)
            .cast::<u32>()
            .add(index)
            .write_volatile(value)
    }
}

fn elapsed(now: u32, before: u32) -> u32 {
    before.wrapping_sub(now) & 0x00ff_ffff
}

#[entry]
fn main() -> ! {
    let mut cfg = py32_hal::Config::default();
    cfg.rcc.hsi = Some(Hertz(CLOCK));
    cfg.rcc.sys = Sysclk::HSI;
    let p = py32_hal::init(cfg);
    // No interrupts or Embassy executor: reception and PWM share a polling loop.
    cortex_m::interrupt::disable();
    let _input = Input::new(p.PA12, Pull::Down);
    let _output = Output::new(p.PA7, Level::Low, Speed::VeryHigh);
    // PA7 AF2 = TIM1_CH1N. PWM1 plus inverted complementary polarity gives
    // positive pulses. CCR1 preload lets the CPU feed one bit ahead.
    py32_hal::rcc::enable_and_reset::<py32_hal::peripherals::TIM1>();
    let tim = pac::TIM1;
    tim.psc().write_value(0);
    tim.arr().write_value(pac::timer::regs::ArrCore(29));
    tim.ccr(0).write(|w| w.set_ccr(0));
    tim.ccmr_output(0).write(|w| {
        w.set_ocm(0, pac::timer::vals::Ocm::PWMMODE1);
        w.set_ocpe(0, true);
    });
    tim.ccer().write(|w| {
        w.set_ccne(0, true);
        w.set_ccnp(0, true);
    });
    tim.bdtr().write(|w| w.set_moe(true));
    tim.egr().write(|w| w.set_ug(true));
    tim.sr().write(|_| {});
    pac::GPIOA.afr(0).modify(|w| w.set_afr(7, 2));
    pac::GPIOA
        .moder()
        .modify(|w| w.set_moder(7, pac::gpio::vals::Moder::ALTERNATE));
    tim.cr1().write(|w| w.set_cen(true));
    let button = Input::new(p.PB0, Pull::None);
    let _leds = (
        Output::new(p.PA0, Level::Low, Speed::Low),
        Output::new(p.PA1, Level::Low, Speed::Low),
        Output::new(p.PA2, Level::Low, Speed::Low),
        Output::new(p.PA3, Level::Low, Speed::Low),
        Output::new(p.PA4, Level::Low, Speed::Low),
        Output::new(p.PA5, Level::Low, Speed::Low),
        Output::new(p.PA6, Level::Low, Speed::Low),
        Output::new(p.PA8, Level::Low, Speed::Low),
    );
    let mut core = cortex_m::Peripherals::take().unwrap();
    core.SYST.set_clock_source(SystClkSource::Core);
    core.SYST.set_reload(0x00ff_ffff);
    core.SYST.clear_current();
    core.SYST.enable_counter();
    let mut bytes = [0u8; MAX_FRAME_BYTES];
    let mut values = [0u8; 8];
    let mut pending = false;
    let mut captured = 0usize;
    let mut receiving = false;
    let mut low_since = None;
    let mut enabled = true;
    let mut button_raw = button.is_low();
    let mut button_stable = button_raw;
    let mut button_since = cortex_m::peripheral::SYST::get_current();
    let mut pwm_since = button_since;
    let mut accumulators = [0u16; 8];
    let mut frames = 0u32;
    let mut rejected = 0u32;
    status(2, 1);

    loop {
        let now = cortex_m::peripheral::SYST::get_current();
        let low = pac::GPIOA.idr().read().0 & (1 << 12) == 0;
        if low {
            let since = *low_since.get_or_insert(now);
            if elapsed(now, since) >= RESET_TICKS {
                if pending {
                    values = packed_brightness(&bytes[..LOCAL_BYTES]);
                    frames = frames.wrapping_add(1);
                    status(0, frames);
                    for i in 0..8 {
                        status(i + 3, values[i] as u32);
                    }
                    pending = false;
                    // The local eight pixels are consumed; retain the GRB byte
                    // order and values of every following pixel unchanged.
                    let tail = &bytes[LOCAL_BYTES..captured];
                    pac::GPIOA.brr().write_value(pac::gpio::regs::Brr(LED_MASK));
                    unsafe {
                        ws2812_forward(tail.as_ptr(), tail.len() as u32, tim.as_ptr().cast())
                    };
                    status(11, captured as u32);
                    status(12, tail.len() as u32);
                    low_since = None;
                }
                receiving = true;
            }
        } else {
            low_since = None;
        }

        let pressed = button.is_low();
        if pressed != button_raw {
            button_raw = pressed;
            button_since = now;
        }
        if pressed != button_stable && elapsed(now, button_since) >= CLOCK / 25 {
            button_stable = pressed;
            if pressed {
                enabled = !enabled;
                status(2, enabled as u32);
                if !enabled {
                    pac::GPIOA.brr().write_value(pac::gpio::regs::Brr(LED_MASK));
                }
            }
        }

        // Pulse-density modulation, capped at 25% duty because the board has
        // no LED series resistors. Capture blanks LEDs when data begins.
        if elapsed(now, pwm_since) >= CLOCK / 10_000 {
            pwm_since = now;
            let mut on = 0;
            for i in 0..8 {
                accumulators[i] += if enabled { values[i] as u16 } else { 0 };
                if accumulators[i] >= 1024 {
                    accumulators[i] -= 1024;
                    on |= 1 << LED_PINS[i];
                }
            }
            pac::GPIOA
                .bsrr()
                .write_value(pac::gpio::regs::Bsrr(on | ((LED_MASK & !on) << 16)));
        }

        if receiving {
            // The routine itself waits for the first edge, avoiding Rust entry
            // latency on bit zero. Idle calls return after about 85 us.
            let count = unsafe {
                ws2812_capture(
                    bytes.as_mut_ptr(),
                    pac::GPIOA.idr().as_ptr().cast(),
                    MAX_FRAME_BYTES as u32,
                )
            };
            if count != 0 {
                pending = count >= LOCAL_BYTES as u32
                    && count <= MAX_FRAME_BYTES as u32
                    && count % 3 == 0;
                captured = if pending { count as usize } else { 0 };
                if !pending {
                    rejected = rejected.wrapping_add(1);
                    status(1, rejected);
                }
                receiving = false;
                low_since = None;
            }
        }
    }
}
