//! Reports how far the Bluetooth stack gets on this host.
//!
//! Run this first if `isdtctl scan` finds nothing. A hang at the adapter step
//! means the operating system has not granted this program Bluetooth access.

use std::time::Duration;

use isdt_charger::ble;

#[tokio::main]
async fn main() {
    match ble::adapter().await {
        Err(e) => {
            eprintln!("adapter: {e}");
            std::process::exit(1);
        }
        Ok(adapter) => {
            println!("adapter ready");
            match ble::scan(&adapter, Duration::from_secs(5)).await {
                Ok(found) if found.is_empty() => println!("no ISDT charger advertising"),
                Ok(found) => {
                    for device in found {
                        println!("{}  rssi {:?}  {}", device.label(), device.rssi, device.id);
                    }
                }
                Err(e) => eprintln!("scan: {e}"),
            }
        }
    }
}
