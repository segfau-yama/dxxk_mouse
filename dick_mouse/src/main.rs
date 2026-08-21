#![no_std]
#![no_main]

use dick_mouse::input::{Button, Joystick, RotaryEncoder};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    gpio::{AnyPin, Input, InputConfig, Level, Pin, Pull},
    interrupt::software::SoftwareInterruptControl,
    pcnt::{Pcnt, channel, unit::Unit},
    peripherals::{ADC1, GPIO1, GPIO2},
    time::Instant,
    timer::timg::TimerGroup,
};

esp_bootloader_esp_idf::esp_app_desc!();

const JOYSTICK_LOG_DELTA: u16 = 64;

fn configure_encoder_unit<const NUM: usize>(
    unit: &Unit<'static, NUM>,
    input_a: &Input<'static>,
    input_b: &Input<'static>,
) {
    let signal_a = input_a.peripheral_input();
    let signal_b = input_b.peripheral_input();

    unit.set_filter(Some(800)).expect("invalid pcnt filter");

    let ch0 = &unit.channel0;
    ch0.set_ctrl_signal(signal_a.clone());
    ch0.set_edge_signal(signal_b.clone());
    ch0.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    ch0.set_input_mode(channel::EdgeMode::Increment, channel::EdgeMode::Decrement);

    let ch1 = &unit.channel1;
    ch1.set_ctrl_signal(signal_b.clone());
    ch1.set_edge_signal(signal_a.clone());
    ch1.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    ch1.set_input_mode(channel::EdgeMode::Decrement, channel::EdgeMode::Increment);
}

fn setup_encoder<const NUM: usize>(
    unit: &Unit<'static, NUM>,
    gpio_a: AnyPin<'static>,
    gpio_b: AnyPin<'static>,
) -> (Input<'static>, Input<'static>, RotaryEncoder, i32) {
    let input_a = Input::new(gpio_a, InputConfig::default().with_pull(Pull::Up));
    let input_b = Input::new(gpio_b, InputConfig::default().with_pull(Pull::Up));
    configure_encoder_unit(unit, &input_a, &input_b);

    let count = unit.value() as i32;
    let now_ms = Instant::now().duration_since_epoch().as_millis();
    (
        input_a,
        input_b,
        RotaryEncoder::new(count, now_ms, 2),
        count,
    )
}

fn encoder_detents<const NUM: usize>(
    unit: &Unit<'static, NUM>,
    encoder: &mut RotaryEncoder,
    reported_count: &mut i32,
) -> i32 {
    let now_ms = Instant::now().duration_since_epoch().as_millis();
    *encoder = (*encoder).update(unit.value() as i32, now_ms);

    let detents = encoder.detents_from(*reported_count, 4);
    if detents != 0 {
        *reported_count = (*reported_count).saturating_add(detents.saturating_mul(4));
    }

    detents
}

#[embassy_executor::task]
async fn scroll_task(
    unit: Unit<'static, 0>,
    gpio_a: AnyPin<'static>,
    gpio_b: AnyPin<'static>,
) {
    let (_input_a, _input_b, mut encoder, mut reported_count) =
        setup_encoder(&unit, gpio_a, gpio_b);

    loop {
        let detents = encoder_detents(&unit, &mut encoder, &mut reported_count);

        if detents != 0 {
            esp_println::println!("scroll encoder detents: {}", detents);
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
async fn microphone_volume_task(
    unit: Unit<'static, 1>,
    gpio_a: AnyPin<'static>,
    gpio_b: AnyPin<'static>,
) {
    let (_input_a, _input_b, mut encoder, mut reported_count) =
        setup_encoder(&unit, gpio_a, gpio_b);

    loop {
        let detents = encoder_detents(&unit, &mut encoder, &mut reported_count);

        if detents != 0 {
            esp_println::println!("microphone volume encoder detents: {}", detents);
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
async fn speaker_volume_task(
    unit: Unit<'static, 2>,
    gpio_a: AnyPin<'static>,
    gpio_b: AnyPin<'static>,
) {
    let (_input_a, _input_b, mut encoder, mut reported_count) =
        setup_encoder(&unit, gpio_a, gpio_b);

    loop {
        let detents = encoder_detents(&unit, &mut encoder, &mut reported_count);

        if detents != 0 {
            esp_println::println!("speaker volume encoder detents: {}", detents);
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task(pool_size = 11)]
async fn button_task(label: &'static str, gpio: AnyPin<'static>) {
    let input = Input::new(gpio, InputConfig::default().with_pull(Pull::Up));
    let mut button = Button::new(input.level(), Level::Low, 5);

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        let (next_button, changed) = button.update(input.level(), now_ms);
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

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    esp_println::println!("Init!");

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let pcnt = Pcnt::new(peripherals.PCNT);

    spawner.spawn(
        scroll_task(
            pcnt.unit0,
            peripherals.GPIO11.degrade(),
            peripherals.GPIO12.degrade(),
        )
        .expect("failed to create scroll encoder task"),
    );
    spawner.spawn(
        microphone_volume_task(
            pcnt.unit1,
            peripherals.GPIO13.degrade(),
            peripherals.GPIO14.degrade(),
        )
        .expect("failed to create microphone volume encoder task"),
    );
    spawner.spawn(
        speaker_volume_task(
            pcnt.unit2,
            peripherals.GPIO15.degrade(),
            peripherals.GPIO16.degrade(),
        )
        .expect("failed to create speaker volume encoder task"),
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
