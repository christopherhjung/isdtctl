//! Everything a charger can say back.
//!
//! Each variant of [`Response`] mirrors one `IsdtPack*` parser class from
//! `com.isdt.hubin.isdtapp.ble`. Field layouts, widths and signedness follow
//! those parsers exactly.
//!
//! Replies carry the request's command word plus one. A charger also sends
//! [`Response::Electrical`], [`Response::WorkState`] and friends unprompted
//! once the host starts polling, so treat the notification stream as a feed
//! rather than a strict request and reply exchange.

use serde::{Deserialize, Serialize};

use crate::types::{BatteryKind, ChannelState, ChargerState, ErrorFlags, PowerType};

/// A decoded frame from the charger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    /// Answer to [`crate::Request::Identify`]: region code and part number.
    Identity(Identity),

    /// A block of logged chart samples for one channel.
    LineChart(LineChart),

    /// Bluetooth mode, pairing mode and how long the link has been up.
    ///
    /// The app's dispatch table gives command word 0x13 to a speed-controller
    /// packet instead, so a charger that answers 0x13 is decoded here and an
    /// electronic speed controller on the same command word is not.
    BleMode(BleMode),

    /// Answer to [`crate::Request::Bind`].
    Bind {
        /// True when the charger accepted the binding.
        bound: bool,
    },

    /// Identity of the BattGo pack on a channel.
    BattgoInfo(BattgoInfo),

    /// Manufacturer profile of the BattGo pack on a channel.
    BattgoOem(BattgoOem),

    /// Live state of the BattGo pack on a channel.
    BattgoRealState(BattgoRealState),

    /// Answer to [`crate::Request::BattgoWriteParameters`].
    BattgoWriteAck {
        /// The channel this concerns, counted from zero.
        channel: u8,
        /// The charger's status byte. Zero means success.
        state: u8,
    },

    /// Charge settings stored in the BattGo pack on a channel.
    BattgoParameters(BattgoParameters),

    /// A raw sampling frame: eight cell voltages plus input and output.
    SampleData(SampleData),

    /// Identity, ratings and live output of the attached power supply.
    SmartPowerInfo(SmartPowerInfo),

    /// Answer to [`crate::Request::SetSmartPower`].
    ///
    /// A `value` of 0xFFFF means the charger rejected the setting.
    SmartPowerSettingAck {
        /// Which parameter to write.
        setting: u8,
        /// The value the charger echoed back. 0xFFFF means it refused.
        value: u16,
    },

    /// The power supply's configured working point.
    SmartPowerParameters(SmartPowerParameters),

    /// Answer to [`crate::Request::SetName`].
    NameAck {
        /// The charger's status byte. Zero means success.
        state: u8,
    },

    /// Answer to [`crate::Request::SetLimitParameters`].
    LimitParametersAck {
        /// The charger's status byte. Zero means success.
        state: u8,
    },

    /// Answer to [`crate::Request::SetOneKeyLaunch`].
    OneKeyLaunchAck {
        /// The charger's status byte. Zero means success.
        state: u8,
    },

    /// The profile the charger runs when its button is pressed.
    OneKeyLaunch(OneKeyLaunch),

    /// Answer to [`crate::Request::Calibrate8`].
    Calibrate8Ack {
        /// The channel this concerns, counted from zero.
        channel: u8,
        /// The charger's status byte. Zero means success.
        state: u8,
    },

    /// Answer to [`crate::Request::Calibrate6`].
    Calibrate6Ack {
        /// The channel this concerns, counted from zero.
        channel: u8,
        /// The charger's status byte. Zero means success.
        state: u8,
    },

    /// Device identifier, versions, display name and part number.
    HardwareInfo(Box<HardwareInfo>),

    /// The input and output power and current ceilings.
    LimitParameters(LimitParameters),

    /// Live voltages and currents for one channel.
    Electrical(Electrical),

    /// Task state and progress for one channel.
    WorkState(Box<WorkState>),

    /// Temperatures and fan duty for one channel.
    Temperature(Temperature),

    /// Answer to [`crate::Request::SetTask`].
    ///
    /// `error_code` is 0 when the charger accepted the task.
    TaskAck {
        /// The channel this concerns, counted from zero.
        channel: u8,
        /// Zero when the charger accepted the task.
        error_code: u8,
    },

    /// Answer to [`crate::Request::EnterBootloader`].
    BootloaderAck {
        /// The charger's status byte. Zero means success.
        state: u8,
    },

    /// Answer to [`crate::Request::EraseApp`].
    EraseAck {
        /// Which processor the bootloader addressed. The app only targets zero.
        cpu: u8,
        /// The charger's status byte. Zero means success.
        result: u8,
    },

    /// Answer to [`crate::Request::WriteApp`].
    WriteAck {
        /// Which processor the bootloader addressed. The app only targets zero.
        cpu: u8,
        /// Flash address the block begins at.
        start_address: u32,
        /// What the channel is doing right now.
        state: u8,
    },

    /// Answer to [`crate::Request::ChecksumApp`].
    ChecksumAck {
        /// Which processor the bootloader addressed. The app only targets zero.
        cpu: u8,
        /// The charger's status byte. Zero means success.
        state: u8,
    },

    /// Measured per-cell internal resistance for one channel.
    InnerResistance(InnerResistance),

    /// Answer to [`crate::Request::Reboot`]. The charger restarts immediately
    /// and the link drops.
    RebootAck,

    /// A frame this library does not model, kept verbatim.
    Unknown {
        /// The command word the frame arrived under.
        command: u8,
        /// The payload bytes, exactly as sent or received.
        data: Vec<u8>,
    },
}

