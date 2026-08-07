#![allow(non_snake_case)]

use dick_mouse::button::{Button, Led, Toggle};
use esp_hal::gpio::Level;

#[test]
fn T01_active_lowの安定値がLowなら押下になる() {
    let button = Button::new(Level::Low, Level::Low, Level::Low, 0, 5);

    assert!(button.is_pressed());
    assert!(!button.is_released());
}

#[test]
fn T02_チャタリング時間未満では押下状態に変わらない() {
    let button = Button::new(Level::High, Level::High, Level::Low, 100, 5);

    let candidate = button.update(Level::Low, 101);
    let still_released = candidate.update(Level::Low, 105);

    assert!(still_released.is_released());
    assert_eq!(still_released.level(), Level::High);
}

#[test]
fn T03_チャタリング時間経過後に押下状態へ変わる() {
    let button = Button::new(Level::High, Level::High, Level::Low, 100, 5);

    let candidate = button.update(Level::Low, 101);
    let pressed = candidate.update(Level::Low, 106);

    assert!(pressed.is_pressed());
    assert_eq!(pressed.level(), Level::Low);
}

#[test]
fn T04_updateは元のButtonを変更せず新しいButtonを返す() {
    let button = Button::new(Level::High, Level::High, Level::Low, 100, 5);

    let next_button = button.update(Level::Low, 101);
    let pressed = next_button.update(Level::Low, 106);

    assert_eq!(button.level(), Level::High);
    assert_eq!(next_button.level(), Level::High);
    assert_eq!(pressed.level(), Level::Low);
}

#[test]
fn T05_debounceが0なら実測値をすぐに反映する() {
    let button = Button::new(Level::High, Level::High, Level::Low, 100, 0);

    let pressed = button.update(Level::Low, 101);

    assert!(pressed.is_pressed());
    assert_eq!(pressed.level(), Level::Low);
}

#[test]
fn T06_Toggleは押下エッジで一度だけ切り替わる() {
    let released = Button::new(Level::High, Level::High, Level::Low, 0, 5);
    let pressed = Button::new(Level::Low, Level::Low, Level::Low, 0, 5);

    let toggle = Toggle::new(false, released.is_pressed());
    let toggled = toggle.update(pressed);
    let unchanged = toggled.update(pressed);

    assert!(toggled.is_on());
    assert!(unchanged.is_on());
}

#[test]
fn T07_LedはButtonの押下状態から出力Levelを更新する() {
    let pressed = Button::new(Level::Low, Level::Low, Level::Low, 0, 5);
    let released = Button::new(Level::High, Level::High, Level::Low, 0, 5);
    let led = Led::new(Level::Low, Level::High);

    let led_on = led.update_with_button(pressed);
    let led_off = led_on.update_with_button(released);

    assert_eq!(led_on.level(), Level::High);
    assert_eq!(led_off.level(), Level::Low);
}
