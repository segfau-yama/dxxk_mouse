# DESIGN

## 基本方針

device struct は入力状態を表す immutable state として扱います。
task が peripheral を所有し、GPIO / ADC / PCNT / I2S から読んだ値で device struct を `update` します。
task 間の受け渡しは Embassy の `Channel` に限定します。

## Struct

| Struct | 役割 | 主な利用元 |
| --- | --- | --- |
| `Button` | debounce 済みの押下状態 | `hid_task`, `keyboard_task`, `mouse_task`, `microphone_task`, `speaker_task` |
| `RotaryEncoder` | PCNT count の安定化 | `mouse_task`, `microphone_task`, `speaker_task` |
| `Joystick` | ADC中心値からのX/Y差分 | `mouse_task` |
| `KeyboardInput` | キーボード系入力のsnapshot | `keyboard_task` → `hid_task` |
| `MouseInput` | マウス系入力のsnapshot | `mouse_task` → `hid_task` |
| `UsbHidReport` | USB HIDへ送るkeyboard/mouse report | `hid_task` → `usb_task` |
| `Microphone` | I2S RX frame | `microphone_task` |
| `Speaker` | USB speaker frame | `speaker_task` |

## Channel

| Channel | Payload | Producer | Consumer |
| --- | --- | --- | --- |
| `KEYBOARD_REPORTS` | `KeyboardInput` | `keyboard_task` | `hid_task` |
| `MOUSE_REPORTS` | `MouseInput` | `mouse_task` | `hid_task` |
| `USB_HID_REPORTS` | `UsbHidReport` | `hid_task` | `usb_task` |
| `MICROPHONE_FRAMES` | `AudioFrame` | `microphone_task` | `usb_task` |
| `SPEAKER_FRAMES` | `AudioFrame` | `usb_task` | `speaker_task` |

## HID sequence

```mermaid
sequenceDiagram
    participant K as keyboard_task
    participant M as mouse_task
    participant H as hid_task
    participant U as usb_task
    participant Host as USB host
    participant KB as KEYBOARD_REPORTS
    participant MS as MOUSE_REPORTS
    participant UH as USB_HID_REPORTS
    participant Button
    participant Encoder as RotaryEncoder
    participant Joy as Joystick

    loop every USB_HID_POLL_MS
        K->>Button: update(GPIO level, now_ms)
        K->>KB: send(KeyboardInput)

        M->>Encoder: update(PCNT count, now_ms)
        M->>Button: update(left/right GPIO level, now_ms)
        M->>Joy: update(ADC x/y)
        M->>MS: send(MouseInput)

        H->>Button: update(mode GPIO level, now_ms)
        H->>KB: try_receive()
        H->>MS: try_receive()
        H->>H: keyboard_report(...) / mouse_report(...)
        H->>UH: send(UsbHidReport)

        U->>UH: receive()
        U->>Host: HidWriter.write(report bytes)
    end
```

`hid_task` が mouse mode / game mode の変換境界です。
`keyboard_task` と `mouse_task` は物理入力を snapshot にするだけで、USB report の最終形は `hid_task` で決めます。

## Audio sequence

```mermaid
sequenceDiagram
    participant Mic as microphone_task
    participant Spk as speaker_task
    participant U as usb_task
    participant Host as USB host
    participant MF as MICROPHONE_FRAMES
    participant SF as SPEAKER_FRAMES
    participant Button
    participant Encoder as RotaryEncoder
    participant Microphone
    participant Speaker

    loop microphone input
        Mic->>Button: update(mute GPIO level, now_ms)
        Mic->>Encoder: update(volume PCNT count, now_ms)
        Mic->>Mic: read_dma_async(I2S RX)
        Mic->>Microphone: new(frame)
        Mic->>MF: send(AudioFrame)
        U->>MF: receive()
        U->>Host: microphone_audio.write(bytes)
    end

    loop speaker output
        Host->>U: speaker_stream.read_packet(bytes)
        U->>SF: send(AudioFrame)
        Spk->>SF: receive()
        Spk->>Button: update(mute GPIO level, now_ms)
        Spk->>Encoder: update(volume PCNT count, now_ms)
        Spk->>Speaker: new(frame)
        Spk->>Spk: write_dma_async(I2S TX)
    end
```

Audio は `usb_task` を境界にして、microphone は device → host、speaker は host → device の向きで流れます。