/// The answer to [`crate::Request::Identify`], from `IsdtPackBleTest`.
///
/// The app reads this as a region byte plus an eight byte part number. A
/// CM1620 sends twelve bytes that do not fit that reading: bytes 2 to 5 are a
/// millisecond counter since the link came up, and the last six look like the
/// device address. The payload is therefore kept whole and the app's reading
/// offered as an accessor rather than baked into the fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    /// The first byte after the command word. The app calls this the region
    /// code (`runningDistrict`); a CM1620 always sends 4.
    pub region: u8,
    /// Everything after the region byte, exactly as sent.
    pub payload: Vec<u8>,
}

impl Identity {
    /// The part number as the app reads it: the first eight payload bytes.
    ///
    /// On a CM1620 these are not a part number. Use
    /// [`crate::Response::HardwareInfo`] for that.
    pub fn part_number(&self) -> Option<[u8; 8]> {
        self.payload.get(..8)?.try_into().ok()
    }

    /// Milliseconds since the link came up, as a CM1620 reports them in bytes
    /// 2 to 5 of the payload.
    ///
    /// This counts down to the unbound disconnect: a charger that has not been
    /// bound drops the link once this passes roughly five thousand.
    pub fn link_period_ms(&self) -> Option<u32> {
        let bytes: [u8; 4] = self.payload.get(1..5)?.try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    /// The last six payload bytes, which on a CM1620 hold the device address.
    pub fn device_address(&self) -> Option<[u8; 6]> {
        let start = self.payload.len().checked_sub(6)?;
        self.payload.get(start..)?.try_into().ok()
    }
}

/// A block of logged chart samples, from `IsdtPackBleLineChart`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineChart {
    /// The channel this frame describes, counted from zero.
    pub channel: u8,
    /// Up to 100 sample pairs. The app plots the first element of each pair
    /// against the second without naming either.
    pub samples: Vec<[u16; 2]>,
}

/// Link state, from `IsdtPackBleMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleMode {
    /// The charger's Bluetooth mode byte.
    pub ble_mode: u8,
    /// The charger's pairing mode byte.
    pub pair_mode: u8,
    /// How long the current link has been up, in the charger's own units.
    pub linking_period: u32,
}

/// BattGo pack identity, from `IsdtPackBattgoInfo`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BattgoInfo {
    /// The channel this frame describes, counted from zero.
    pub channel: u8,
    /// 0 when no pack is linked on this channel.
    pub link_state: u8,
    /// Eight byte device identifier, as sent.
    pub device_id: [u8; 8],
    /// Ten byte sub-device identifier, as sent.
    pub sub_device_id: [u8; 10],
    /// Sub-board hardware revision.
    pub sub_hardware_version: u8,
    /// Main board hardware revision.
    pub main_hardware_version: u8,
    /// Sub-board firmware revision.
    pub sub_software_version: u8,
    /// Main board firmware revision.
    pub main_software_version: u8,
    /// Which seat of a multi-bay charger the pack sits in.
    pub work_seat_id: u8,
    /// Identifier of the host bound to the pack.
    pub client_id: u16,
    /// The pack's serial number.
    pub battery_id: u32,
    /// Manufacturing timestamp in the pack's own encoding.
    pub manufactured: u32,
}

/// BattGo manufacturer profile, from `IsdtPackBattgoOem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BattgoOem {
    /// The channel this frame describes, counted from zero.
    pub channel: u8,
    /// Zero when nothing is linked on this channel.
    pub link_state: u8,
    /// Sixteen byte brand string, as sent.
    pub brand: [u8; 16],
    /// Chemistry code as the pack reports it.
    pub battery_type: u8,
    /// Cells the pack reports in series.
    pub cell_count: u8,
    /// Per-cell overdischarge threshold, in millivolts.
    pub over_discharge_mv: u16,
    /// Per-cell warning threshold, in millivolts.
    pub warning_mv: u16,
    /// Per-cell storage voltage, in millivolts.
    pub store_mv: u16,
    /// Per-cell fully charged voltage, in millivolts.
    pub full_charged_mv: u16,
    /// Rated capacity, in milliamp hours.
    pub capacity_mah: u32,
    /// Rated charge rate, as a multiple of capacity.
    pub charge_c: u16,
    /// Rated discharge rate, as a multiple of capacity.
    pub discharge_c: u16,
    /// Operating temperature window, in degrees Celsius.
    pub use_temp_min_c: i8,
    /// Upper end of the operating temperature window, in degrees Celsius.
    pub use_temp_max_c: i8,
    /// Storage temperature window, in degrees Celsius.
    pub store_temp_min_c: i8,
    /// Upper end of the storage temperature window, in degrees Celsius.
    pub store_temp_max_c: i8,
    /// Whether the pack self-discharges to storage voltage on its own.
    pub auto_storage: bool,
}

/// BattGo live state, from `IsdtPackBattgoRealState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BattgoRealState {
    /// The channel this frame describes, counted from zero.
    pub channel: u8,
    /// Zero when nothing is linked on this channel.
    pub link_state: u8,
    /// Per-cell voltages in millivolts, as many as the pack reports.
    pub cell_mv: Vec<u16>,
    /// Pack temperature in degrees Celsius.
    pub temperature_c: i8,
    /// Lifetime discharge current, in milliamps.
    pub discharging_current_ma: u32,
    /// Lifetime charge current, in milliamps.
    pub charging_current_ma: u32,
    /// Charge and discharge cycles the pack has logged.
    pub cycle_count: u16,
    /// Faults the pack has logged.
    pub fault_count: u16,
    /// Overtemperature events the pack has logged.
    pub over_temperature_count: u16,
    /// Overvoltage events the pack has logged.
    pub over_voltage_count: u16,
    /// Undervoltage events the pack has logged.
    pub under_voltage_count: u16,
    /// Firmware restarts the pack has logged.
    pub firmware_crash_count: u8,
}

