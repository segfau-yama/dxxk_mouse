#![no_std]
#![no_main]

use embassy_executor::Spawner;
use panic_halt as _;

mod tasks;

#[embassy_executor::main(entry = "ch32_hal::entry")]
async fn main(spawner: Spawner) {
    let peripherals = ch32_hal::init(ch32_hal::Config {
        rcc: ch32_hal::rcc::Config::SYSCLK_FREQ_144MHZ_HSI,
        ..Default::default()
    });

    spawner.spawn(tasks::hid::hid_task(peripherals.PB0.into()).expect("failed to create HID task"));
    spawner.spawn(
        tasks::mouse::mouse_task(
            peripherals.TIM3,
            peripherals.PA6,
            peripherals.PA7,
            peripherals.ADC1,
            peripherals.PA0,
            peripherals.PA1,
            peripherals.PB6.into(),
            peripherals.PB7.into(),
        )
        .expect("failed to create mouse task"),
    );
    spawner.spawn(
        tasks::usb::usb_task(peripherals.USBD, peripherals.PA12, peripherals.PA11)
            .expect("failed to create USB task"),
    );
    spawner.spawn(
        tasks::keyboard::keyboard_task(
            peripherals.PB1.into(),
            peripherals.PA4.into(),
            peripherals.PA5.into(),
        )
        .expect("failed to create keyboard task"),
    );

    core::future::pending::<()>().await;
}
