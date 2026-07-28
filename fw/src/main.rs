#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use cortex_m::peripheral::SCB;
use cortex_m_rt::{exception, ExceptionFrame};
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};

use py32_hal::gpio::{AnyPin, Flex, Input, Pin, Pull, Speed};

use defmt::*;
use py32_hal::rcc::Sysclk;
use py32_hal::time::Hertz;
use py32_hal::Peripheral;
use {defmt_rtt as _, panic_halt as _};

mod bat;
mod sleep;

const AWAKE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const BUTTON_DEBOUNCE: Duration = Duration::from_millis(40);
const FLASH_RESCUE_WINDOW: Duration = Duration::from_secs(5);

struct DirectLedBank<'a, const PIN_COUNT: usize> {
    pin_list: [Flex<'a>; PIN_COUNT],
}

impl<'a, const PIN_COUNT: usize> DirectLedBank<'a, PIN_COUNT> {
    fn new(pin_list: [AnyPin; PIN_COUNT]) -> Self {
        let gpio_list =
            core::array::from_fn(|i| unsafe { Flex::new(pin_list[i].clone_unchecked()) });

        let mut bank = Self {
            pin_list: gpio_list,
        };
        bank.all_analog();
        bank
    }

    fn all_analog(&mut self) {
        for pin in self.pin_list.iter_mut() {
            pin.set_as_analog();
        }
    }

    fn all_on(&mut self) {
        for pin in self.pin_list.iter_mut() {
            pin.set_high();
            pin.set_as_output(Speed::VeryHigh);
        }
    }
}

async fn wait_for_button_release(button: &Input<'_>) {
    while button.is_low() {
        Timer::after_millis(10).await;
    }
    Timer::after(BUTTON_DEBOUNCE).await;
}

async fn pressed_after_debounce(button: &Input<'_>) -> bool {
    if button.is_high() {
        return false;
    }

    Timer::after(BUTTON_DEBOUNCE).await;
    button.is_low()
}

async fn flash_rescue_window(button: &Input<'_>) {
    let deadline = Instant::now() + FLASH_RESCUE_WINDOW;

    while Instant::now() < deadline {
        if button.is_low() {
            info!("button held during rescue window; staying awake");
            wait_for_button_release(button).await;
            return;
        }

        Timer::after_millis(25).await;
    }
}

async fn keep_leds_on_until_button_or_timeout(
    red: &mut DirectLedBank<'_, 4>,
    white: &mut DirectLedBank<'_, 4>,
    button: &Input<'_>,
) -> bool {
    let deadline = Instant::now() + AWAKE_TIMEOUT;
    red.all_on();
    white.all_on();

    while Instant::now() < deadline {
        if pressed_after_debounce(button).await {
            return true;
        }

        Timer::after_millis(20).await;
    }

    false
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Hello World!");

    let mut cfg: py32_hal::Config = Default::default();
    cfg.rcc.hsi = Some(Hertz::mhz(24));
    cfg.rcc.sys = Sysclk::HSI;
    let p = py32_hal::init(cfg);

    // _spawner.spawn(bat::run_bat_monitor(p.ADC)).unwrap();

    let button = Input::new(p.PB0, Pull::None);

    let red_pins = [
        p.PA0.degrade(),
        p.PA1.degrade(),
        p.PA2.degrade(),
        p.PA3.degrade(),
    ];

    let white_pins = [
        p.PA4.degrade(),
        p.PA5.degrade(),
        p.PA6.degrade(),
        p.PA8.degrade(),
    ];

    let mut red = DirectLedBank::new(red_pins);
    let mut white = DirectLedBank::new(white_pins);

    info!("flash rescue window");
    flash_rescue_window(&button).await;

    info!("entering initial stop mode");
    sleep::enter_stop_mode();
    wait_for_button_release(&button).await;
    info!("awake");

    loop {
        if keep_leds_on_until_button_or_timeout(&mut red, &mut white, &button).await {
            info!("button requested stop");
            wait_for_button_release(&button).await;
        }

        info!("entering stop mode");
        red.all_analog();
        white.all_analog();
        sleep::enter_stop_mode();
        wait_for_button_release(&button).await;
        info!("awake");
    }
}

//conserve flash space with the non-handler
#[exception]
unsafe fn HardFault(_frame: &ExceptionFrame) -> ! {
    SCB::sys_reset()
}