/// Charge settings stored in a BattGo pack, from `IsdtPackBattgoReadPara`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattgoParameters {
    /// The channel this frame describes, counted from zero.
    pub channel: u8,
    /// Preferred charge current, in milliamps.
    pub charging_current_ma: u32,
    /// Per-cell storage voltage, in millivolts.
    pub store_mv: u16,
    /// Per-cell fully charged voltage, in millivolts.
    pub full_charged_mv: u16,
    /// Days of rest before the pack drops itself to storage voltage.
    pub self_discharging_days: u8,
}

/// A raw sampling frame, from `IsdtPackSampleQueryData`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleData {
    /// Per-cell voltages in millivolts.
    pub cell_mv: [u16; 8],
    /// Input voltage in millivolts.
    pub input_mv: u16,
    /// Output voltage in millivolts.
    pub output_mv: u16,
    /// Output current in milliamps.
    pub output_ma: u16,
}

/// Power supply identity and live output, from `IsdtPackBleSmartPowerInfo`.
///
/// A CM1620 never sends this frame. It belongs to ISDT's mains-powered
/// stations, whose screen divides every voltage here by ten, so voltages are
/// in tenths of a volt. The app never displays the currents, so their unit is
/// not established by the app and they are left raw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartPowerInfo {
    /// True when the supply is a plain one with nothing to report.
    pub common_power: bool,
    /// What kind of supply is attached.
    pub power_type: PowerType,
    /// Rated output power, in watts.
    pub max_power_w: u16,
    /// Mains input voltage, in volts.
    pub input_ac_v: u16,
    /// Supply temperature, in degrees Celsius.
    pub temperature_c: i8,
    /// Warning bits. The app reads the byte and never decodes it, so the
    /// meaning of each bit is not established by the app.
    pub warning_flags: u8,
    /// Fault bits, likewise never decoded by the app.
    pub error_flags: u8,
    /// Output voltage window, in tenths of a volt.
    pub min_output_dv: u16,
    /// Upper end of the output voltage window, in tenths of a volt.
    pub max_output_dv: u16,
    /// Output current window, in the supply's own units.
    pub min_output_current_raw: u16,
    /// Upper end of the output current window, in the supply's own units.
    pub max_output_current_raw: u16,
    /// Present output voltage, in tenths of a volt.
    pub output_dv: u16,
    /// Present output current, in the supply's own units.
    pub output_current_raw: u16,
}

/// The supply's configured working point, from `IsdtPackBleSmartPowerPara`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartPowerParameters {
    /// True when the supply is a plain one with nothing to report.
    pub common_power: bool,
    /// What kind of supply is attached.
    pub power_type: PowerType,
    /// How long the supply has been working, in seconds.
    pub working_seconds: u32,
    /// Configured output current, in the supply's own units.
    pub working_current_raw: u16,
    /// Configured output voltage, in tenths of a volt.
    pub working_voltage_dv: u16,
}

/// The button-press profile, from `IsdtPackOneKeyLaunchQuery`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OneKeyLaunch {
    /// Whether the profile runs on a button press.
    pub enabled: bool,
    /// Chemistry code, on the same scale [`crate::BatteryKind`] writes.
    pub battery_type: u8,
    /// Cells the pack reports in series.
    pub cell_count: u8,
    /// Per-cell target voltage, in millivolts.
    pub full_charged_mv: u16,
    /// Charge current, in milliamps.
    pub work_current_ma: u32,
}

/// Device identity and versions, from `IsdtPackBleHardInfo`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareInfo {
    /// Eight byte device identifier, usually printable ASCII such as `CM1620`.
    pub device_id: [u8; 8],
    /// Hardware version as major, minor, patch, board layout.
    pub hardware_version: [u8; 4],
    /// Bootloader version as major, minor, patch, build.
    pub bootloader_version: [u8; 4],
    /// Firmware version as major, minor, patch, build.
    pub firmware_version: [u8; 4],
    /// Ten byte display name. Absent on firmware that stops the frame early.
    pub device_name: Option<[u8; 10]>,
    /// Eight byte part number. Absent on firmware that stops the frame early.
    pub part_number: Option<[u8; 8]>,
}

impl HardwareInfo {
    /// The device identifier as text, with trailing padding removed.
    pub fn device_id_string(&self) -> String {
        trim_ascii(&self.device_id)
    }

    /// The display name as text, with trailing padding removed.
    pub fn device_name_string(&self) -> Option<String> {
        self.device_name.map(|n| trim_ascii(&n))
    }

    /// The part number as text, with trailing padding removed.
    pub fn part_number_string(&self) -> Option<String> {
        self.part_number.map(|p| trim_ascii(&p))
    }
}

/// Power and current ceilings, from `IsdtPackBleLimitPara`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitParameters {
    /// Input power ceiling, in milliwatts.
    pub max_input_power_mw: u32,
    /// Output power ceiling, in milliwatts.
    pub max_output_power_mw: u32,
    /// Input current ceiling, in milliamps.
    pub max_input_current_ma: u32,
    /// Output current ceiling, in milliamps.
    pub max_output_current_ma: u32,
}

