//! Reusable client for Liene Pearl N200 Pro photo printers.
//!
//! [`printer::Printer`] owns a Bluetooth SPP connection and speaks the
//! Hannto frame protocol. [`image_prep`] turns arbitrary photos into the
//! 2x3 JPEG the printer expects.

pub mod collage;
pub mod daemon;
pub mod edit;
pub mod image_prep;
pub mod pairing;
pub mod printer;

pub use daemon::{CollageRequest, Command, Event, PrintRequest};
pub use edit::{NormalizedRect, PhotoEdit};
pub use image_prep::{prepare_jpeg, Fit};
pub use printer::{
    ble_scan, ble_services, list_devices, Printer, INFO_PROPS, SPP_UUID, STATUS_PROPS,
};
