#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

use ch32_hal as hal;

hal::bind_interrupts!(struct Irqs {
    USB_LP_CAN1_RX0 => hal::usbd::InterruptHandler<hal::peripherals::USBD>;
});

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use super::{Irqs, hal};
    use hal::{
        adc::Adc,
        gpio::{Input, Pull},
        timer::low_level::Timer,
        usbd::Driver,
    };

    #[test]
    fn mainで使うperipheralを初期化できる() {
        let peripherals = hal::init(hal::Config::default());
        let _adc = Adc::new(peripherals.ADC1, Default::default());
        let _encoder_a = Input::new(peripherals.PA6, Pull::Up);
        let _encoder_b = Input::new(peripherals.PA7, Pull::Up);
        let _timer = Timer::new(peripherals.TIM3);
        let _usb = Driver::new(peripherals.USBD, Irqs, peripherals.PA12, peripherals.PA11);
    }
}