/// Live voltages and currents, from `IsdtPackBleElecQuery`.
///
/// The frame comes in two widths. Short frames carry 16 bit input and output
/// voltages and eight cells; long frames carry 32 bit voltages and sixteen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Electrical {
    /// The channel this frame describes, counted from zero.
    pub channel: u8,
    /// Input voltage, in millivolts.
    pub input_mv: u32,
    /// Input current, in milliamps.
    pub input_ma: u32,
    /// Output voltage, in millivolts.
    pub output_mv: u32,
    /// Charge or discharge current, in milliamps.
    pub current_ma: u32,
    /// Per-cell voltages in millivolts: eight on short frames, sixteen on long.
    pub cell_mv: Vec<u16>,
}

impl Electrical {
    /// Input power in milliwatts, computed from the reported input readings.
    pub fn input_power_mw(&self) -> u64 {
        u64::from(self.input_mv) * u64::from(self.input_ma) / 1000
    }

    /// Output power in milliwatts, computed from the reported output readings.
    pub fn output_power_mw(&self) -> u64 {
        u64::from(self.output_mv) * u64::from(self.current_ma) / 1000
    }

    /// The cells reporting a non-zero voltage, which is how many the charger
    /// can actually see through the balance connector.
    pub fn connected_cells(&self) -> usize {
        self.cell_mv.iter().filter(|mv| **mv > 0).count()
    }
}

/// Task state and progress, from `IsdtPackBleWorkState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkState {
    /// The channel this frame describes, counted from zero.
    pub channel: u8,
    /// What the channel is doing right now.
    pub state: ChargerState,
    /// Progress toward the task's target, in percent. The app clamps its
    /// gauge to 100, so a charger may report more than that.
    pub capacity_percent: u8,
    /// Charge moved so far, in milliamp hours.
    pub capacity_done_mah: u32,
    /// Energy moved so far, in milliwatt hours.
    pub energy_done_mwh: u32,
    /// Elapsed task time, in milliseconds.
    pub work_period_ms: u32,
    /// The chemistry the charger is working to, on the same scale
    /// [`crate::Request::SetTask`] writes.
    pub battery_type: BatteryKind,
    /// Cells the charger has detected in series.
    pub cell_count: u8,
    /// Which leads the charger sees connected. Zero means nothing is attached
    /// and the charger will refuse to start.
    pub link_type: u8,
    /// Per-cell target voltage, in millivolts.
    pub full_charged_mv: u16,
    /// Configured task current, in milliamps.
    pub work_current_ma: u32,
    /// Batteries in the whole job, for chargers that run a magazine.
    pub batteries_total: u16,
    /// Which battery of that job is in progress.
    pub batteries_done: u16,
    /// Input undervoltage cutoff, in millivolts.
    pub min_input_mv: u16,
    /// Output power ceiling, in milliwatts.
    pub max_output_power_mw: u32,
    /// Fault mask, absent on firmware whose frame stops before it.
    pub errors: Option<ErrorFlags>,
}

impl WorkState {
    /// Elapsed task time as a duration.
    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::from_millis(u64::from(self.work_period_ms))
    }

    /// Which leads the charger sees connected, when the code is one it names.
    pub fn link(&self) -> Option<crate::types::LinkType> {
        crate::types::LinkType::from_code(self.link_type)
    }

    /// The same work-state byte read on the scale ISDT's cell-slot chargers
    /// use, for a device that does not follow the charger table.
    pub fn slot_state(&self) -> ChannelState {
        ChannelState::from_code(self.state_code())
    }

    /// The raw work-state byte.
    pub fn state_code(&self) -> u8 {
        match self.state {
            ChargerState::Standby => 0,
            ChargerState::Activating => 1,
            ChargerState::CurrentRise => 2,
            ChargerState::ConstantCurrent => 3,
            ChargerState::ConstantVoltage => 4,
            ChargerState::ConstantVoltageBalance => 5,
            ChargerState::TrickleBalance => 6,
            ChargerState::StorageCharging => 7,
            ChargerState::StorageDischarging => 8,
            ChargerState::Stored => 9,
            ChargerState::Discharging => 10,
            ChargerState::Discharged => 11,
            ChargerState::Unknown(code) => code,
        }
    }

    /// True when the charger reported at least one fault.
    pub fn has_error(&self) -> bool {
        matches!(self.errors, Some(flags) if !flags.is_clear())
    }
}

/// Temperatures and fan duty, from `IsdtPackBleTemper`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Temperature {
    /// The channel this frame describes, counted from zero.
    pub channel: u8,
    /// Charger internal temperature, in degrees Celsius. Signed.
    pub device_c: i8,
    /// Battery temperature from the external probe, in degrees Celsius. Signed.
    pub battery_c: i8,
    /// Fan setting. The app parses this byte and never shows it, so its scale
    /// is not established by the app.
    pub fan_raw: u8,
}

/// Per-cell internal resistance, from `IsdtPackBleInnerResistance`.
///
/// The app divides each reading by ten before printing it in milliohms, so the
/// wire unit is a tenth of a milliohm. A reading above 6553 milliohms is the
/// charger's way of saying it has no measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InnerResistance {
    /// The channel this frame describes, counted from zero.
    pub channel: u8,
    /// Raw readings, in tenths of a milliohm.
    pub raw: [u16; 8],
}

