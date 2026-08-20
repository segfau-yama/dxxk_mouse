# dxxk_mouse

ESP32-S3 上で動作する、Rust 製のマウス入力デバイス実験用リポジトリです。

現在の実装は、`esp-hal` と `embassy` を使って GPIO 入力、ADC、PCNT を扱い、ボタン、ロータリーエンコーダ、ジョイスティックの入力状態を処理します。現時点では USB HID マウスとして送信する処理ではなく、入力イベントをシリアルログへ出力する段階です。

## 対象環境

- MCU: ESP32-S3
- Rust target: `xtensa-esp32s3-none-elf`
- HAL: `esp-hal`
- async runtime: `embassy` / `esp-rtos`
- 書き込み: `espflash`
- 実機テスト: `embedded-test` + `probe-rs`

## リポジトリ構成

```text
.
├── README.md
└── dick_mouse
    ├── Cargo.toml
    ├── .cargo/config.toml
    ├── src
    │   ├── main.rs
    │   ├── lib.rs
    │   └── input
    │       ├── mod.rs
    │       ├── button.rs
    │       ├── encoder.rs
    │       └── joystick.rs
    └── tests
        ├── main.rs
        └── input
            ├── button.rs
            └── encoder.rs
```

## 実装概要

### `src/main.rs`

ESP32-S3 の peripheral を初期化し、Embassy task として以下を起動します。

| task | 用途 | GPIO / peripheral |
| --- | --- | --- |
| `scroll_wheel_task` | ロータリーエンコーダを PCNT で読み取り、スクロール量をログ出力する | `PCNT`, `GPIO11`, `GPIO12` |
| `left_button_task` | 左ボタン入力を読み取り、押下状態の変化をログ出力する | `GPIO41` |
| `right_button_task` | 右ボタン入力を読み取り、押下状態の変化をログ出力する | `GPIO42` |
| `joystick_task` | ADC でジョイスティック X/Y を読み取り、中心値からの差分をログ出力する | `ADC1`, `GPIO1`, `GPIO2` |

ボタンは active low として扱い、5 ms の debounce を設定しています。

### `src/input/button.rs`

ボタン状態を扱う入力モジュールです。

- `Button`
  - GPIO 入力を保持する
  - `active_level` と現在の安定 `level` を比較して押下状態を判定する
  - `update(now_ms)` で入力を読み直し、次の `Button` と変化有無を返す
  - debounce 中は `pending_since_ms` に変化開始時刻を保持する

### `src/input/encoder.rs`

ロータリーエンコーダのカウント状態を扱うモジュールです。

- `RotaryEncoder`
  - A/B 相の GPIO 入力を保持する
  - `stable_count` と `measured_count` を分けて debounce する
  - `detents_from(previous_count, counts_per_detent)` でクリック数へ変換する

実際の PCNT 設定は `src/main.rs` の `scroll_wheel_task` 側で行います。

### `src/input/joystick.rs`

ジョイスティックの中心値と現在値を扱う入力モジュールです。

- `Joystick`
  - 起動時に読み取った ADC 値を中心値として保持する
  - `update(raw_x, raw_y)` で中心値からの差分を持つ次の `Joystick` を返す

## ビルドと書き込み

作業ディレクトリを crate に移動します。

```sh
cd dick_mouse
```

コンパイル確認:

```sh
cargo check
```

ESP32-S3 へ書き込み:

```sh
cargo run --release
```

通常実行時の runner は `.cargo/config.toml` の `espflash flash --monitor` を使います。

## 実機テスト

このリポジトリのテストは、通常のホスト PC 上の unit test ではなく、ESP32-S3 実機上で動かす HIL テストです。

テストには `embedded-test` を使います。`tests/input/button.rs`、`tests/input/encoder.rs`、`tests/main.rs` はそれぞれ独立した ESP アプリとしてビルドされ、`probe-rs` runner で実行されます。

```sh
cargo test-hil
```

`cargo test-hil` は `.cargo/config.toml` の alias で、テスト時だけ runner を `probe-rs run --chip esp32s3` に上書きします。

通常の書き込み:

```sh
cargo run --release
```

実機テスト:

```sh
cargo test-hil
```

という使い分けを想定しています。

### probe-rs の注意点

`probe-rs` は ESP32-S3 の USB-JTAG にアクセスできる必要があります。

確認:

```sh
probe-rs list
```

通常ユーザーで `(inaccessible)` と表示される場合は、udev rule またはグループ権限の設定が不足しています。

## テスト項目

### `tests/input/button.rs`

現在の `Button` API に対して以下を確認します。

1. `Button` が `active_level`、`pending_since_ms`、`debounce_ms` を保持する
2. `Button::update(now_ms)` が次の `Button` と変化有無を返す
3. `Button::is_pressed()` が `level == active_level` と一致する

### `tests/input/encoder.rs`

現在の `RotaryEncoder` API に対して以下を確認します。

1. 初期化時に `stable_count` と `measured_count` が一致する
2. debounce 時間未満では `stable_count` が変わらない
3. debounce 時間経過後に `stable_count` が更新される
4. `update` が `measured_count` を更新する
5. `detents_from` がカウント差分をデテント数に変換する
6. `counts_per_detent` が 0 の場合は 0 を返す

## 現在の状態

- ボタン入力処理: 実装済み
- ロータリーエンコーダ入力処理: 実装済み
- ジョイスティック入力処理: 実装済み
- Embassy task による周期処理: 実装済み
- 実機 HIL テスト設定: 実装済み
- USB HID マウスとしてのレポート送信: 未実装
