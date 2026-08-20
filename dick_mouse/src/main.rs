#![no_std]
#![no_main]

use dick_mouse::input::{Button, Joystick, RotaryEncoder};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    gpio::{AnyPin, Level, Pin},
    interrupt::software::SoftwareInterruptControl,
    pcnt::{Pcnt, channel},
    peripherals::{ADC1, GPIO1, GPIO2, PCNT},
    time::Instant,
    timer::timg::TimerGroup,
};

esp_bootloader_esp_idf::esp_app_desc!();

const JOYSTICK_LOG_DELTA: u16 = 64;

#[embassy_executor::task(pool_size = 3)]
async fn encoder_task(
    label: &'static str,
    pcnt: PCNT<'static>,
    gpio_a: AnyPin<'static>,
    gpio_b: AnyPin<'static>,
) {
    let pcnt = Pcnt::new(pcnt);
    let unit = pcnt.unit0;
    unit.set_filter(Some(800)).expect("invalid pcnt filter");

    let now_ms = Instant::now().duration_since_epoch().as_millis();
    let mut encoder = RotaryEncoder::initial(gpio_a, gpio_b, unit.value() as i32, now_ms, 2);
    let (input_a, input_b, stable_count, _, _, _) = encoder.values();
    let signal_a = input_a.peripheral_input();
    let signal_b = input_b.peripheral_input();

    let ch0 = &unit.channel0;
    ch0.set_ctrl_signal(signal_a.clone());
    ch0.set_edge_signal(signal_b.clone());
    ch0.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    ch0.set_input_mode(channel::EdgeMode::Increment, channel::EdgeMode::Decrement);

    let ch1 = &unit.channel1;
    ch1.set_ctrl_signal(signal_b);
    ch1.set_edge_signal(signal_a);
    ch1.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    ch1.set_input_mode(channel::EdgeMode::Decrement, channel::EdgeMode::Increment);

    let mut reported_count = stable_count;

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        encoder = encoder.update(unit.value() as i32, now_ms);

        let detents = encoder.detents_from(reported_count, 4);
        if detents != 0 {
            reported_count = reported_count.saturating_add(detents.saturating_mul(4));
            esp_println::println!("{} encoder detents: {}", label, detents);
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task(pool_size = 11)]
async fn button_task(label: &'static str, gpio: AnyPin<'static>) {
    let mut button = Button::new(gpio, Level::Low, 5);

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        let (next_button, changed) = button.update(now_ms);
        button = next_button;

        if changed {
            esp_println::println!("{} button pressed: {}", label, button.is_pressed());
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
async fn joystick_task(adc: ADC1<'static>, gpio_x: GPIO1<'static>, gpio_y: GPIO2<'static>) {
    let mut adc_config = AdcConfig::new();
    let mut x_pin = adc_config.enable_pin(gpio_x, Attenuation::_11dB);
    let mut y_pin = adc_config.enable_pin(gpio_y, Attenuation::_11dB);
    let mut adc = Adc::new(adc, adc_config);
    let center_x = adc.read_blocking(&mut x_pin);
    let center_y = adc.read_blocking(&mut y_pin);
    let mut joystick = Joystick::new(center_x, center_y);
    let mut reported_joystick = joystick;

    esp_println::println!("joystick center x: {}, y: {}", center_x, center_y);

    loop {
        joystick = joystick.update(adc.read_blocking(&mut x_pin), adc.read_blocking(&mut y_pin));

        if reported_joystick.x().abs_diff(joystick.x()) >= JOYSTICK_LOG_DELTA
            || reported_joystick.y().abs_diff(joystick.y()) >= JOYSTICK_LOG_DELTA
        {
            esp_println::println!("joystick x: {}, y: {}", joystick.x(), joystick.y());
            reported_joystick = joystick;
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
async fn keyboard_task() {
    core::future::pending::<()>().await;
}

#[embassy_executor::task]
async fn mouse_task() {
    core::future::pending::<()>().await;
}

#[embassy_executor::task]
async fn audio_input_task() {
    core::future::pending::<()>().await;
}

#[embassy_executor::task]
async fn audio_output_task() {
    core::future::pending::<()>().await;
}

#[embassy_executor::task]
async fn usb_task() {
    core::future::pending::<()>().await;
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    esp_println::println!("Init!");

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    spawner.spawn(
        encoder_task(
            "scroll",
            peripherals.PCNT,
            peripherals.GPIO11.degrade(),
            peripherals.GPIO12.degrade(),
        )
        .expect("failed to create scroll encoder task"),
    );
    spawner.spawn(
        joystick_task(peripherals.ADC1, peripherals.GPIO1, peripherals.GPIO2)
            .expect("failed to create joystick task"),
    );
    spawner.spawn(
        button_task("left", peripherals.GPIO41.degrade())
            .expect("failed to create left button task"),
    );
    spawner.spawn(
        button_task("right", peripherals.GPIO42.degrade())
            .expect("failed to create right button task"),
    );

    core::future::pending::<()>().await;
}
