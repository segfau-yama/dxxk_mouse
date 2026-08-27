use ch32_hal::{
    Peri, bind_interrupts, peripherals,
    usbd::{Driver as UsbDriver, InterruptHandler},
};
use embassy_futures::join::join;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_usb::{
    Builder as UsbBuilder, Config as UsbConfig,
    class::hid::{Config as UsbHidConfig, HidWriter, State as UsbHidState},
};
use usbd_hid::descriptor::{KeyboardReport, MouseReport};

bind_interrupts!(struct Irqs {
    USB_LP_CAN1_RX0 => InterruptHandler<peripherals::USBD>;
});

pub(crate) const USB_HID_POLL_MS: u8 = 10;
const USB_HID_REPORT_BYTES: usize = 9;
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 256;
const USB_BOS_DESCRIPTOR_SIZE: usize = 64;
const USB_CONTROL_BUFFER_SIZE: usize = 64;
const USB_KEYBOARD_REPORT_ID: u8 = 1;
const USB_MOUSE_REPORT_ID: u8 = 2;
const USB_KEYBOARD_MOUSE_REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00,
    0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x19, 0x00, 0x29, 0xff, 0x26, 0xff, 0x00, 0x75,
    0x08, 0x95, 0x01, 0x81, 0x03, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x25, 0x01, 0x75, 0x01, 0x95,
    0x05, 0x91, 0x02, 0x95, 0x03, 0x91, 0x03, 0x05, 0x07, 0x19, 0x00, 0x29, 0xdd, 0x26, 0xff, 0x00,
    0x75, 0x08, 0x95, 0x06, 0x81, 0x00, 0xc0, 0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x02, 0x09,
    0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29, 0x08, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95,
    0x08, 0x81, 0x02, 0x05, 0x01, 0x09, 0x30, 0x17, 0x81, 0xff, 0xff, 0xff, 0x25, 0x7f, 0x75, 0x08,
    0x95, 0x01, 0x81, 0x06, 0x09, 0x31, 0x81, 0x06, 0x09, 0x38, 0x81, 0x06, 0x05, 0x0c, 0x0a, 0x38,
    0x02, 0x81, 0x06, 0xc0, 0xc0,
];

pub(crate) enum UsbHidReport {
    Keyboard(KeyboardReport),
    Mouse(MouseReport),
}

pub(crate) static USB_HID_REPORTS: Channel<CriticalSectionRawMutex, UsbHidReport, 4> =
    Channel::new();
#[embassy_executor::task]
pub(crate) async fn usb_task(
    usbd: Peri<'static, peripherals::USBD>,
    dp: Peri<'static, peripherals::PA12>,
    dm: Peri<'static, peripherals::PA11>,
) {
    let driver = UsbDriver::new(usbd, Irqs, dp, dm);
    let mut config_descriptor = [0; USB_CONFIG_DESCRIPTOR_SIZE];
    let mut bos_descriptor = [0; USB_BOS_DESCRIPTOR_SIZE];
    let mut control_buffer = [0; USB_CONTROL_BUFFER_SIZE];
    let mut hid_state = UsbHidState::new();

    let mut config = UsbConfig::new(0xc0de, 0x0001);
    config.manufacturer = Some("dick mouse");
    config.product = Some("DXXK Mouse");
    config.serial_number = Some("0001");

    let mut builder = UsbBuilder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [],
        &mut control_buffer,
    );

    let mut hid_writer = HidWriter::<_, USB_HID_REPORT_BYTES>::new(
        &mut builder,
        &mut hid_state,
        UsbHidConfig {
            report_descriptor: USB_KEYBOARD_MOUSE_REPORT_DESCRIPTOR,
            request_handler: None,
            poll_ms: USB_HID_POLL_MS,
            max_packet_size: USB_HID_REPORT_BYTES as u16,
        },
    );
    let mut device = builder.build();

    join(device.run(), async move {
        loop {
            hid_writer.ready().await;

            loop {
                match USB_HID_REPORTS.receive().await {
                    UsbHidReport::Keyboard(report) => {
                        let bytes = [
                            USB_KEYBOARD_REPORT_ID,
                            report.modifier,
                            report.reserved,
                            report.keycodes[0],
                            report.keycodes[1],
                            report.keycodes[2],
                            report.keycodes[3],
                            report.keycodes[4],
                            report.keycodes[5],
                        ];

                        if hid_writer.write(&bytes).await.is_err() {
                            break;
                        }
                    }
                    UsbHidReport::Mouse(report) => {
                        let bytes = [
                            USB_MOUSE_REPORT_ID,
                            report.buttons,
                            report.x as u8,
                            report.y as u8,
                            report.wheel as u8,
                            report.pan as u8,
                        ];

                        if hid_writer.write(&bytes).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    })
    .await;
}
