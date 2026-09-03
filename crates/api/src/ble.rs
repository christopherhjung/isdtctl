//! The Bluetooth Low Energy backend: discovery, connection and the byte pipe.
//!
//! This is one implementation of [`Link`](crate::Link). Everything above it is
//! transport agnostic, so a charger reached over a serial bridge or a test
//! double needs only another `Link`.
//!
//! ISDT chargers advertise service `0000FFF0` and expose three characteristics
//! under it:
//!
//! | Characteristic | Role |
//! |---|---|
//! | `0000FFF6` | notify, and write on the 20 byte channel |
//! | `0000FFF7` | write on the wide channel, used once the MTU passes 140 |
//! | `0000FFF8` | write, used for the initial version handshake |
//!
//! Notifications always arrive on FFF6. This module subscribes there and
//! writes to whichever channel the caller picked.

use std::time::Duration;

use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::frame;

/// The service every ISDT charger advertises.
pub const SERVICE: Uuid = Uuid::from_u128(0x0000fff0_0000_1000_8000_00805f9b34fb);

/// Notify characteristic, and the write characteristic on the 20 byte channel.
pub const CHAR_FFF6: Uuid = Uuid::from_u128(0x0000fff6_0000_1000_8000_00805f9b34fb);

/// Write characteristic used once the MTU passes 140 bytes.
pub const CHAR_FFF7: Uuid = Uuid::from_u128(0x0000fff7_0000_1000_8000_00805f9b34fb);

/// Write characteristic used for the version handshake.
pub const CHAR_FFF8: Uuid = Uuid::from_u128(0x0000fff8_0000_1000_8000_00805f9b34fb);

/// Payload budget on the legacy channel: 20 bytes minus the length prefix.
pub const MTU_SMALL: usize = 20;

/// Payload budget the app assumes once it has negotiated the wide channel.
pub const MTU_WIDE: usize = 140;

/// Which characteristic carries host-to-charger writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteChannel {
    /// FFF6 with 20 byte writes. Every charger supports this.
    #[default]
    Narrow,
    /// FFF7 with wide writes, for chargers that negotiated a larger MTU.
    Wide,
    /// FFF8, which the app uses only for the version handshake.
    Handshake,
}

impl WriteChannel {
    fn uuid(self) -> Uuid {
        match self {
            WriteChannel::Narrow => CHAR_FFF6,
            WriteChannel::Wide => CHAR_FFF7,
            WriteChannel::Handshake => CHAR_FFF8,
        }
    }

    fn mtu(self) -> usize {
        match self {
            WriteChannel::Narrow => MTU_SMALL,
            WriteChannel::Wide | WriteChannel::Handshake => MTU_WIDE,
        }
    }
}

/// Something that went wrong on the Bluetooth link.
#[derive(Debug, thiserror::Error)]
pub enum BleError {
    /// The Bluetooth stack reported a failure.
    #[error("bluetooth error: {0}")]
    Bluetooth(#[from] btleplug::Error),

    /// The host has no usable Bluetooth adapter.
    #[error("no bluetooth adapter available")]
    NoAdapter,

    /// The operating system never handed over an adapter.
    ///
    /// On macOS this is what a missing Bluetooth permission looks like: the
    /// system stays silent rather than refusing. Grant the terminal or the
    /// application Bluetooth access under Privacy and Security.
    #[error(
        "the bluetooth stack did not respond; on macOS grant Bluetooth access to \
         this program under System Settings, Privacy and Security, Bluetooth"
    )]
    AdapterUnavailable,

    /// No charger showed up before the scan deadline.
    #[error("no ISDT charger found within {0:?}")]
    NotFound(Duration),

    /// The peripheral connected but did not expose the expected service.
    #[error("connected device does not expose characteristic {0}")]
    MissingCharacteristic(Uuid),

    /// A frame was too long for the protocol's length field.
    #[error(transparent)]
    Frame(#[from] frame::FrameError),

    /// The charger dropped the link.
    ///
    /// A charger drops any client that has not completed the bind handshake,
    /// about five seconds after it connects, so this usually means the wrong
    /// client identifier or none at all.
    #[error(
        "the charger dropped the link; it disconnects clients that have not \
         bound within about five seconds"
    )]
    LinkLost,
}

/// One charger seen while scanning.
#[derive(Debug, Clone)]
pub struct Discovered {
    /// Platform address or, on macOS, the system-assigned peripheral identifier.
    pub id: String,
    /// The advertised local name, when the charger sent one.
    pub name: Option<String>,
    /// Signal strength from the last advertisement.
    pub rssi: Option<i16>,
    peripheral: Peripheral,
}

