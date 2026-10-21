#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use cortex_m::peripheral::SCB;
use cortex_m_rt::{exception, ExceptionFrame};
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};

use py32_hal::gpio::{Input, Pin, Pull};

use defmt::*;
use py32_hal::rcc::Sysclk;
use py32_hal::time::Hertz;
use {defmt_rtt as _, panic_halt as _};

mod bat;
mod sleep;
mod soft_pwm;

use soft_pwm::{LedBank, PWM_MAX};

const AWAKE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const BUTTON_DEBOUNCE: Duration = Duration::from_millis(40);
const FLASH_RESCUE_WINDOW: Duration = Duration::from_secs(5);
const CONTROL_TICK: Duration = Duration::from_millis(4);

const VCC_EMPTY_MV: u32 = 2000;
const VCC_FULL_MV: u32 = 3600;

const SINE_QUARTER_10: [u16; 17] = [
    0, 100, 200, 297, 392, 483, 568, 647, 724, 792, 851, 903, 946, 981, 1004, 1018, 1023,
];

#[derive(Clone, Copy)]
struct VoltageBrightness {
    empty: u16,
    full: u16,
}

#[derive(Clone, Copy)]
struct BankAnimation {
    baseline: VoltageBrightness,
    wobble: u16,
    phases: [u16; 4],
}

#[derive(Clone, Copy)]
struct AnimationMode {
    name: &'static str,
    phase_step: u16,
    green: BankAnimation,
    pink: BankAnimation,
}

const ANIMATION_MODES: [AnimationMode; 5] = [
    AnimationMode {
        name: "dimm",
        phase_step: 2,
        green: BankAnimation {
            baseline: VoltageBrightness {
                 empty: 10,
                  full: 50 
                },
            wobble: 10,
            phases: [0, 224, 0, 224],
        },
        pink: BankAnimation {
            baseline: VoltageBrightness {
                empty: 100  ,
                full: 200,
            },
            wobble: 100,
            phases: [128, 416, 128, 416],
        },
    },
    AnimationMode {
        name: "sync",
        phase_step: 1,
        green: BankAnimation {
            baseline: VoltageBrightness {
                empty: 30,
                full: 120,
            },
            wobble: 50,
            phases: [500, 500, 500, 500],
        },
        pink: BankAnimation {
            baseline: VoltageBrightness {
                empty: 48,
                full: 224,
            },
            wobble: 90,
            phases: [0, 0, 0, 0],
        },
    },
    AnimationMode {
        name: "osc",
        phase_step: 1,
        green: BankAnimation {
            baseline: VoltageBrightness {
                empty: 48,
                full: 150,
            },
            wobble: 300,
            phases: [0, 80, 160, 240],
        },
        pink: BankAnimation {
            baseline: VoltageBrightness {
                empty: 48,
                full: 224,
            },
            wobble: 288,
            phases: [512, 592, 672, 752],
        },
    },
    AnimationMode {
        name: "glow",
        phase_step: 1,
        green: BankAnimation {
            baseline: VoltageBrightness {
                empty: 100,
                full: 200,
            },
            wobble: 120,
            phases: [0, 0, 0, 0],
        },
        pink: BankAnimation {
            baseline: VoltageBrightness {
                empty: 100,
                full: 250,
            },
            wobble: 140,
            phases: [256, 256, 256, 256],
        },
    },
    AnimationMode {
        name: "pink",
        phase_step: 1,
        green: BankAnimation {
            baseline: VoltageBrightness {
                empty: 0,
                full: 1,
            },
            wobble: 0,
            phases: [0, 0, 0, 0],
        },
        pink: BankAnimation {
            baseline: VoltageBrightness {
                empty: 100,
                full: 300,
            },
            wobble: 112,
            phases: [128, 128, 128, 128],
        },
    }
];

struct ButtonDebouncer {
    low_since: Option<Instant>,
    armed: bool,
}

impl ButtonDebouncer {
    fn new() -> Self {
        Self {
            low_since: None,
            armed: true,
        }
    }

    fn update(&mut self, button: &Input<'_>) -> bool {
        if button.is_high() {
            self.low_since = None;
            self.armed = true;
            return false;
        }

        let now = Instant::now();
        let low_since = match self.low_since {
            Some(low_since) => low_since,
            None => {
                self.low_since = Some(now);
                return false;
            }
        };

        if self.armed && now >= low_since + BUTTON_DEBOUNCE {
            self.armed = false;
            true
        } else {
            false
        }
    }
}

