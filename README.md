# dxxk_mouse

ESP32-S3 上で動作する、Rust 製のマウス入力デバイス実験用リポジトリです。

現在の実装は、`esp-hal` と `embassy` を使って GPIO 入力と PCNT を扱い、ボタンとロータリーエンコーダの入力状態を処理します。現時点では USB HID マウスとして送信する処理ではなく、入力イベントをシリアルログへ出力する段階です。

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
    │       └── encoder.rs
    └── tests
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

ボタンは active low として扱い、5 ms の debounce を設定しています。

### `src/input/button.rs`

ボタン、トグル、LED 状態を扱う入力モジュールです。

- `Button`
  - GPIO 入力を保持する
  - `active_level` と現在の安定 `level` を比較して押下状態を判定する
  - `update(now_ms)` で入力を読み直し、次の `Button` と変化有無を返す
  - debounce 中は `pending_since_ms` に変化開始時刻を保持する

- `Toggle`
  - ボタンの押下エッジで ON/OFF を切り替える
  - `was_pressed` に前回の押下状態を保持する

- `Led`
  - bool、`Button`、`Toggle` の状態から出力 level を決定する

### `src/input/encoder.rs`

ロータリーエンコーダのカウント状態を扱うモジュールです。

- `RotaryEncoder`
  - A/B 相の GPIO 入力を保持する
  - `stable_count` と `measured_count` を分けて debounce する
  - `delta_from(previous_count)` で差分を返す
  - `detents_from(previous_count, counts_per_detent)` でクリック数へ変換する

実際の PCNT 設定は `src/main.rs` の `scroll_wheel_task` 側で行います。

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

テストには `embedded-test` を使います。`tests/button.rs` と `tests/encoder.rs` はそれぞれ独立した ESP アプリとしてビルドされ、`probe-rs` runner で実行されます。

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

### `tests/button.rs`

現在の `Button` / `Toggle` / `Led` API に対して以下を確認します。

1. `Button` が `active_level`、`pending_since_ms`、`debounce_ms` を保持する
2. `Button::update(now_ms)` が次の `Button` と変化有無を返す
3. `Button::is_pressed()` が `level == active_level` と一致する
4. `Toggle` が初期状態を保持する
5. `Toggle` が押下エッジで ON/OFF を切り替える
6. `Led` が bool から出力 level を更新する
7. `Led` が `Toggle` の状態から出力 level を更新する

### `tests/encoder.rs`

現在の `RotaryEncoder` API に対して以下を確認します。

1. 初期化時に `stable_count` と `measured_count` が一致する
2. debounce 時間未満では `stable_count` が変わらない
3. debounce 時間経過後に `stable_count` が更新される
4. `update` が `measured_count` を更新する
5. `delta_from` が安定カウントとの差分を返す
6. `detents_from` がカウント差分をデテント数に変換する
7. `counts_per_detent` が 0 の場合は 0 を返す

## 現在の状態

- ボタン入力処理: 実装済み
- ロータリーエンコーダ入力処理: 実装済み
- Embassy task による周期処理: 実装済み
- 実機 HIL テスト設定: 実装済み
- USB HID マウスとしてのレポート送信: 未実装
