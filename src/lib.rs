//! A Bluetooth Low Energy client for ISDT chargers.
//!
//! This crate speaks the serial protocol ISDT's Android app uses over BLE,
//! reimplemented from the app's own packet classes. It targets the CM1620 and
//! the other chargers on the same protocol, and it exposes every command and
//! query the app can send, including several the app's CM1620 screens never
//! reach.
//!
//! # Layers
//!
//! * [`frame`] carries the link layer: sync byte, address, length, byte
//!   stuffing, checksum and the length-prefixed GATT chunking.
//! * [`request`] and [`response`] carry the packet layer, one type per packet
//!   class in the app.
//! * [`transport`] does discovery, connection and the byte pipe.
//! * [`Client`] ties them together into request and reply with timeouts,
//!   plus a telemetry poller.
//!
//! # Getting started
//!
//! ```no_run
//! use std::time::Duration;
//! use isdt_charger::{Client, Request};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let mut client = Client::discover(None, Duration::from_secs(10)).await?;
//! let info = client.hardware_info().await?;
//! println!("{} firmware {:?}", info.device_id_string(), info.firmware_version);
//!
//! let state = client.work_state(0).await?;
//! println!("{} at {}%", state.state.label(), state.capacity_percent);
//! # Ok(())
//! # }
//! ```
//!
//! # Safety of writes
//!
//! [`Request::SetTask`] starts real current through a real battery, and
//! [`Request::Calibrate6`], [`Request::Calibrate8`], [`Request::EraseApp`] and
//! [`Request::WriteApp`] change persistent device state. Nothing here
//! second-guesses a request, so validate parameters before sending. The
//! bounds the app enforces are in [`types`] as `WORK_CURRENT_MA` and friends.

#![warn(missing_docs)]

pub mod client;
pub mod frame;
pub mod request;
pub mod response;
pub mod tokens;
pub mod transport;
pub mod types;

pub use client::{Client, ClientError, Telemetry};
pub use request::Request;
pub use response::Response;
pub use tokens::{ClientId, Store as TokenStore};
pub use transport::{Discovered, WriteChannel};
pub use types::{
    BatteryKind, ChannelState, ChargerState, ErrorFlags, LinkType, PowerType, TaskType,
};