async fn wait_for_button_release(button: &Input<'_>) {
    while button.is_low() {
        Timer::after_millis(10).await;
    }
    Timer::after(BUTTON_DEBOUNCE).await;
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
    green: &mut LedBank<'_, 4>,
    pink: &mut LedBank<'_, 4>,
    button: &Input<'_>,
    vcc_mv: u32,
) -> bool {
    let deadline = Instant::now() + AWAKE_TIMEOUT;
    green.start_pwm();
    pink.start_pwm();
    soft_pwm::start();

    let mut animation_step: u16 = 0;
    let mut button_debouncer = ButtonDebouncer::new();
    let mut mode_index = 0;
    info!("animation mode: {}", ANIMATION_MODES[mode_index].name);

    while Instant::now() < deadline {
        let mode = ANIMATION_MODES[mode_index];

        apply_bank_animation(green, animation_step, vcc_mv, mode.green);
        apply_bank_animation(pink, animation_step, vcc_mv, mode.pink);
        animation_step = animation_step.wrapping_add(mode.phase_step);

        if button_debouncer.update(button) {
            mode_index += 1;
            if mode_index == ANIMATION_MODES.len() {
                return true;
            }

            animation_step = 0;
            info!("animation mode: {}", ANIMATION_MODES[mode_index].name);
        }

        Timer::after(CONTROL_TICK).await;
    }

    false
}

fn brightness_for_voltage(vcc_mv: u32, empty_brightness: u16, full_brightness: u16) -> u16 {
    let vcc_mv = vcc_mv.clamp(VCC_EMPTY_MV, VCC_FULL_MV);
    let span = VCC_FULL_MV - VCC_EMPTY_MV;
    let pos = vcc_mv - VCC_EMPTY_MV;
    let empty = empty_brightness as u32;
    let full = full_brightness as u32;

    (empty + (full - empty) * pos / span) as u16
}

fn apply_bank_animation<const PIN_COUNT: usize>(
    bank: &mut LedBank<'_, PIN_COUNT>,
    step: u16,
    vcc_mv: u32,
    config: BankAnimation,
) {
    let baseline = brightness_for_voltage(vcc_mv, config.baseline.empty, config.baseline.full);

    for i in 0..PIN_COUNT {
        let wave = sine_10(step.wrapping_add(config.phases[i]));
        let brightness = baseline as i32 + wave as i32 * config.wobble as i32 / 1023;
        bank.set_pwm(i, brightness.clamp(0, PWM_MAX as i32) as u16);
    }
}

fn sine_10(phase: u16) -> i16 {
    let phase = phase & 0x03ff;
    let quadrant = phase >> 8;
    let quarter_phase = phase & 0x00ff;
    let mirrored = if quadrant & 1 == 0 {
        quarter_phase
    } else {
        255 - quarter_phase
    };

    let index = (mirrored >> 4) as usize;
    let frac = mirrored & 0x0f;
    let lo = SINE_QUARTER_10[index] as u32;
    let hi = SINE_QUARTER_10[index + 1] as u32;
    let magnitude = lo + (hi - lo) * frac as u32 / 16;

    if quadrant < 2 {
        magnitude as i16
    } else {
        -(magnitude as i16)
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Hello World!");

    let mut cfg: py32_hal::Config = Default::default();
    cfg.rcc.hsi = Some(Hertz::mhz(4)); //0.5mA in theory
    cfg.rcc.sys = Sysclk::HSI;
    let p = py32_hal::init(cfg);

    let vcc_mv = bat::measure_vcc_mv(p.ADC).await;

    let button = Input::new(p.PB0, Pull::None);

    // pa2, pa6, pa3, pa8 - green
    // pa0, pa4, pa1, pa5 - pink

    let green_pins = [
        p.PA2.degrade(),
        p.PA6.degrade(),
        p.PA3.degrade(),
        p.PA8.degrade(),
    ];

    let pink_pins = [
        p.PA0.degrade(),
        p.PA4.degrade(),
        p.PA1.degrade(),
        p.PA5.degrade(),
    ];

    let mut green = LedBank::new(0, green_pins);
    let mut pink = LedBank::new(4, pink_pins);
    soft_pwm::init_timer(p.TIM14);

    info!("flash rescue window");
    flash_rescue_window(&button).await;

    info!("entering initial stop mode");
    // sleep::enter_stop_mode();
    // wait_for_button_release(&button).await;
    // info!("awake");

    loop {
        if keep_leds_on_until_button_or_timeout(&mut green, &mut pink, &button, vcc_mv).await {
            info!("button requested stop");
            wait_for_button_release(&button).await;
        }

        info!("entering stop mode");
        soft_pwm::stop();
        green.all_analog();
        pink.all_analog();
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