impl Discovered {
    /// The advertised name, or the identifier when the charger sent none.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id.clone())
    }

    /// The ISDT-structured part of the advertised name, if there is one.
    pub fn isdt_name(&self) -> Option<IsdtName> {
        IsdtName::parse(self.name.as_deref()?)
    }
}

/// The structure ISDT encodes into a charger's advertised name.
///
/// From `IsdtBleProtocol.isOnBindMode`, `getType` and `getName`, which index
/// the name at fixed offsets:
///
/// ```text
/// ISDT 1 CM1620␣␣
/// ^^^^ ^ ^^^^^^^^ ^^^^^^^^
/// tag  | model    user name
///      binding mode flag
/// ```
///
/// Some units wrap this inside another string, so the tag is located rather
/// than assumed to start at offset zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsdtName {
    /// True while the charger is waiting to be bound.
    pub binding_mode: bool,
    /// The model, such as `CM1620`.
    pub model: String,
    /// The name the owner gave it.
    pub name: String,
}

impl IsdtName {
    /// Extracts the structured name, or `None` when the string has no ISDT tag
    /// or is too short to carry the fields.
    pub fn parse(advertised: &str) -> Option<Self> {
        let start = advertised.find("ISDT")?;
        let rest: Vec<char> = advertised[start..].chars().collect();
        if rest.len() < 13 {
            return None;
        }
        let binding_mode = rest[4] == '1';
        let model: String = rest[5..13].iter().collect();
        // Trailing punctuation belongs to the wrapper, not to the charger.
        let name: String = rest[13..]
            .iter()
            .collect::<String>()
            .trim_end_matches([']', ')', '>'])
            .trim()
            .to_string();
        Some(Self {
            binding_mode,
            model: model.trim().to_string(),
            name,
        })
    }
}

/// The underlying peripheral handle, for tools that need the raw GATT table.
pub fn peripheral_of(device: &Discovered) -> Peripheral {
    device.peripheral.clone()
}

/// Shortest gap the charger tolerates between frames.
///
/// These units are Bluetooth-to-serial bridges, and the app never sends two
/// packets closer together than this (`BleMng3.BLE_DO_SLEEP_TIME`). Writing
/// back to back overruns the bridge and the frame is silently dropped: a
/// CM1620 will not answer a task set sent immediately after the bind reply,
/// though it answers the identical bytes once paced.
pub const MIN_WRITE_GAP: Duration = Duration::from_millis(150);

/// How long to allow a single GATT write.
///
/// A charger that has dropped the link leaves CoreBluetooth writes pending
/// forever rather than failing them, so every write is bounded.
const WRITE_TIMEOUT: Duration = Duration::from_secs(3);

/// How long to wait for the operating system to hand over an adapter.
///
/// On macOS the Bluetooth stack simply never answers when the calling program
/// lacks Bluetooth permission, so this bounds the wait rather than hanging.
const ADAPTER_TIMEOUT: Duration = Duration::from_secs(5);

/// Returns the host's first Bluetooth adapter.
pub async fn adapter() -> Result<Adapter, BleError> {
    let manager = tokio::time::timeout(ADAPTER_TIMEOUT, Manager::new())
        .await
        .map_err(|_| BleError::AdapterUnavailable)??;
    let adapters = tokio::time::timeout(ADAPTER_TIMEOUT, manager.adapters())
        .await
        .map_err(|_| BleError::AdapterUnavailable)??;
    adapters.into_iter().next().ok_or(BleError::NoAdapter)
}

/// Scans for chargers for `duration` and returns everything advertising the
/// ISDT service.
///
/// Some units advertise the service identifier and some do not, so anything
/// whose name starts with `ISDT` is kept as well.
pub async fn scan(adapter: &Adapter, duration: Duration) -> Result<Vec<Discovered>, BleError> {
    adapter
        .start_scan(ScanFilter {
            services: vec![SERVICE],
        })
        .await?;
    tokio::time::sleep(duration).await;
    let peripherals = adapter.peripherals().await?;
    let _ = adapter.stop_scan().await;

    let mut found = Vec::new();
    for peripheral in peripherals {
        let Some(props) = peripheral.properties().await? else {
            continue;
        };
        let advertises_service = props.services.contains(&SERVICE);
        let looks_like_isdt = props
            .local_name
            .as_deref()
            .is_some_and(|n| n.to_ascii_uppercase().contains("ISDT"));
        if !advertises_service && !looks_like_isdt {
            continue;
        }
        found.push(Discovered {
            id: peripheral.id().to_string(),
            name: props.local_name.clone(),
            rssi: props.rssi,
            peripheral,
        });
    }
    found.sort_by_key(|d| std::cmp::Reverse(d.rssi.unwrap_or(i16::MIN)));
    Ok(found)
}

