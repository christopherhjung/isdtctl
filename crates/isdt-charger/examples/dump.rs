//! Connects to a charger, enumerates its GATT table, subscribes to everything
//! that can notify, and sends probe frames on each writable characteristic.
//!
//! Prints every byte that comes back, tagged with the characteristic it
//! arrived on. Use this when a request times out and you need to know whether
//! the charger is silent, answering somewhere unexpected, or refusing until
//! it has been bound.
//!
//! ```text
//! cargo run --example dump -- <device-fragment> [client-id-hex]
//! ```

use std::time::Duration;

use btleplug::api::{
    Central, CentralEvent, CharPropFlags, Characteristic, Peripheral as _, WriteType,
};
use futures::StreamExt;
use isdt_charger::ble::{self, CHAR_FFF6, CHAR_FFF7, CHAR_FFF8};
use isdt_charger::frame;
use isdt_charger::{Request, Response};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let needle = args.next();
    let client_id = args.next();

    let adapter = ble::adapter().await?;
    println!("scanning...");
    let device = ble::find(&adapter, needle.as_deref(), Duration::from_secs(15)).await?;
    println!("found {} ({})", device.label(), device.id);

    // Watch the adapter's own event stream so a disconnect is visible rather
    // than showing up later as a hang.
    let mut events = adapter.events().await?;
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            match event {
                CentralEvent::DeviceDisconnected(id) => {
                    println!("!! central event: disconnected {id:?}")
                }
                CentralEvent::DeviceConnected(id) => {
                    println!("!! central event: connected {id:?}")
                }
                CentralEvent::StateUpdate(state) => {
                    println!("!! central event: state {state:?}")
                }
                _ => {}
            }
        }
        println!("!! central event stream ended");
    });

    // Reach past the library's own connect so nothing is filtered out.
    let peripheral = ble::peripheral_of(&device);
    if !peripheral.is_connected().await? {
        peripheral.connect().await?;
    }
    peripheral.discover_services().await?;
    println!("connected\n");

    let characteristics: Vec<Characteristic> = peripheral.characteristics().into_iter().collect();
    println!("GATT table");
    for c in &characteristics {
        println!(
            "  service {}  char {}  {}",
            short(&c.service_uuid.to_string()),
            short(&c.uuid.to_string()),
            properties(c.properties)
        );
    }
    println!();

    // Subscribe to everything that can notify, not just the one the app uses.
    let notifiable: Vec<Characteristic> = characteristics
        .iter()
        .filter(|c| {
            c.properties
                .intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE)
        })
        .cloned()
        .collect();
    for c in &notifiable {
        match peripheral.subscribe(c).await {
            Ok(()) => println!("subscribed to {}", short(&c.uuid.to_string())),
            Err(e) => println!("subscribe to {} failed: {e}", short(&c.uuid.to_string())),
        }
    }
    let mut notifications = peripheral.notifications().await?;
    println!();

    let writable: Vec<Characteristic> = characteristics
        .iter()
        .filter(|c| {
            c.properties
                .intersects(CharPropFlags::WRITE | CharPropFlags::WRITE_WITHOUT_RESPONSE)
                && if std::env::var("DUMP_ALL_CHARS").is_ok() {
                    [CHAR_FFF6, CHAR_FFF7, CHAR_FFF8].contains(&c.uuid)
                } else {
                    c.uuid == CHAR_FFF6
                }
        })
        .cloned()
        .collect();

    let mut probes: Vec<(String, Request)> = Vec::new();
    if let Some(hex) = client_id {
        match parse_client_id(&hex) {
            Ok(id) => probes.push(("bind".into(), Request::Bind { client_id: id })),
            Err(e) => println!("ignoring client id: {e}\n"),
        }
    }
    // Read-only queries. None of these change device state.
    probes.push(("identify".into(), Request::Identify));
    probes.push(("hardware info".into(), Request::HardwareInfo));
    probes.push(("limits".into(), Request::LimitParameters));
    probes.push(("electrical".into(), Request::Electrical { channel: 0 }));
    probes.push(("work state".into(), Request::WorkState { channel: 0 }));
    probes.push(("temperature".into(), Request::Temperature { channel: 0 }));
    probes.push(("resistance".into(), Request::InnerResistance { channel: 0 }));
    probes.push(("one-key launch".into(), Request::OneKeyLaunch));
    probes.push(("identify again".into(), Request::Identify));

    for target in &writable {
        let name = short(&target.uuid.to_string());
        let write_type = if target.properties.contains(CharPropFlags::WRITE) {
            WriteType::WithResponse
        } else {
            WriteType::WithoutResponse
        };
        println!("=== writing on {name} ({write_type:?}) ===");

        for (label, request) in &probes {
            let encoded = frame::encode(&request.data())?;
            let packets = frame::chunk(&encoded, 20);
            print!("  -> {label:<14} ");
            for packet in &packets {
                print!("{}", hex_of(packet));
            }
            println!();

            println!(
                "     link up before write: {:?}",
                peripheral.is_connected().await
            );
            let mut wrote = true;
            for packet in &packets {
                // A dropped link makes CoreBluetooth writes hang rather than
                // fail, so bound the wait.
                match tokio::time::timeout(
                    Duration::from_secs(3),
                    peripheral.write(target, packet, write_type),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        println!("     write failed: {e}");
                        wrote = false;
                        break;
                    }
                    Err(_) => {
                        println!("     write timed out (link is probably gone)");
                        wrote = false;
                        break;
                    }
                }
            }
            if !wrote {
                continue;
            }

            // Collect everything that arrives in the next two seconds.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            let mut decoder = frame::Decoder::new();
            let mut heard = false;
            loop {
                let left = deadline.saturating_duration_since(tokio::time::Instant::now());
                if left.is_zero() {
                    break;
                }
                match tokio::time::timeout(left, notifications.next()).await {
                    Ok(Some(note)) => {
                        heard = true;
                        println!(
                            "     <- {} {}",
                            short(&note.uuid.to_string()),
                            hex_of(&note.value)
                        );
                        for data in decoder.push_notification(&note.value) {
                            println!("        frame {}", hex_of(&data));
                            match Response::parse(&data) {
                                Some(r) => println!("        parsed {r:?}"),
                                None => println!("        parsed nothing"),
                            }
                        }
                    }
                    Ok(None) => {
                        println!("     notification stream ended");
                        break;
                    }
                    Err(_) => break,
                }
            }
            if !heard {
                println!("     (silence)");
            }
        }
        println!();
    }

    println!("connected at end: {}", peripheral.is_connected().await?);
    let _ = peripheral.disconnect().await;
    Ok(())
}

fn parse_client_id(hex: &str) -> Result<[u8; 16], String> {
    let cleaned: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if cleaned.len() < 32 {
        return Err(format!("need 32 hex digits, got {}", cleaned.len()));
    }
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

fn properties(p: CharPropFlags) -> String {
    let mut parts = Vec::new();
    for (flag, name) in [
        (CharPropFlags::READ, "read"),
        (CharPropFlags::WRITE, "write"),
        (CharPropFlags::WRITE_WITHOUT_RESPONSE, "write-no-rsp"),
        (CharPropFlags::NOTIFY, "notify"),
        (CharPropFlags::INDICATE, "indicate"),
    ] {
        if p.contains(flag) {
            parts.push(name);
        }
    }
    parts.join(" ")
}

fn short(uuid: &str) -> String {
    uuid.split('-').next().unwrap_or(uuid).to_string()
}

fn hex_of(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
