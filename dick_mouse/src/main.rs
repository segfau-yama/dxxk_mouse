#![no_std]
#![no_main]

use dick_mouse::{
    input::{Button, Joystick, RotaryEncoder},
    usb::audio::{Microphone, Speaker},
    usb::hid::{KeyboardReport, MouseReport},
};
use embassy_executor::Spawner;
use embassy_futures::join::{join, join5};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use embassy_usb::{
    Builder as UsbBuilder, Config as UsbConfig,
    class::{
        hid::{
            Config as UsbHidConfig, HidBootProtocol, HidSubclass, HidWriter, State as UsbHidState,
        },
        uac1::{
            Channel as UsbAudioChannel, FeedbackRefresh, SampleWidth,
            source::{
                AudioSource as UsbMicrophoneClass,
                AudioSourceControlHandler as UsbMicrophoneControlHandler,
            },
            speaker::{Speaker as UsbSpeakerClass, State as UsbSpeakerState},
        },
    },
};
use esp_backtrace as _;
use esp_hal::{
    Async,
    analog::adc::{Adc, AdcConfig, Attenuation},
    gpio::{AnyPin, Input, InputConfig, Level, Pin, Pull},
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s, I2sRx, I2sTx},
    interrupt::software::SoftwareInterruptControl,
    otg_fs::{
        Usb,
        asynch::{Config as UsbDriverConfig, Driver as UsbDriver},
    },
    pcnt::{Pcnt, channel, unit::Unit},
    peripherals::{ADC1, GPIO1, GPIO2},
    time::{Instant, Rate},
    timer::timg::TimerGroup,
};
use static_cell::StaticCell;
use usbd_hid::descriptor::SerializedDescriptor;

esp_bootloader_esp_idf::esp_app_desc!();

const JOYSTICK_LOG_DELTA: u16 = 64;
const AUDIO_FRAME_SAMPLES: usize = 48;
const AUDIO_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i16>();
const USB_HID_POLL_MS: u8 = 10;
const USB_KEYBOARD_REPORT_BYTES: usize = 8;
const USB_MOUSE_REPORT_BYTES: usize = 5;
const USB_AUDIO_MAX_PACKET_BYTES: usize = AUDIO_FRAME_BYTES * 2;
const USB_AUDIO_FEEDBACK_48K: [u8; 3] = [0x00, 0x00, 0x0c];
const USB_EP_OUT_BUFFER_SIZE: usize = 256;
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 64;

type AudioFrame = [i16; AUDIO_FRAME_SAMPLES];

