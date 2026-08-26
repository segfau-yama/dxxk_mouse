#![no_std]
#![no_main]

use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::{
    gpio::Pin, interrupt::software::SoftwareInterruptControl, otg_fs::Usb, pcnt::Pcnt,
    timer::timg::TimerGroup,
};

esp_bootloader_esp_idf::esp_app_desc!();

mod tasks;

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let pcnt = Pcnt::new(peripherals.PCNT);
    let usb = Usb::new(peripherals.USB0, peripherals.GPIO20, peripherals.GPIO19);

    spawner.spawn(
        tasks::hid::hid_task(peripherals.GPIO21.degrade()).expect("failed to create HID task"),
    );
    spawner.spawn(
        tasks::mouse::mouse_task(
            pcnt.unit0,
            peripherals.GPIO11.degrade(),
            peripherals.GPIO12.degrade(),
            peripherals.ADC1,
            peripherals.GPIO1,
            peripherals.GPIO2,
            peripherals.GPIO38.degrade(),
            peripherals.GPIO39.degrade(),
        )
        .expect("failed to create mouse task"),
    );
    spawner.spawn(tasks::usb::usb_task(usb).expect("failed to create usb task"));
    spawner.spawn(
        tasks::keyboard::keyboard_task(
            peripherals.GPIO42.degrade(),
            peripherals.GPIO6.degrade(),
            peripherals.GPIO7.degrade(),
        )
        .expect("failed to create keyboard task"),
    );

    core::future::pending::<()>().await;
}
