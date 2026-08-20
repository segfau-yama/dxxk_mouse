#![no_std]
#![no_main]

use dick_mouse::{button::Button, encoder::RotaryEncoder, input::Joystick};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    gpio::{AnyPin, Input, InputConfig, Level, Pin, Pull},
    interrupt::software::SoftwareInterruptControl,
    pcnt::{Pcnt, channel},
    peripherals::{ADC1, GPIO1, GPIO2, PCNT},
    timer::timg::TimerGroup,
};

esp_bootloader_esp_idf::esp_app_desc!();

const JOYSTICK_CENTER: i32 = 2048;
const JOYSTICK_LOG_DELTA: u16 = 64;

fn joystick_axis(raw: u16) -> i16 {
    i32::from(raw)
        .saturating_sub(JOYSTICK_CENTER)
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

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

    let input_config = InputConfig::default().with_pull(Pull::Up);
    let input_a = Input::new(gpio_a, input_config);
    let input_b = Input::new(gpio_b, input_config);
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

    let now_ms = embassy_time::Instant::now().as_millis();
    let mut encoder = RotaryEncoder::new(unit.value() as i32, now_ms, 2);
    let mut reported_count = encoder.stable_count();

    loop {
        let now_ms = embassy_time::Instant::now().as_millis();
        encoder = encoder.update(unit.value() as i32, now_ms);

        let detents = encoder.stable_count().saturating_sub(reported_count) / 4;
        if detents != 0 {
            reported_count = reported_count.saturating_add(detents.saturating_mul(4));
            esp_println::println!("{} encoder detents: {}", label, detents);
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task(pool_size = 11)]
async fn button_task(label: &'static str, input: Input<'static>) {
    let mut button = Button::new(
        input.level(),
        Level::Low,
        embassy_time::Instant::now().as_millis(),
        5,
    );

    loop {
        let now_ms = embassy_time::Instant::now().as_millis();
        let next_button = button.update(input.level(), now_ms);

        if next_button.is_pressed() != button.is_pressed() {
            esp_println::println!("{} button pressed: {}", label, next_button.is_pressed());
        }

        button = next_button;
        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
async fn joystick_task(adc: ADC1<'static>, gpio_x: GPIO1<'static>, gpio_y: GPIO2<'static>) {
    let mut adc_config = AdcConfig::new();
    let mut x_pin = adc_config.enable_pin(gpio_x, Attenuation::_11dB);
    let mut y_pin = adc_config.enable_pin(gpio_y, Attenuation::_11dB);
    let mut adc = Adc::new(adc, adc_config);
    let mut reported_joystick = Joystick::new(0, 0);

    loop {
        let joystick = Joystick::new(
            joystick_axis(adc.read_blocking(&mut x_pin)),
            joystick_axis(adc.read_blocking(&mut y_pin)),
        );

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

    spawner
        .spawn(encoder_task(
            "scroll",
            peripherals.PCNT,
            peripherals.GPIO11.degrade(),
            peripherals.GPIO12.degrade(),
        ))
        .expect("failed to spawn scroll encoder task");
    spawner
        .spawn(joystick_task(
            peripherals.ADC1,
            peripherals.GPIO1,
            peripherals.GPIO2,
        ))
        .expect("failed to spawn joystick task");
    spawner
        .spawn(button_task(
            "left",
            Input::new(
                peripherals.GPIO41,
                InputConfig::default().with_pull(Pull::Up),
            ),
        ))
        .expect("failed to spawn left button task");
    spawner
        .spawn(button_task(
            "right",
            Input::new(
                peripherals.GPIO42,
                InputConfig::default().with_pull(Pull::Up),
            ),
        ))
        .expect("failed to spawn right button task");

    core::future::pending::<()>().await;
}
