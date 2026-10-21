use py32_hal::gpio::{AnyPin, Flex, Pin, Speed};
use py32_hal::interrupt::typelevel::Interrupt;
use py32_hal::pac;
use py32_hal::peripherals::TIM14;
use py32_hal::time::Hertz;
use py32_hal::timer::low_level::Timer;
use py32_hal::Peripheral;

pub const PWM_MAX: u16 = 1024;
pub const PWM_CHANNELS: usize = 8;
pub const PWM_ISR_HZ: u32 = 10_000;

static PORT_NUM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
static PIN_MASK: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static PIN_BITS: [core::sync::atomic::AtomicU32; PWM_CHANNELS] =
    [const { core::sync::atomic::AtomicU32::new(0) }; PWM_CHANNELS];
static PWM_VAL: [core::sync::atomic::AtomicU16; PWM_CHANNELS] =
    [const { core::sync::atomic::AtomicU16::new(0) }; PWM_CHANNELS];
static PWM_ACC: [core::sync::atomic::AtomicU16; PWM_CHANNELS] =
    [const { core::sync::atomic::AtomicU16::new(0) }; PWM_CHANNELS];

pub struct LedBank<'a, const PIN_COUNT: usize> {
    pin_list: [Flex<'a>; PIN_COUNT],
    first_channel: usize,
    port: pac::gpio::Gpio,
    pin_bits: [u32; PIN_COUNT],
    pin_mask: u32,
}

impl<'a, const PIN_COUNT: usize> LedBank<'a, PIN_COUNT> {
    pub fn new(first_channel: usize, pin_list: [AnyPin; PIN_COUNT]) -> Self {
        assert!(PIN_COUNT > 0);
        assert!(first_channel + PIN_COUNT <= PWM_CHANNELS);

        let port_num = pin_list[0].port();
        let port = pac::GPIO(port_num as _);
        let pin_bits = core::array::from_fn(|i| {
            assert_eq!(pin_list[i].port(), port_num);
            1u32 << pin_list[i].pin()
        });
        let pin_mask = pin_bits.iter().fold(0, |mask, bit| mask | bit);

        let gpio_list =
            core::array::from_fn(|i| unsafe { Flex::new(pin_list[i].clone_unchecked()) });

        let mut bank = Self {
            pin_list: gpio_list,
            first_channel,
            port,
            pin_bits,
            pin_mask,
        };
        bank.register_channels(port_num);
        bank.all_analog();
        bank
    }

    pub fn all_analog(&mut self) {
        self.set_mask(0);

        for pin in self.pin_list.iter_mut() {
            pin.set_as_analog();
        }
    }

    fn all_off(&mut self) {
        self.set_mask(0);
    }

    pub fn set_pwm(&mut self, index: usize, value: u16) {
        PWM_VAL[self.first_channel + index]
            .store(value.min(PWM_MAX), core::sync::atomic::Ordering::Relaxed);
    }

    pub fn start_pwm(&mut self) {
        for pin in self.pin_list.iter_mut() {
            pin.set_as_output(Speed::VeryHigh);
        }
        self.all_off();
    }

    fn set_mask(&self, on_mask: u32) {
        let off_mask = self.pin_mask & !on_mask;
        self.port.bsrr().write_value(pac::gpio::regs::Bsrr(on_mask));
        self.port.brr().write_value(pac::gpio::regs::Brr(off_mask));
    }

    fn register_channels(&mut self, port_num: u8) {
        PORT_NUM.store(port_num, core::sync::atomic::Ordering::Relaxed);

        for i in 0..PIN_COUNT {
            let channel = self.first_channel + i;
            PIN_BITS[channel].store(self.pin_bits[i], core::sync::atomic::Ordering::Relaxed);
            PWM_ACC[channel].store(
                (self.pin_bits[i].trailing_zeros() as u16 * 149) % PWM_MAX,
                core::sync::atomic::Ordering::Relaxed,
            );
        }

        PIN_MASK.store(
            PIN_MASK.load(core::sync::atomic::Ordering::Relaxed) | self.pin_mask,
            core::sync::atomic::Ordering::Relaxed,
        );
    }
}

pub fn init_timer(tim: TIM14) {
    let timer = Timer::new(tim);
    timer.set_frequency(Hertz(PWM_ISR_HZ));
    timer.clear_update_interrupt();
    timer.enable_update_interrupt(true);
    core::mem::forget(timer);
}

pub fn start() {
    pac::TIM14.dier().modify(|r| r.set_uie(true));
    pac::TIM14.sr().modify(|r| r.set_uif(false));
    pac::TIM14.cr1().modify(|r| r.set_cen(true));
    unsafe { py32_hal::interrupt::typelevel::TIM14::enable() };
}

pub fn stop() {
    py32_hal::interrupt::typelevel::TIM14::disable();
    pac::TIM14.cr1().modify(|r| r.set_cen(false));
    pac::TIM14.dier().modify(|r| r.set_uie(false));
    pac::GPIO(PORT_NUM.load(core::sync::atomic::Ordering::Relaxed) as _)
        .brr()
        .write_value(pac::gpio::regs::Brr(
            PIN_MASK.load(core::sync::atomic::Ordering::Relaxed),
        ));
}

fn tick_isr() {
    let mut on_mask = 0;

    for i in 0..PWM_CHANNELS {
        let duty = PWM_VAL[i].load(core::sync::atomic::Ordering::Relaxed);
        let acc = PWM_ACC[i].load(core::sync::atomic::Ordering::Relaxed) + duty;
        if acc >= PWM_MAX {
            PWM_ACC[i].store(acc - PWM_MAX, core::sync::atomic::Ordering::Relaxed);
            on_mask |= PIN_BITS[i].load(core::sync::atomic::Ordering::Relaxed);
        } else {
            PWM_ACC[i].store(acc, core::sync::atomic::Ordering::Relaxed);
        }
    }

    let port = pac::GPIO(PORT_NUM.load(core::sync::atomic::Ordering::Relaxed) as _);
    let pin_mask = PIN_MASK.load(core::sync::atomic::Ordering::Relaxed);
    port.bsrr().write_value(pac::gpio::regs::Bsrr(on_mask));
    port.brr()
        .write_value(pac::gpio::regs::Brr(pin_mask & !on_mask));
}

#[no_mangle]
unsafe extern "C" fn TIM14() {
    if pac::TIM14.sr().read().uif() {
        pac::TIM14.sr().modify(|r| r.set_uif(false));
        tick_isr();
    }
}
