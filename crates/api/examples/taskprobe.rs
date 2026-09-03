//! Sends a task-set frame several ways to find out which one a charger accepts.
//!
//! Uses the stop task, which is harmless on an idle charger.
//!
//! ```text
//! cargo run --example taskprobe -- <device-fragment> <client-id-hex>
//! ```

use std::time::Duration;

use api::ble::{self, CHAR_FFF6, CHAR_FFF7};
use api::frame;
use api::types::{BatteryKind, LinkType, TaskType};
use api::{Request, Response};
use btleplug::api::{CharPropFlags, Peripheral as _, WriteType};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let needle = args.next();
    let client_id = args.next().expect("pass the client id");
    let id = api::tokens::parse(&client_id)?;

    let adapter = ble::adapter().await?;
    let device = ble::find(&adapter, needle.as_deref(), Duration::from_secs(15)).await?;
    let peripheral = ble::peripheral_of(&device);
    if !peripheral.is_connected().await? {
        peripheral.connect().await?;
    }
    peripheral.discover_services().await?;

    let chars = peripheral.characteristics();
    for c in chars
        .iter()
        .filter(|c| [CHAR_FFF6, CHAR_FFF7].contains(&c.uuid))
        .filter(|c| c.properties.contains(CharPropFlags::NOTIFY))
    {
        peripheral.subscribe(c).await?;
    }
    let target = chars.iter().find(|c| c.uuid == CHAR_FFF6).unwrap().clone();
    let mut notes = peripheral.notifications().await?;
    println!("connected");

    // Bind first, or the charger drops us after five seconds.
    send(
        &peripheral,
        &target,
        &frame::encode(&Request::Bind { client_id: id }.data())?,
        20,
    )
    .await?;
    drain(&mut notes, Duration::from_millis(800), "bind").await;

    let stop = Request::SetTask {
        channel: 0,
        task: TaskType::Stop,
        battery: BatteryKind::LiHv,
        link: LinkType::SerialOnly,
        work_current_ma: 0,
        cell_count: 0,
        full_charged_volt_mv: 0,
    };
    let encoded = frame::encode(&stop.data())?;
    println!("frame ({}) {}", encoded.len(), hex(&encoded));

    // Vary only how the frame is split across GATT writes.
    for budget in [encoded.len(), 19, 16, 12, 8, 4, 1] {
        println!("--- writes of at most {budget} frame bytes ---");
        send(&peripheral, &target, &encoded, budget + 1).await?;
        drain(&mut notes, Duration::from_millis(1500), "task").await;
    }

    // A known-good query afterwards proves the link is still alive.
    send(&peripheral, &target, &frame::encode(&[0xE6, 0x00])?, 20).await?;
    drain(&mut notes, Duration::from_millis(1500), "work state").await;

    let _ = peripheral.disconnect().await;
    Ok(())
}

async fn send(
    p: &btleplug::platform::Peripheral,
    c: &btleplug::api::Characteristic,
    frame_bytes: &[u8],
    mtu: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let budget = mtu.saturating_sub(1).max(1);
    for chunk in frame_bytes.chunks(budget) {
        let mut packet = Vec::with_capacity(chunk.len() + 1);
        packet.push(chunk.len() as u8);
        packet.extend_from_slice(chunk);
        print!("  -> {}", hex(&packet));
        tokio::time::timeout(
            Duration::from_secs(3),
            p.write(c, &packet, WriteType::WithResponse),
        )
        .await??;
        println!();
    }
    Ok(())
}

async fn drain(
    notes: &mut (impl futures::Stream<Item = btleplug::api::ValueNotification> + Unpin),
    window: Duration,
    label: &str,
) {
    let deadline = tokio::time::Instant::now() + window;
    let mut decoder = frame::Decoder::new();
    let mut heard = false;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, notes.next()).await {
            Ok(Some(n)) => {
                heard = true;
                for data in decoder.push_notification(&n.value) {
                    println!("     <- {} {:?}", hex(&data), Response::parse(&data));
                }
            }
            _ => break,
        }
    }
    if !heard {
        println!("     ({label}: silence)");
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x} ")).collect()
}