/// Scans until one charger matches `needle`, or until `timeout` expires.
///
/// `needle` is matched case-insensitively against both the advertised name and
/// the peripheral identifier. Pass `None` to take the strongest signal.
pub async fn find(
    adapter: &Adapter,
    needle: Option<&str>,
    timeout: Duration,
) -> Result<Discovered, BleError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let step = Duration::from_millis(1500);
    loop {
        let candidates = scan(adapter, step).await?;
        let hit = match needle {
            None => candidates.into_iter().next(),
            Some(needle) => {
                let needle = needle.to_ascii_lowercase();
                candidates.into_iter().find(|d| {
                    d.id.to_ascii_lowercase().contains(&needle)
                        || d.name
                            .as_deref()
                            .is_some_and(|n| n.to_ascii_lowercase().contains(&needle))
                })
            }
        };
        if let Some(hit) = hit {
            return Ok(hit);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(BleError::NotFound(timeout));
        }
    }
}

/// A Bluetooth Low Energy link to a charger.
pub struct BleLink {
    peripheral: Peripheral,
    write_char: Characteristic,
    write_type: WriteType,
    mtu: usize,
    frames: mpsc::UnboundedReceiver<Vec<u8>>,
    pump: tokio::task::JoinHandle<()>,
    /// When the last GATT write finished, so the next one can be paced.
    last_write: tokio::sync::Mutex<Option<tokio::time::Instant>>,
}

impl BleLink {
    /// Connects to `device`, subscribes to notifications and starts decoding.
    pub async fn connect(device: &Discovered, channel: WriteChannel) -> Result<Self, BleError> {
        let peripheral = device.peripheral.clone();
        if !peripheral.is_connected().await? {
            peripheral.connect().await?;
        }
        peripheral.discover_services().await?;

        let characteristics = peripheral.characteristics();
        if !characteristics
            .iter()
            .any(|c| c.uuid == CHAR_FFF6 && c.properties.contains(CharPropFlags::NOTIFY))
        {
            return Err(BleError::MissingCharacteristic(CHAR_FFF6));
        }

        // The app enables notifications on FFF6 and, when the charger exposes
        // it, on FFF7 as well. A CM1620 answers nothing beyond identify unless
        // both subscriptions are in place, even though every reply then
        // arrives on FFF6.
        let notify: Vec<Characteristic> = characteristics
            .iter()
            .filter(|c| {
                [CHAR_FFF6, CHAR_FFF7].contains(&c.uuid)
                    && c.properties.contains(CharPropFlags::NOTIFY)
            })
            .cloned()
            .collect();

        // Fall back to the narrow channel when the charger does not expose the
        // wide one, which is what the app does.
        let wanted = channel.uuid();
        let write_char = characteristics
            .iter()
            .find(|c| c.uuid == wanted && is_writable(c))
            .or_else(|| {
                characteristics
                    .iter()
                    .find(|c| c.uuid == CHAR_FFF6 && is_writable(c))
            })
            .cloned()
            .ok_or(BleError::MissingCharacteristic(wanted))?;

        let mtu = if write_char.uuid == CHAR_FFF6 {
            MTU_SMALL
        } else {
            channel.mtu()
        };
        let write_type = if write_char.properties.contains(CharPropFlags::WRITE) {
            WriteType::WithResponse
        } else {
            WriteType::WithoutResponse
        };

        for characteristic in &notify {
            peripheral.subscribe(characteristic).await?;
        }
        let mut notifications = peripheral.notifications().await?;

        let (tx, frames) = mpsc::unbounded_channel();
        let pump = tokio::spawn(async move {
            let mut decoder = frame::Decoder::new();
            while let Some(note) = notifications.next().await {
                if ![CHAR_FFF6, CHAR_FFF7].contains(&note.uuid) {
                    continue;
                }
                for decoded in decoder.push_notification(&note.value) {
                    if tx.send(decoded).is_err() {
                        return;
                    }
                }
            }
        });

        Ok(Self {
            peripheral,
            write_char,
            write_type,
            mtu,
            frames,
            pump,
            last_write: tokio::sync::Mutex::new(None),
        })
    }

