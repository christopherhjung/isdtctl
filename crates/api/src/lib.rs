//! Talk to ISDT battery chargers.
//!
//! This crate is the backend: the wire protocol, a Bluetooth Low Energy
//! transport, and a client that ties them together. The `cli` and `gui` crates
//! are built on this one and have no privileged access; anything they do, you
//! can do.
//!
//! The protocol was reconstructed from the vendor's Android application and
//! verified against a CM1620. Where the application leaves something
//! undefined, the documentation says so rather than guessing.
//!
//! # The short version
//!
//! ```no_run
//! use std::time::Duration;
//! use api::{BatteryKind, Client, LinkType, TaskType};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! // A charger drops any client that has not bound, five seconds in, so the
//! // identifier is not optional. See the `bind` section below.
//! let client_id = api::tokens::parse("9102782c5bfb5047a4533d071feb6eca")?;
//! let mut charger =
//!     Client::discover_bound(None, Duration::from_secs(10), Default::default(), client_id)
//!         .await?;
//!
//! let state = charger.work_state(0).await?;
//! println!("{} at {}%", state.state.label(), state.capacity_percent);
//!
//! charger
//!     .start_task(
//!         0,
//!         TaskType::Charge,
//!         BatteryKind::LiPo,
//!         LinkType::SerialOnly,
//!         2000,
//!         4,
//!         4200,
//!     )
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Layers, and where to cut in
//!
//! Each layer is usable on its own, so take as much or as little as you need.
//!
//! | Layer | Use it when |
//! |---|---|
//! | [`frame`] | You have your own bytes and want framing, stuffing and checksums |
//! | [`Request`] and [`Response`] | You have your own link and want the packet layer |
//! | [`Link`] | You have a serial port or a bridge and want the whole client |
//! | [`Client`] | You want a charger on Bluetooth and no ceremony |
//!
//! ## Bytes only
//!
//! The packet layer does no I/O and needs no runtime.
//!
//! ```
//! use api::{frame, Request, Response};
//!
//! let bytes = Request::WorkState { channel: 0 }.encode().unwrap();
//! assert_eq!(bytes, [0xAA, 0x12, 0x02, 0xE6, 0x00, 0xFA]);
//!
//! let mut decoder = frame::Decoder::new();
//! for data in decoder.push_notification(&[0x05, 0xAA, 0x21, 0x02, 0xEB, 0x00, 0x0E]) {
//!     if let Some(Response::TaskAck { error_code, .. }) = Response::parse(&data) {
//!         assert_eq!(error_code, 0, "the charger accepted the task");
//!     }
//! }
//! ```
//!
//! ## Your own transport
//!
//! [`Client`] is generic over [`Link`] and knows nothing about Bluetooth.
//! Implement the trait and every typed command works over your carrier. See
//! the [`link`] module for a worked example.
//!
//! # Binding is not optional
//!
//! A charger disconnects any client that has not bound, about five seconds
//! after connecting, and answers almost nothing in the meantime. Binding sends
//! a 16 byte identifier you invent; the charger stores it and expects the same
//! one on every later connection. It cannot tell you the identifier back, so
//! keep it. [`tokens`] handles generating, parsing and storing them.
//!
//! # Writes carry real consequences
//!
//! [`Client::start_task`] puts current through a battery. The calibration and
//! firmware calls change persistent device state, and an interrupted firmware
//! write can leave a charger unbootable. Nothing here second-guesses a
//! request. The bounds the vendor application enforces are exposed in
//! [`types`] as [`types::WORK_CURRENT_MA`] and friends, but they are advisory.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod client;
pub mod frame;
pub mod link;
pub mod request;
pub mod response;
pub mod types;

#[cfg(feature = "ble")]
pub mod ble;
#[cfg(feature = "ble")]
pub mod tokens;

pub use client::{Client, ClientError, Telemetry};
pub use frame::{Decoder, FrameError};
pub use link::{Link, LinkError};
pub use request::{Request, WRITE_APP_BLOCK};
pub use response::Response;
pub use types::{
    BatteryKind, CalibrationMode, ChannelState, ChargerState, ErrorFlags, LinkType, PowerType,
    TaskType,
};

#[cfg(feature = "ble")]
pub use ble::{BleError, BleLink, Discovered, IsdtName, WriteChannel};
#[cfg(feature = "ble")]
pub use tokens::ClientId;

/// A charger reached over Bluetooth Low Energy, which is the usual case.
#[cfg(feature = "ble")]
pub type BleClient = Client<BleLink>;
