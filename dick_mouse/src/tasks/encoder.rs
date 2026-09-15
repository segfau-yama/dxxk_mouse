use crate::device::RotaryEncoder;
use esp_hal::{
    gpio::{AnyPin, Input, InputConfig, Pull},
    pcnt::{channel, unit::Unit},
    time::Instant,
};

const COUNTS_PER_DETENT: i32 = 4;

pub(crate) fn setup<const NUM: usize>(
    unit: &Unit<'static, NUM>,
    gpio_a: AnyPin<'static>,
    gpio_b: AnyPin<'static>,
) -> (RotaryEncoder, i32) {
    let input_a = Input::new(gpio_a, InputConfig::default().with_pull(Pull::Up));
    let input_b = Input::new(gpio_b, InputConfig::default().with_pull(Pull::Up));
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

    let count = unit.value() as i32;
    let now_ms = Instant::now().duration_since_epoch().as_millis();
    (RotaryEncoder::new(count, now_ms, 2), count)
}

pub(crate) fn detents<const NUM: usize>(
    unit: &Unit<'static, NUM>,
    encoder: &mut RotaryEncoder,
    reported_count: &mut i32,
    now_ms: u64,
) -> i32 {
    *encoder = encoder.update(unit.value() as i32, now_ms);
    let detents = encoder.stable_count().saturating_sub(*reported_count) / COUNTS_PER_DETENT;
    if detents != 0 {
        *reported_count = reported_count.saturating_add(detents.saturating_mul(COUNTS_PER_DETENT));
    }
    detents
}
