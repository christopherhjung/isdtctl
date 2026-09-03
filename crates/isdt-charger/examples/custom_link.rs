//! Drives a charger over a link this crate knows nothing about.
//!
//! The link here is a stub that answers from a table, which makes the example
//! runnable without hardware. Swap the body of `send` and `recv` for a serial
//! port, a TCP bridge or a recorded capture and every typed command below
//! keeps working unchanged.
//!
//! ```text
//! cargo run --example custom_link
//! ```

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use isdt_charger::link::{Link, LinkError};
use isdt_charger::{frame, BatteryKind, Client, LinkType, Request, TaskType};

/// A charger simulated well enough to answer the calls below.
struct FakeCharger {
    /// Frames waiting to be read, `DATA` field only.
    ///
    /// `Link::send` takes `&self`, because a real transport hands bytes to a
    /// socket or a Bluetooth stack that is already shared. Anything an
    /// implementation needs to mutate there lives behind a lock or a channel.
    outbox: Mutex<VecDeque<Vec<u8>>>,
}

impl FakeCharger {
    fn new() -> Self {
        Self {
            outbox: Mutex::new(VecDeque::new()),
        }
    }

    /// Builds the reply a real charger would send for a request.
    fn reply_to(data: &[u8]) -> Option<Vec<u8>> {
        match data.first()? {
            // Bind: accepted.
            0x18 => Some(vec![0x19, 0x00]),
            // Work state: standby, 4S LiPo, both leads, no faults.
            0xE6 => {
                let mut frame = vec![0xE7, data[1], 0x00, 0x00];
                frame.extend_from_slice(&0u32.to_le_bytes()); // mAh
                frame.extend_from_slice(&0u32.to_le_bytes()); // mWh
                frame.extend_from_slice(&0u32.to_le_bytes()); // elapsed ms
                frame.extend_from_slice(&[0x01, 0x04, 0x03]); // LiPo, 4S, both
                frame.extend_from_slice(&4200u16.to_le_bytes());
                frame.extend_from_slice(&1000u32.to_le_bytes());
                frame.extend_from_slice(&1u16.to_le_bytes());
                frame.extend_from_slice(&0u16.to_le_bytes());
                frame.extend_from_slice(&11_000u16.to_le_bytes());
                frame.extend_from_slice(&1_050_000u32.to_le_bytes());
                frame.extend_from_slice(&0u16.to_le_bytes()); // no faults
                Some(frame)
            }
            // Task set: accepted.
            0xEA => Some(vec![0xEB, data[1], 0x00]),
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl Link for FakeCharger {
    async fn send(&self, data: &[u8]) -> Result<(), LinkError> {
        // A real link would frame and write here. Proving the framing layer is
        // available to any implementation:
        let on_air = frame::encode(data)?;
        println!("  -> {}", hex(&on_air));

        // The stub answers into its own outbox. A real link would let replies
        // arrive on the transport's receive path instead.
        if let Some(reply) = Self::reply_to(data) {
            self.outbox.lock().unwrap().push_back(reply);
        }
        Ok(())
    }

    async fn recv(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>, LinkError> {
        Ok(self.outbox.lock().unwrap().pop_front())
    }

    fn drain(&mut self) {
        self.outbox.lock().unwrap().clear();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut charger = Client::new(FakeCharger::new());

    println!("bind");
    charger.bind([0x11; 16]).await?;

    println!("work state");
    let state = charger.work_state(0).await?;
    println!(
        "  {} {} {}S, faults: {}",
        state.state.label(),
        state.battery_type.label(),
        state.cell_count,
        state
            .errors
            .map(|f| f.to_string())
            .unwrap_or_else(|| "not reported".into())
    );

    println!("start a charge");
    charger
        .start_task(
            0,
            TaskType::Charge,
            BatteryKind::LiPo,
            LinkType::SerialOnly,
            1000,
            4,
            4200,
        )
        .await?;
    println!("  accepted");

    // Anything the crate does not model still goes out as raw bytes.
    charger.send(&Request::Raw { data: vec![0xE4, 0x00] }).await?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x} ")).collect()
}
