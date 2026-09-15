#![no_std]
//! Embassy USB with the project's fixed-rate UAC1 capture implementation.

pub use embassy_usb_upstream::*;

/// USB class implementations.
pub mod class;
