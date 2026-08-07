#![allow(non_snake_case)]

use dick_mouse::encoder::RotaryEncoder;

#[test]
fn T01_初期化時は安定カウントと実測カウントが一致する() {
    let encoder = RotaryEncoder::initial(12, 100, 2);

    assert_eq!(encoder.stable_count(), 12);
    assert_eq!(encoder.measured_count(), 12);
    assert!(!encoder.is_chattering());
}

#[test]
fn T02_チャタリング時間未満では安定カウントが変わらない() {
    let encoder = RotaryEncoder::initial(0, 100, 2);

    let candidate = encoder.update(3, 101);
    let still_stable = candidate.update(3, 102);

    assert_eq!(still_stable.stable_count(), 0);
    assert_eq!(still_stable.measured_count(), 3);
    assert!(still_stable.is_chattering());
}

#[test]
fn T03_チャタリング時間経過後に安定カウントが変わる() {
    let encoder = RotaryEncoder::initial(0, 100, 2);

    let candidate = encoder.update(3, 101);
    let stable = candidate.update(3, 103);

    assert_eq!(stable.stable_count(), 3);
    assert_eq!(stable.measured_count(), 3);
    assert!(!stable.is_chattering());
}

#[test]
fn T04_updateは元のRotaryEncoderを変更せず新しいインスタンスを返す() {
    let encoder = RotaryEncoder::initial(0, 100, 2);

    let next_encoder = encoder.update(3, 101);

    assert_eq!(encoder.stable_count(), 0);
    assert_eq!(encoder.measured_count(), 0);
    assert_eq!(next_encoder.stable_count(), 0);
    assert_eq!(next_encoder.measured_count(), 3);
}

#[test]
fn T05_delta_fromは安定カウントとの差分を返す() {
    let encoder = RotaryEncoder::new(12, 12, 100, 2);

    assert_eq!(encoder.delta_from(8), 4);
}

#[test]
fn T06_detents_fromは分解能でクリック数に変換する() {
    let encoder = RotaryEncoder::new(16, 16, 100, 2);

    assert_eq!(encoder.detents_from(8, 4), 2);
}

#[test]
fn T07_counts_per_detentが0ならデテント数は0になる() {
    let encoder = RotaryEncoder::new(16, 16, 100, 2);

    assert_eq!(encoder.detents_from(8, 0), 0);
}
