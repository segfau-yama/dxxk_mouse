# 音声修正の確認

対象は `dick_mouse` 本体です。USB microphone: mono / S16_LE / 48 kHz、最大98 BのAsync IN、feedback/sample-rate control/Feature Unitなし。USB speakerはmono S16のAsync OUT＋Feedback INを維持します。`bcdDevice=0.11` が今回のdescriptorです。

## 確認できたコード上の問題

- 115200 baudの長いUARTログを5秒・10秒周期でUSB/I2S処理から出していた。UARTのcritical-sectionはUSB/DMA割り込みを遅らせるため、削除してカウンタだけを保持した。
- マイク開始時に空リングから4096サンプルまで47 sample/msを送り続けていた。最初の4 packetは48 sampleの無音を即座にqueueしてRXの余裕を作り、その後は192 sampleを目安に8 packetに一度だけ47/49へ補正する。RXのdrainはUSB停止中も続ける。
- マイクのFeature Unitはmute/volume要求を受理するだけでPCMへ適用していなかった。固定mono sourceから未実装の制御を取り除き、他のインターフェース宛ての要求は次のhandlerへ渡す。
- スピーカーの`control_monitor`を捨てていた。USB mute/volumeを監視し、物理mute/volumeと合わせてTXのPCMへ適用する。
- TXのData16をData32へ変更し、S16→S32の符号・左詰め・左右複製を同時に修正。PCM5102AはData16自体も対応しており、Data16だけを音質不良の原因とは判断していない。
- Back/ForwardがREADMEのCtrlではなくAltを送信していた。modifierを0x01（Left Ctrl）へ修正。

## 現在の試作機のピン

`main.rs`の編集中だったGPIO40/41/42の順序は保持しています。

| 機能 | GPIO |
| --- | --- |
| INMP441 BCLK / WS / DIN | 5 / 6 / 7 |
| PCM5102A BCLK / WS / DOUT | 15 / 16 / 17 |
| マイクmute / スピーカーmute | 38 / 39 |
| joystick push / Back / Forward | 40 / 41 / 42 |
| mode | 3 |

ミュートは内部pull-up、押下時GNDのモーメンタリボタンを前提とします。INMP441はL/R=GND（左）に対応します。L/R=3.3 Vの実配線ならRXの`Channels::LEFT`を`Channels::RIGHT`へ合わせる必要があります。浮いたL/Rはソフトウェアで直せません。

## ソフトウェア検証

リポジトリルートから:

```sh
rustup run nightly cargo test --manifest-path dick_mouse/tests/host_audio/Cargo.toml --target x86_64-unknown-linux-gnu --locked
cd dick_mouse
rustup run esp cargo build --release --bin dick_mouse --locked
```

テストは実際のUAC descriptor生成、他interfaceへの制御要求の委譲、S16/S32の符号とmute、再録音開始・±1000 ppmのクロック差、SPSCの折り返しを確認します。USBホストコントローラやI2Sの物理信号はこのテストでは検証しません。

## 書き込み後のLinux確認

```sh
espflash flash --monitor --port /dev/ttyACM0 target/xtensa-esp32s3-none-elf/release/dick_mouse
```

ネイティブUSBを挿し直し、descriptorがmono、98 bytes、bcdDevice 0.11になったことを確認します。マイクはOS側のソフトウェア音量を使用します。

```sh
lsusb -v -d c0de:0001
cat /proc/asound/cards
arecord -l
wpctl status -n
```

調査時のこのPCでは`CARD=Microphone`はDCMTの別マイクでした。DXXKは`CARD=Audio`です。数値のPipeWire IDは再接続で変わります。

録音アプリとpavucontrolのメーターを閉じ、DXXKのPCMが使用中でない場合:

```sh
arecord -D hw:CARD=Audio,DEV=0 -f S16_LE -r 48000 -c 1 -d 10 /tmp/dxxk-check.wav
ffmpeg -i /tmp/dxxk-check.wav -af volumedetect -f null -
```

使用中ならサービスを停止せず、`wpctl status -n`のDXXK **Source名**を使って録音します。再列挙後はmono-fallback等へ変わるので、古いanalog-stereo名や数値IDを流用しないでください。

```sh
pw-record --target '<現在のDXXK Source名>' --rate 48000 --channels 1 --format s16 /tmp/dxxk-check.wav
```

10秒後にCtrl+Cで停止してください。mono S16/48 kHzの10秒分はデータ部分が960000 bytesです。開始・停止を繰り返し、44 Bのヘッダーだけになる、長い欠落、0 dBFSへの張り付きがないかを確認します。Windowsもモノラル48 kHzで同じ比較を行ってください。

スピーカーミュートは低い再生音量で、OSのmuteとGPIO39のボタンを別々に確認します。BackはCtrl+Left、ForwardはCtrl+Right、joystick pushは通常モードでPrintScreenです。ピン表と違う入力が来る場合は、実配線との対応確認が必要です。

## 参照

- [USB-IF UAC1仕様・Appendix Bの非同期マイク例](https://www.usb.org/sites/default/files/audio10.pdf)
- [使用中のEmbassy snapshotのAudioSource](https://github.com/embassy-rs/embassy/blob/cd7570483a7036f23a9925a339152396bec4c041/embassy-usb/src/class/uac1/source.rs)
- [esp-hal I2S実装](https://github.com/esp-rs/esp-hal/tree/main/esp-hal/src/i2s)
- [INMP441 datasheet](https://invensense.tdk.com/wp-content/uploads/2015/02/INMP441.pdf)
- [PCM5102A datasheet](https://www.ti.com/lit/ds/symlink/pcm5102a.pdf)
