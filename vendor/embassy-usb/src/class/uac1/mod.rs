//! USB Audio Class 1.0 implementations.

pub use embassy_usb_upstream::class::uac1::*;

/// Fixed-rate audio capture without a feedback endpoint.
pub mod source;

mod class_codes;
