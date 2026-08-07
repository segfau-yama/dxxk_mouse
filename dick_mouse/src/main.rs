#![no_std]
#![no_main]

use dick_mouse::{button::Button, encoder::RotaryEncoder};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, InputConfig, Level, Pull},
    interrupt::software::SoftwareInterruptControl,
    pcnt::{Pcnt, channel},
    peripherals::{GPIO11, GPIO12, GPIO41, GPIO42, PCNT},
    time::Instant,
    timer::timg::TimerGroup,
};

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn run() {
    loop {
        esp_println::println!("Hello world from embassy!");
        Timer::after(Duration::from_millis(1_000)).await;
    }
}

#[embassy_executor::task]
async fn scroll_wheel_task(pcnt: PCNT<'static>, gpio_a: GPIO11<'static>, gpio_b: GPIO12<'static>) {
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

    let now_ms = Instant::now().duration_since_epoch().as_millis();
    let mut encoder = RotaryEncoder::initial(unit.value() as i32, now_ms, 2);
    let mut reported_count = encoder.stable_count();

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        encoder = encoder.update(unit.value() as i32, now_ms);

        let detents = encoder.detents_from(reported_count, 4);
        if detents != 0 {
            reported_count = reported_count.saturating_add(detents.saturating_mul(4));
            esp_println::println!("scroll wheel detents: {}", detents);
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
async fn left_button_task(gpio: GPIO41<'static>) {
    let input = Input::new(gpio, InputConfig::default().with_pull(Pull::Up));
    let now_ms = Instant::now().duration_since_epoch().as_millis();
    let mut button = Button::new(input.level(), input.level(), Level::Low, now_ms, 5);

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        let next_button = button.update(input.level(), now_ms);

        if next_button.is_pressed() != button.is_pressed() {
            esp_println::println!("left button pressed: {}", next_button.is_pressed());
        }

        button = next_button;
        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
async fn right_button_task(gpio: GPIO42<'static>) {
    let input = Input::new(gpio, InputConfig::default().with_pull(Pull::Up));
    let now_ms = Instant::now().duration_since_epoch().as_millis();
    let mut button = Button::new(input.level(), input.level(), Level::Low, now_ms, 5);

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        let next_button = button.update(input.level(), now_ms);

        if next_button.is_pressed() != button.is_pressed() {
            esp_println::println!("right button pressed: {}", next_button.is_pressed());
        }

        button = next_button;
        Timer::after(Duration::from_millis(1)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    esp_println::println!("Init!");

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    spawner.spawn(run()).expect("failed to spawn run task");
    spawner
        .spawn(scroll_wheel_task(
            peripherals.PCNT,
            peripherals.GPIO11,
            peripherals.GPIO12,
        ))
        .expect("failed to spawn scroll wheel task");
    spawner
        .spawn(left_button_task(peripherals.GPIO41))
        .expect("failed to spawn left button task");
    spawner
        .spawn(right_button_task(peripherals.GPIO42))
        .expect("failed to spawn right button task");

    loop {
        esp_println::println!("Bing!");
        Timer::after(Duration::from_millis(5_000)).await;
    }
}