/// Above this many milliohms the app treats a reading as absent.
/// From `IsdtBleProtocol.MAX_INNER_RESISTANCE`.
pub const MAX_INNER_RESISTANCE_MOHM: f32 = 6553.0;

impl InnerResistance {
    /// Each cell's resistance in milliohms, or `None` where the charger has no
    /// measurement.
    pub fn cells_mohm(&self) -> [Option<f32>; 8] {
        self.raw.map(|raw| {
            let mohm = f32::from(raw) / 10.0;
            (mohm <= MAX_INNER_RESISTANCE_MOHM).then_some(mohm)
        })
    }
}

impl Response {
    /// Decodes a frame `DATA` field.
    ///
    /// An empty slice yields `None`. A command word the library does not model,
    /// or a frame too short for its layout, yields [`Response::Unknown`] so the
    /// bytes survive for inspection.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let (&command, body) = data.split_first()?;
        Some(
            Self::parse_body(command, body).unwrap_or_else(|| Response::Unknown {
                command,
                data: body.to_vec(),
            }),
        )
    }

    fn parse_body(command: u8, body: &[u8]) -> Option<Self> {
        let mut r = Reader::new(body);
        Some(match command {
            0x01 => Response::Identity(Identity {
                region: r.u8()?,
                payload: body.get(1..).unwrap_or_default().to_vec(),
            }),

            0x0E => {
                let channel = r.u8()?;
                let mut samples = Vec::new();
                while r.remaining() >= 4 {
                    samples.push([r.u16()?, r.u16()?]);
                }
                Response::LineChart(LineChart { channel, samples })
            }

            0x13 => Response::BleMode(BleMode {
                ble_mode: r.u8()?,
                pair_mode: r.u8()?,
                linking_period: r.u32()?,
            }),

            0x19 => Response::Bind {
                bound: r.u8()? == 0,
            },

            0x41 => Response::BattgoInfo(BattgoInfo {
                channel: r.u8()?,
                link_state: r.u8()?,
                device_id: r.array::<8>()?,
                sub_device_id: r.array::<10>()?,
                sub_hardware_version: r.u8()?,
                main_hardware_version: r.u8()?,
                sub_software_version: r.u8()?,
                main_software_version: r.u8()?,
                work_seat_id: r.u8()?,
                client_id: r.u16()?,
                battery_id: r.u32()?,
                manufactured: r.u32()?,
            }),

            0x43 => Response::BattgoOem(BattgoOem {
                channel: r.u8()?,
                link_state: r.u8()?,
                brand: r.array::<16>()?,
                battery_type: r.u8()?,
                cell_count: r.u8()?,
                over_discharge_mv: r.u16()?,
                warning_mv: r.u16()?,
                store_mv: r.u16()?,
                full_charged_mv: r.u16()?,
                capacity_mah: r.u32()?,
                charge_c: r.u16()?,
                discharge_c: r.u16()?,
                use_temp_min_c: r.i8()?,
                use_temp_max_c: r.i8()?,
                store_temp_min_c: r.i8()?,
                store_temp_max_c: r.i8()?,
                auto_storage: r.u8()? != 0,
            }),

            0x45 => {
                let channel = r.u8()?;
                let link_state = r.u8()?;
                let cells = r.u8()?;
                let mut cell_mv = Vec::with_capacity(cells as usize);
                for _ in 0..cells {
                    cell_mv.push(r.u16()?);
                }
                Response::BattgoRealState(BattgoRealState {
                    channel,
                    link_state,
                    cell_mv,
                    temperature_c: r.i8()?,
                    discharging_current_ma: r.u32()?,
                    charging_current_ma: r.u32()?,
                    cycle_count: r.u16()?,
                    fault_count: r.u16()?,
                    over_temperature_count: r.u16()?,
                    over_voltage_count: r.u16()?,
                    under_voltage_count: r.u16()?,
                    firmware_crash_count: r.u8()?,
                })
            }

            0x47 => Response::BattgoWriteAck {
                channel: r.u8()?,
                state: r.u8()?,
            },

            0x49 => Response::BattgoParameters(BattgoParameters {
                channel: r.u8()?,
                charging_current_ma: r.u32()?,
                store_mv: r.u16()?,
                full_charged_mv: r.u16()?,
                self_discharging_days: r.u8()?,
            }),

            0xB1 => {
                let mut cell_mv = [0u16; 8];
                for slot in &mut cell_mv {
                    *slot = r.u16()?;
                }
                Response::SampleData(SampleData {
                    cell_mv,
                    input_mv: r.u16()?,
                    output_mv: r.u16()?,
                    output_ma: r.u16()?,
                })
            }

            0xBB => Response::SmartPowerInfo(SmartPowerInfo {
                common_power: r.u8()? == 0,
                power_type: PowerType::from_code(r.u8()?),
                max_power_w: r.u16()?,
                input_ac_v: r.u16()?,
                temperature_c: r.i8()?,
                warning_flags: r.u8()?,
                error_flags: r.u8()?,
                min_output_dv: r.u16()?,
                max_output_dv: r.u16()?,
                min_output_current_raw: r.u16()?,
                max_output_current_raw: r.u16()?,
                output_dv: r.u16()?,
                output_current_raw: r.u16()?,
            }),

            0xBD => Response::SmartPowerSettingAck {
                setting: r.u8()?,
                value: r.u16()?,
            },

            0xBF => Response::SmartPowerParameters(SmartPowerParameters {
                common_power: r.u8()? == 0,
                power_type: PowerType::from_code(r.u8()?),
                working_seconds: r.u32()?,
                working_current_raw: r.u16()?,
                working_voltage_dv: r.u16()?,
            }),

            0xC1 => Response::NameAck { state: r.u8()? },
            0xD1 => Response::LimitParametersAck { state: r.u8()? },
            0xD3 => Response::OneKeyLaunchAck { state: r.u8()? },

            0xD5 => Response::OneKeyLaunch(OneKeyLaunch {
                enabled: r.u8()? != 0,
                battery_type: r.u8()?,
                cell_count: r.u8()?,
                full_charged_mv: r.u16()?,
                work_current_ma: r.u32()?,
            }),

            0xDD => Response::Calibrate8Ack {
                channel: r.u8()?,
                state: r.u8()?,
            },

            0xDF => Response::Calibrate6Ack {
                channel: r.u8()?,
                state: r.u8()?,
            },

            0xE1 => {
                let device_id = r.array::<8>()?;
                let hardware_version = r.array::<4>()?;
                let bootloader_version = r.array::<4>()?;
                let firmware_version = r.array::<4>()?;
                // Older firmware ends the frame here.
                let device_name = r.array::<10>();
                let part_number = device_name.and_then(|_| r.array::<8>());
                Response::HardwareInfo(Box::new(HardwareInfo {
                    device_id,
                    hardware_version,
                    bootloader_version,
                    firmware_version,
                    device_name,
                    part_number,
                }))
            }

            0xE3 => Response::LimitParameters(LimitParameters {
                max_input_power_mw: r.u32()?,
                max_output_power_mw: r.u32()?,
                max_input_current_ma: r.u32()?,
                max_output_current_ma: r.u32()?,
            }),

            // The app switches layout on the frame length, counting the
            // command word: 35 bytes and up is the wide, sixteen cell form.
            0xE5 => {
                let wide = body.len() + 1 >= 35;
                let channel = r.u8()?;
                let input_mv = if wide { r.u32()? } else { u32::from(r.u16()?) };
                let input_ma = r.u32()?;
                let output_mv = if wide { r.u32()? } else { u32::from(r.u16()?) };
                let current_ma = r.u32()?;
                let cells = if wide { 16 } else { 8 };
                let mut cell_mv = Vec::with_capacity(cells);
                for _ in 0..cells {
                    cell_mv.push(r.u16()?);
                }
                Response::Electrical(Electrical {
                    channel,
                    input_mv,
                    input_ma,
                    output_mv,
                    current_ma,
                    cell_mv,
                })
            }

            0xE7 => Response::WorkState(Box::new(WorkState {
                channel: r.u8()?,
                state: ChargerState::from_code(r.u8()?),
                capacity_percent: r.u8()?,
                capacity_done_mah: r.u32()?,
                energy_done_mwh: r.u32()?,
                work_period_ms: r.u32()?,
                battery_type: BatteryKind::from_code(r.u8()?),
                cell_count: r.u8()?,
                link_type: r.u8()?,
                full_charged_mv: r.u16()?,
                work_current_ma: r.u32()?,
                batteries_total: r.u16()?,
                batteries_done: r.u16()?,
                min_input_mv: r.u16()?,
                max_output_power_mw: r.u32()?,
                // Older firmware ends the frame before the fault mask.
                errors: r.u16().map(ErrorFlags),
            })),

            0xE9 => Response::Temperature(Temperature {
                channel: r.u8()?,
                device_c: r.i8()?,
                battery_c: r.i8()?,
                fan_raw: r.u8()?,
            }),

            0xEB => Response::TaskAck {
                channel: r.u8()?,
                error_code: r.u8()?,
            },

            0xF1 => Response::BootloaderAck { state: r.u8()? },

            0xF3 => Response::EraseAck {
                cpu: r.u8()?,
                result: r.u8()?,
            },

            0xF5 => Response::WriteAck {
                cpu: r.u8()?,
                start_address: r.u32()?,
                state: r.u8()?,
            },

            0xF7 => Response::ChecksumAck {
                cpu: r.u8()?,
                state: r.u8()?,
            },

            0xFB => {
                let channel = r.u8()?;
                let mut raw = [0u16; 8];
                for slot in &mut raw {
                    *slot = r.u16()?;
                }
                Response::InnerResistance(InnerResistance { channel, raw })
            }

            0xFD => Response::RebootAck,

            _ => return None,
        })
    }

    /// The command word this response arrived under.
    pub fn command_word(&self) -> u8 {
        match self {
            Response::Identity(_) => 0x01,
            Response::LineChart(_) => 0x0E,
            Response::BleMode(_) => 0x13,
            Response::Bind { .. } => 0x19,
            Response::BattgoInfo(_) => 0x41,
            Response::BattgoOem(_) => 0x43,
            Response::BattgoRealState(_) => 0x45,
            Response::BattgoWriteAck { .. } => 0x47,
            Response::BattgoParameters(_) => 0x49,
            Response::SampleData(_) => 0xB1,
            Response::SmartPowerInfo(_) => 0xBB,
            Response::SmartPowerSettingAck { .. } => 0xBD,
            Response::SmartPowerParameters(_) => 0xBF,
            Response::NameAck { .. } => 0xC1,
            Response::LimitParametersAck { .. } => 0xD1,
            Response::OneKeyLaunchAck { .. } => 0xD3,
            Response::OneKeyLaunch(_) => 0xD5,
            Response::Calibrate8Ack { .. } => 0xDD,
            Response::Calibrate6Ack { .. } => 0xDF,
            Response::HardwareInfo(_) => 0xE1,
            Response::LimitParameters(_) => 0xE3,
            Response::Electrical(_) => 0xE5,
            Response::WorkState(_) => 0xE7,
            Response::Temperature(_) => 0xE9,
            Response::TaskAck { .. } => 0xEB,
            Response::BootloaderAck { .. } => 0xF1,
            Response::EraseAck { .. } => 0xF3,
            Response::WriteAck { .. } => 0xF5,
            Response::ChecksumAck { .. } => 0xF7,
            Response::InnerResistance(_) => 0xFB,
            Response::RebootAck => 0xFD,
            Response::Unknown { command, .. } => *command,
        }
    }

    /// The channel this response describes, for frames that name one.
    pub fn channel(&self) -> Option<u8> {
        Some(match self {
            Response::Electrical(e) => e.channel,
            Response::WorkState(w) => w.channel,
            Response::Temperature(t) => t.channel,
            Response::InnerResistance(ir) => ir.channel,
            Response::TaskAck { channel, .. } => *channel,
            Response::LineChart(c) => c.channel,
            Response::BattgoInfo(b) => b.channel,
            Response::BattgoOem(b) => b.channel,
            Response::BattgoRealState(b) => b.channel,
            Response::BattgoParameters(b) => b.channel,
            Response::BattgoWriteAck { channel, .. } => *channel,
            Response::Calibrate6Ack { channel, .. } => *channel,
            Response::Calibrate8Ack { channel, .. } => *channel,
            _ => return None,
        })
    }
}

