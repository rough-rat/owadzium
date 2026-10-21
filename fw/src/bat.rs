use defmt::*;
use py32_hal::adc::{Adc, SampleTime};
use py32_hal::peripherals::ADC;
use py32_hal::{adc, bind_interrupts};

bind_interrupts!(struct Irqs {
    ADC_COMP => adc::InterruptHandler<ADC>;
});

pub async fn measure_vcc_mv(p_adc: ADC) -> u32 {
    let mut adc = Adc::new(p_adc, Irqs);
    adc.set_sample_time(SampleTime::CYCLES71_5);

    let mut vrefint = adc.enable_vref();
    let vrefint_sample: u32 = adc.read(&mut vrefint).await.into();
    let vcc: u32 = 1200 * 4095 / vrefint_sample;

    info!("battery: {} - {} mV", vrefint_sample, vcc);
    vcc
}