static MICROPHONE_AUDIO: Channel<CriticalSectionRawMutex, AudioFrame, 2> = Channel::new();
static SPEAKER_AUDIO: Channel<CriticalSectionRawMutex, AudioFrame, 2> = Channel::new();
static USB_KEYBOARD_REPORTS: Channel<CriticalSectionRawMutex, KeyboardReport, 4> = Channel::new();
static USB_MOUSE_REPORTS: Channel<CriticalSectionRawMutex, MouseReport, 4> = Channel::new();
const USB_AUDIO_SAMPLE_RATES: &[u32] = &[48_000];
const USB_AUDIO_CHANNELS: &[UsbAudioChannel] = &[UsbAudioChannel::LeftFront];
static USB_EP_OUT_BUFFER: StaticCell<[u8; USB_EP_OUT_BUFFER_SIZE]> = StaticCell::new();
static USB_CONFIG_DESCRIPTOR: StaticCell<[u8; USB_CONFIG_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_BOS_DESCRIPTOR: StaticCell<[u8; USB_BOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_MSOS_DESCRIPTOR: StaticCell<[u8; USB_MSOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_CONTROL_BUFFER: StaticCell<[u8; USB_CONTROL_BUFFER_SIZE]> = StaticCell::new();
static USB_KEYBOARD_HID_STATE: StaticCell<UsbHidState<'static>> = StaticCell::new();
static USB_MOUSE_HID_STATE: StaticCell<UsbHidState<'static>> = StaticCell::new();
static USB_MICROPHONE_HANDLER: StaticCell<UsbMicrophoneControlHandler> = StaticCell::new();
static USB_SPEAKER_STATE: StaticCell<UsbSpeakerState<'static>> = StaticCell::new();

fn setup_encoder<const NUM: usize>(
    unit: &Unit<'static, NUM>,
    gpio_a: AnyPin<'static>,
    gpio_b: AnyPin<'static>,
) -> (Input<'static>, Input<'static>, RotaryEncoder, i32) {
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

fn bytes_to_audio_frame(bytes: &[u8]) -> AudioFrame {
    let mut frame = [0; AUDIO_FRAME_SAMPLES];

    for (sample, chunk) in frame.iter_mut().zip(bytes.chunks_exact(2)) {
        *sample = i16::from_le_bytes([chunk[0], chunk[1]]);
    }

    frame
}

#[embassy_executor::task]
async fn scroll_task(unit: Unit<'static, 0>, gpio_a: AnyPin<'static>, gpio_b: AnyPin<'static>) {
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

#[embassy_executor::task]
async fn microphone_task(mut i2s_rx: I2sRx<'static, Async>) {
    let mut microphone = Microphone::new([0; AUDIO_FRAME_SAMPLES]);

    loop {
        let mut bytes = [0; AUDIO_FRAME_BYTES];

        if i2s_rx.read_dma_async(&mut bytes).await.is_ok() {
            microphone = microphone.update(bytes_to_audio_frame(&bytes));
            MICROPHONE_AUDIO.send(*microphone.buffer()).await;
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
async fn usb_task(usb: Usb<'static>) {
    let driver = UsbDriver::new(
        usb,
        USB_EP_OUT_BUFFER.init([0; USB_EP_OUT_BUFFER_SIZE]),
        UsbDriverConfig::default(),
    );

    let mut config = UsbConfig::new(0xc0de, 0x0001);
    config.manufacturer = Some("dick mouse");
    config.product = Some("DXXK USB Audio");
    config.serial_number = Some("0001");

    let mut builder = UsbBuilder::new(
        driver,
        config,
        USB_CONFIG_DESCRIPTOR.init([0; USB_CONFIG_DESCRIPTOR_SIZE]),
        USB_BOS_DESCRIPTOR.init([0; USB_BOS_DESCRIPTOR_SIZE]),
        USB_MSOS_DESCRIPTOR.init([0; USB_MSOS_DESCRIPTOR_SIZE]),
        USB_CONTROL_BUFFER.init([0; USB_CONTROL_BUFFER_SIZE]),
    );

    let microphone = UsbMicrophoneClass::new(
        &mut builder,
        USB_AUDIO_SAMPLE_RATES,
        SampleWidth::Width2Byte,
        FeedbackRefresh::Period32Frames as u8,
        None,
    );
    // ponytail: git AudioSource assumes interfaces 0/1; remove this ordering constraint when upstream uses stored interface numbers.
    builder.handler(USB_MICROPHONE_HANDLER.init(microphone.handler));

    let speaker = UsbSpeakerClass::new(
        &mut builder,
        USB_SPEAKER_STATE.init(UsbSpeakerState::new()),
        USB_AUDIO_MAX_PACKET_BYTES as u16,
        SampleWidth::Width2Byte,
        USB_AUDIO_SAMPLE_RATES,
        USB_AUDIO_CHANNELS,
        FeedbackRefresh::Period32Frames,
    );
    let keyboard_writer = HidWriter::<_, USB_KEYBOARD_REPORT_BYTES>::new(
        &mut builder,
        USB_KEYBOARD_HID_STATE.init(UsbHidState::new()),
        UsbHidConfig {
            report_descriptor: KeyboardReport::desc(),
            request_handler: None,
            poll_ms: USB_HID_POLL_MS,
            max_packet_size: USB_KEYBOARD_REPORT_BYTES as u16,
            hid_subclass: HidSubclass::Boot,
            hid_boot_protocol: HidBootProtocol::Keyboard,
        },
    );
    let mouse_writer = HidWriter::<_, USB_MOUSE_REPORT_BYTES>::new(
        &mut builder,
        USB_MOUSE_HID_STATE.init(UsbHidState::new()),
        UsbHidConfig {
            report_descriptor: MouseReport::desc(),
            request_handler: None,
            poll_ms: USB_HID_POLL_MS,
            max_packet_size: USB_MOUSE_REPORT_BYTES as u16,
            hid_subclass: HidSubclass::Boot,
            hid_boot_protocol: HidBootProtocol::Mouse,
        },
    );
    let mut device = builder.build();
    let mut speaker_stream = speaker.stream;
    let mut speaker_feedback = speaker.feedback;
    let mut microphone_audio = microphone.audio_ep_in;
    let mut microphone_feedback = microphone.feedback_ep_in;

    join(
        device.run(),
        join5(
            async move {
                loop {
                    speaker_stream.wait_connection().await;

                    loop {
                        let mut packet = [0; USB_AUDIO_MAX_PACKET_BYTES];

                        match speaker_stream.read_packet(&mut packet).await {
                            Ok(size) if size > 0 => {
                                SPEAKER_AUDIO
                                    .send(bytes_to_audio_frame(&packet[..size]))
                                    .await;
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                }
            },
            async move {
                loop {
                    speaker_feedback.wait_connection().await;

                    while speaker_feedback
                        .write_packet(&USB_AUDIO_FEEDBACK_48K)
                        .await
                        .is_ok()
                    {
                        Timer::after(Duration::from_millis(
                            FeedbackRefresh::Period32Frames.frame_count() as u64,
                        ))
                        .await;
                    }
                }
            },
            async move {
                loop {
                    microphone_audio.wait_enabled().await;

                    loop {
                        let frame = MICROPHONE_AUDIO.receive().await;
                        let mut bytes = [0; USB_AUDIO_MAX_PACKET_BYTES];

                        for (sample, chunk) in frame.iter().zip(bytes.chunks_exact_mut(4)) {
                            let sample = sample.to_le_bytes();
                            chunk[..2].copy_from_slice(&sample);
                            chunk[2..].copy_from_slice(&sample);
                        }

                        if microphone_audio.write(&bytes).await.is_err() {
                            break;
                        }
                    }
                }
            },
            async move {
                loop {
                    microphone_feedback.wait_enabled().await;

                    while microphone_feedback
                        .write(&USB_AUDIO_FEEDBACK_48K)
                        .await
                        .is_ok()
                    {
                        Timer::after(Duration::from_millis(
                            FeedbackRefresh::Period32Frames.frame_count() as u64,
                        ))
                        .await;
                    }
                }
            },
            join(
                async move {
                    loop {
                        keyboard_writer.ready().await;

                        loop {
                            let report = USB_KEYBOARD_REPORTS.receive().await;

                            if keyboard_writer.write_serialize(&report).await.is_err() {
                                break;
                            }
                        }
                    }
                },
                async move {
                    loop {
                        mouse_writer.ready().await;

                        loop {
                            let report = USB_MOUSE_REPORTS.receive().await;

                            if mouse_writer.write_serialize(&report).await.is_err() {
                                break;
                            }
                        }
                    }
                },
            ),
        ),
    )
    .await;
}

#[embassy_executor::task]
async fn speaker_task(mut i2s_tx: I2sTx<'static, Async>) {
    let mut speaker = Speaker::new([0; AUDIO_FRAME_SAMPLES]);

    loop {
        let pc_frame = SPEAKER_AUDIO.receive().await;
        speaker = speaker.update(pc_frame);
        let mut bytes = [0; AUDIO_FRAME_BYTES];

        for (sample, chunk) in speaker.buffer().iter().zip(bytes.chunks_exact_mut(2)) {
            chunk.copy_from_slice(&sample.to_le_bytes());
        }

        let _ = i2s_tx.write_dma_async(&mut bytes).await;
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
    let (rx_descriptors, tx_descriptors) =
        esp_hal::dma_descriptors!(AUDIO_FRAME_BYTES, AUDIO_FRAME_BYTES);
    let i2s = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH0,
        I2sConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(48_000))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::MONO),
    )
    .expect("failed to create I2S")
    .into_async();
    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(peripherals.GPIO17)
        .with_ws(peripherals.GPIO18)
        .with_din(peripherals.GPIO8)
        .build(rx_descriptors);
    let i2s_tx = i2s
        .i2s_tx
        .with_bclk(peripherals.GPIO21)
        .with_ws(peripherals.GPIO38)
        .with_dout(peripherals.GPIO9)
        .build(tx_descriptors);
    let usb = Usb::new(peripherals.USB0, peripherals.GPIO20, peripherals.GPIO19);

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
    spawner.spawn(microphone_task(i2s_rx).expect("failed to create microphone task"));
    spawner.spawn(usb_task(usb).expect("failed to create usb task"));
    spawner.spawn(speaker_task(i2s_tx).expect("failed to create speaker task"));
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