    /// Sends one frame's `DATA` field, splitting it across writes as needed.
    ///
    /// A write to a charger that has already dropped the link never completes
    /// on macOS, so each one is bounded and reported as [`BleError::LinkLost`].
    pub async fn send_frame(&self, data: &[u8]) -> Result<(), BleError> {
        let encoded = frame::encode(data)?;
        let mut last_write = self.last_write.lock().await;
        for packet in frame::chunk(&encoded, self.mtu) {
            if let Some(previous) = *last_write {
                let elapsed = previous.elapsed();
                if elapsed < MIN_WRITE_GAP {
                    tokio::time::sleep(MIN_WRITE_GAP - elapsed).await;
                }
            }
            tokio::time::timeout(
                WRITE_TIMEOUT,
                self.peripheral
                    .write(&self.write_char, &packet, self.write_type),
            )
            .await
            .map_err(|_| BleError::LinkLost)??;
            *last_write = Some(tokio::time::Instant::now());
        }
        Ok(())
    }

    /// Waits for the next decoded frame, or `None` once the link is gone.
    pub async fn recv_frame(&mut self) -> Option<Vec<u8>> {
        self.frames.recv().await
    }

    /// Waits for the next decoded frame, giving up after `timeout`.
    pub async fn recv_frame_timeout(&mut self, timeout: Duration) -> Option<Vec<u8>> {
        tokio::time::timeout(timeout, self.frames.recv())
            .await
            .ok()
            .flatten()
    }

    /// Drops frames that are already queued, so a later read cannot return a
    /// reply to something asked before.
    pub fn drain_frames(&mut self) {
        while self.frames.try_recv().is_ok() {}
    }

    /// The payload budget in force on the write channel.
    pub fn mtu(&self) -> usize {
        self.mtu
    }

    /// Closes the link.
    pub async fn disconnect(self) -> Result<(), BleError> {
        self.pump.abort();
        self.peripheral.disconnect().await?;
        Ok(())
    }
}

impl Drop for BleLink {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

fn is_writable(c: &Characteristic) -> bool {
    c.properties
        .intersects(CharPropFlags::WRITE | CharPropFlags::WRITE_WITHOUT_RESPONSE)
}

impl From<BleError> for crate::LinkError {
    fn from(error: BleError) -> Self {
        match error {
            BleError::LinkLost => crate::LinkError::Closed,
            BleError::Frame(e) => crate::LinkError::Frame(e),
            other => crate::LinkError::transport(other),
        }
    }
}

#[async_trait::async_trait]
impl crate::Link for BleLink {
    async fn send(&self, data: &[u8]) -> Result<(), crate::LinkError> {
        Ok(self.send_frame(data).await?)
    }

    async fn recv(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, crate::LinkError> {
        // The notification pump only ends when the peripheral is gone, so a
        // closed channel is a closed link rather than an empty read.
        if self.pump.is_finished() && self.frames.is_empty() {
            return Err(crate::LinkError::Closed);
        }
        Ok(self.recv_frame_timeout(timeout).await)
    }

    fn drain(&mut self) {
        self.drain_frames();
    }

    fn max_data_len(&self) -> usize {
        crate::frame::MAX_DATA_LEN
    }
}

#[cfg(test)]
mod tests {
    use super::IsdtName;

    #[test]
    fn reads_a_charger_waiting_to_be_bound() {
        let parsed = IsdtName::parse("ISDT1CM1620  Bench").unwrap();
        assert!(parsed.binding_mode);
        assert_eq!(parsed.model, "CM1620");
        assert_eq!(parsed.name, "Bench");
    }

    /// The shape a CM1620 actually advertises: the tag is wrapped inside the
    /// Bluetooth module's own name rather than starting at offset zero.
    #[test]
    fn reads_a_tag_wrapped_in_another_string() {
        let parsed = IsdtName::parse("Phy BLE-Uart [ISDT0CM1620  Bench]").unwrap();
        assert!(!parsed.binding_mode);
        assert_eq!(parsed.model, "CM1620");
        assert_eq!(parsed.name, "Bench");
    }

    #[test]
    fn ignores_a_name_with_no_tag_or_too_little_of_one() {
        assert_eq!(IsdtName::parse("Some Headphones"), None);
        assert_eq!(IsdtName::parse("ISDT1CM"), None);
    }
}
