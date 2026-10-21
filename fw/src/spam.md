
    use py32_hal::timer::simple_pwm::{PwmPin, SimplePwm};
    use py32_hal::time::khz;
    use py32_hal::gpio::OutputType;

    let ch4_pin1 = PwmPin::new_ch1(p.PA2, OutputType::PushPull);
    // let ch4_pin2 = PwmPin::new_ch2(p.PA0, OutputType::PushPull);
    // let ch4_pin3 = PwmPin::new_ch3(p.PA1, OutputType::PushPull);
    let ch4_pin4 = PwmPin::new_ch4(p.PA1, OutputType::PushPull);

    let mut pwm = SimplePwm::new(
        p.TIM1,
        None,
        None,
        None,
        Some(ch4_pin),
        khz(10),
        Default::default(),
    );
    let mut ch4 = pwm.ch4();
    ch4.enable();
    ch4.set_duty_cycle_percent(20);