/// Reads a byte string up to its first NUL, as printable text.
fn trim_ascii(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

/// Little-endian cursor that yields `None` instead of panicking at the end of
/// the buffer, so a truncated frame degrades rather than crashes.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn i8(&mut self) -> Option<i8> {
        Some(self.u8()? as i8)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_temperature_frame() {
        let data = [0xE9, 0x00, 0xF8, 0x1A, 0x32];
        let Some(Response::Temperature(t)) = Response::parse(&data) else {
            panic!("expected a temperature frame");
        };
        assert_eq!(t.channel, 0);
        assert_eq!(t.device_c, -8); // signed, as the app reads it
        assert_eq!(t.battery_c, 26);
        assert_eq!(t.fan_raw, 50);
    }

    #[test]
    fn decodes_the_narrow_electrical_frame() {
        let mut data = vec![0xE5, 0x00];
        data.extend_from_slice(&12_000u16.to_le_bytes()); // input mV
        data.extend_from_slice(&1_500u32.to_le_bytes()); // input mA
        data.extend_from_slice(&16_800u16.to_le_bytes()); // output mV
        data.extend_from_slice(&2_000u32.to_le_bytes()); // current mA
        for cell in 0..8u16 {
            data.extend_from_slice(&(4_100 + cell).to_le_bytes());
        }
        assert!(data.len() < 35);

        let Some(Response::Electrical(e)) = Response::parse(&data) else {
            panic!("expected an electrical frame");
        };
        assert_eq!(e.input_mv, 12_000);
        assert_eq!(e.input_ma, 1_500);
        assert_eq!(e.output_mv, 16_800);
        assert_eq!(e.current_ma, 2_000);
        assert_eq!(e.cell_mv.len(), 8);
        assert_eq!(e.cell_mv[0], 4_100);
        assert_eq!(e.input_power_mw(), 18_000);
        assert_eq!(e.connected_cells(), 8);
    }

    #[test]
    fn decodes_the_wide_electrical_frame() {
        let mut data = vec![0xE5, 0x01];
        data.extend_from_slice(&48_000u32.to_le_bytes());
        data.extend_from_slice(&5_000u32.to_le_bytes());
        data.extend_from_slice(&25_200u32.to_le_bytes());
        data.extend_from_slice(&8_000u32.to_le_bytes());
        for _ in 0..16 {
            data.extend_from_slice(&4_200u16.to_le_bytes());
        }
        assert!(data.len() >= 35);

        let Some(Response::Electrical(e)) = Response::parse(&data) else {
            panic!("expected an electrical frame");
        };
        assert_eq!(e.channel, 1);
        assert_eq!(e.input_mv, 48_000);
        assert_eq!(e.output_mv, 25_200);
        assert_eq!(e.cell_mv.len(), 16);
    }

    #[test]
    fn decodes_a_work_state_frame_with_and_without_a_fault_code() {
        let mut data = vec![0xE7, 0x00, 0x03, 0x2A];
        data.extend_from_slice(&1_234u32.to_le_bytes()); // mAh
        data.extend_from_slice(&18_500u32.to_le_bytes()); // mWh
        data.extend_from_slice(&600_000u32.to_le_bytes()); // milliseconds
        data.push(1); // chemistry: LiPo
        data.push(4); // cells
        data.push(3); // both leads
        data.extend_from_slice(&4_200u16.to_le_bytes());
        data.extend_from_slice(&3_000u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&10_000u16.to_le_bytes());
        data.extend_from_slice(&200_000u32.to_le_bytes());

        let short = Response::parse(&data).unwrap();
        let Response::WorkState(w) = &short else {
            panic!("expected a work state frame");
        };
        assert_eq!(w.state, ChargerState::ConstantCurrent);
        assert_eq!(w.battery_type, BatteryKind::LiPo);
        assert_eq!(w.capacity_percent, 42);
        assert_eq!(w.capacity_done_mah, 1_234);
        assert_eq!(w.elapsed().as_secs(), 600);
        assert_eq!(w.link(), Some(crate::types::LinkType::Both));
        assert_eq!(w.errors, None);
        assert!(!w.has_error());

        // Bits 0 and 1: output overcurrent and overvoltage.
        data.extend_from_slice(&0b11u16.to_le_bytes());
        let Some(Response::WorkState(w)) = Response::parse(&data) else {
            panic!("expected a work state frame");
        };
        assert!(w.has_error());
        assert_eq!(
            w.errors.unwrap().labels(),
            vec!["output overcurrent", "output overvoltage"]
        );
    }

    #[test]
    fn decodes_hardware_info_both_long_and_short() {
        let mut data = vec![0xE1];
        data.extend_from_slice(b"CM1620\0\0");
        data.extend_from_slice(&[1, 0, 0, 2]);
        data.extend_from_slice(&[1, 1, 0, 3]);
        data.extend_from_slice(&[2, 4, 1, 9]);

        let Some(Response::HardwareInfo(h)) = Response::parse(&data) else {
            panic!("expected a hardware info frame");
        };
        assert_eq!(h.device_id_string(), "CM1620");
        assert_eq!(h.firmware_version, [2, 4, 1, 9]);
        assert_eq!(h.device_name, None);

        data.extend_from_slice(b"Bench\0\0\0\0\0");
        data.extend_from_slice(b"PN012345");
        let Some(Response::HardwareInfo(h)) = Response::parse(&data) else {
            panic!("expected a hardware info frame");
        };
        assert_eq!(h.device_name_string().as_deref(), Some("Bench"));
        assert_eq!(h.part_number_string().as_deref(), Some("PN012345"));
    }

    #[test]
    fn decodes_internal_resistance_in_tenths_of_a_milliohm() {
        let mut data = vec![0xFB, 0x00];
        // 12.5 mOhm on the first cell, then the no-measurement sentinel.
        data.extend_from_slice(&125u16.to_le_bytes());
        data.extend_from_slice(&u16::MAX.to_le_bytes());
        for _ in 2..8 {
            data.extend_from_slice(&0u16.to_le_bytes());
        }
        let Some(Response::InnerResistance(ir)) = Response::parse(&data) else {
            panic!("expected an internal resistance frame");
        };
        assert_eq!(ir.raw[0], 125);
        let cells = ir.cells_mohm();
        assert_eq!(cells[0], Some(12.5));
        assert_eq!(cells[1], None);
        assert_eq!(cells[2], Some(0.0));
    }

    #[test]
    fn decodes_a_battgo_pack_with_a_variable_cell_count() {
        let mut data = vec![0x45, 0x00, 0x01, 0x03];
        for _ in 0..3 {
            data.extend_from_slice(&3_950u16.to_le_bytes());
        }
        data.push(24u8); // temperature
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&500u32.to_le_bytes());
        data.extend_from_slice(&12u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);

        let Some(Response::BattgoRealState(b)) = Response::parse(&data) else {
            panic!("expected a BattGo state frame");
        };
        assert_eq!(b.cell_mv, vec![3_950; 3]);
        assert_eq!(b.temperature_c, 24);
        assert_eq!(b.cycle_count, 12);
        assert_eq!(b.under_voltage_count, 1);
    }

    /// Bytes captured from a real CM1620 across three connections.
    #[test]
    fn reads_the_cm1620_identify_frame() {
        let data = [
            0x01u8, 0x04, 0x00, 0xe8, 0x03, 0x00, 0x00, 0x70, 0xa9, 0x38, 0xe4, 0xc2, 0x84,
        ];
        let Some(Response::Identity(id)) = Response::parse(&data) else {
            panic!("expected an identify frame");
        };
        assert_eq!(id.region, 4);
        // The counter that runs out at the unbound disconnect.
        assert_eq!(id.link_period_ms(), Some(1000));
        assert_eq!(
            id.device_address(),
            Some([0x70, 0xa9, 0x38, 0xe4, 0xc2, 0x84])
        );
        // The app's reading of the same bytes, kept for compatibility.
        assert_eq!(
            id.part_number(),
            Some([0x00, 0xe8, 0x03, 0x00, 0x00, 0x70, 0xa9, 0x38])
        );

        // A later connection: only the counter moves.
        let mut later = data;
        later[3] = 0x88;
        later[4] = 0x13;
        let Some(Response::Identity(id)) = Response::parse(&later) else {
            panic!("expected an identify frame");
        };
        assert_eq!(id.link_period_ms(), Some(5000));
        assert_eq!(
            id.device_address(),
            Some([0x70, 0xa9, 0x38, 0xe4, 0xc2, 0x84])
        );
    }

    #[test]
    fn keeps_unmodelled_and_truncated_frames_verbatim() {
        let unmodelled = Response::parse(&[0x77, 0x01, 0x02]).unwrap();
        assert_eq!(
            unmodelled,
            Response::Unknown {
                command: 0x77,
                data: vec![0x01, 0x02]
            }
        );

        // A temperature frame cut short falls back rather than panicking.
        let truncated = Response::parse(&[0xE9, 0x00]).unwrap();
        assert!(matches!(truncated, Response::Unknown { command: 0xE9, .. }));

        assert_eq!(Response::parse(&[]), None);
    }

    #[test]
    fn every_modelled_response_reports_its_own_command_word() {
        let data = [0xEB, 0x00, 0x00];
        let parsed = Response::parse(&data).unwrap();
        assert_eq!(parsed.command_word(), 0xEB);
        assert_eq!(parsed.channel(), Some(0));
    }
}
