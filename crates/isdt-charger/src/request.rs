//! Every command and query the ISDT Android app can send to a charger.
//!
//! Each variant of [`Request`] mirrors one `IsdtPack*Req` class from
//! `com.isdt.hubin.isdtapp.ble`. [`Request::data`] produces the frame `DATA`
//! field, which [`crate::frame::encode`] then wraps and stuffs.

use crate::types::{BatteryKind, CalibrationMode, LinkType, TaskType};

/// Number of firmware bytes carried by a single [`Request::WriteApp`].
pub const WRITE_APP_BLOCK: usize = 128;

/// The magic word guarding smart-power setting writes (`safeCmd` in the app).
const SMART_POWER_SAFE_WORD: u16 = 0xA5C3;

/// The magic byte guarding [`Request::EnterBootloader`].
const BOOTLOADER_MAGIC: u8 = 0xAC;

/// The magic byte guarding [`Request::Reboot`].
const REBOOT_MAGIC: u8 = 0xCA;

/// A command or query addressed to the charger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    // ---- identification and pairing -------------------------------------
    /// Asks the charger to identify itself. The charger answers with
    /// [`crate::Response::Identity`], and Bluetooth-mode capable units also
    /// answer with [`crate::Response::BleMode`].
    Identify,

    /// Binds this host to the charger using a 16 byte client identifier.
    ///
    /// The app derives the identifier from the phone's installation UUID and
    /// sends it while the charger advertises in pairing mode.
    Bind {
        /// Sixteen byte host identifier the charger stores.
        client_id: [u8; 16],
    },

    /// Renames the charger. The name is padded to 16 bytes with zeroes and
    /// longer names are rejected.
    SetName {
        /// The new name. Anything past sixteen bytes is dropped.
        name: String,
    },

    /// Reads device identifier, hardware, bootloader and firmware versions,
    /// display name and part number.
    HardwareInfo,

    // ---- live telemetry --------------------------------------------------
    /// Reads input voltage and current, output voltage, charge current and
    /// the per-cell voltages of one channel.
    Electrical {
        /// The channel this concerns, counted from zero.
        channel: u8,
    },

    /// Reads the task state, progress, delivered capacity and energy, elapsed
    /// time and error code of one channel.
    WorkState {
        /// The channel this concerns, counted from zero.
        channel: u8,
    },

    /// Reads device temperature, battery temperature and fan duty of one channel.
    Temperature {
        /// The channel this concerns, counted from zero.
        channel: u8,
    },

    /// Reads the measured per-cell internal resistance of one channel.
    InnerResistance {
        /// The channel this concerns, counted from zero.
        channel: u8,
    },

    // ---- task control ----------------------------------------------------
    /// Starts, changes or stops the task running on one channel.
    ///
    /// `work_current` is in milliamps and `full_charged_volt` is the per-cell
    /// target in millivolts. Send [`TaskType::Stop`] to halt the channel; the
    /// remaining fields are then ignored by the charger.
    SetTask {
        /// The channel this concerns, counted from zero.
        channel: u8,
        /// What the channel should do.
        task: TaskType,
        /// The pack's chemistry.
        battery: BatteryKind,
        /// Which leads are connected.
        link: LinkType,
        /// Charge or discharge current, in milliamps.
        work_current_ma: u32,
        /// Cells in series.
        cell_count: u8,
        /// Per-cell target voltage, in millivolts.
        full_charged_volt_mv: u16,
    },

    // ---- power limits ----------------------------------------------------
    /// Reads the input and output power and current ceilings.
    LimitParameters,

    /// Writes the input power ceiling and the input undervoltage cutoff.
    SetLimitParameters {
        /// Input undervoltage cutoff, in millivolts.
        min_input_volt_mv: u16,
        /// Input power ceiling, in milliwatts.
        max_input_power_mw: u32,
    },

    // ---- one-key launch --------------------------------------------------
    /// Reads the profile the charger runs when its button is pressed.
    OneKeyLaunch,

    /// Writes the profile the charger runs when its button is pressed.
    SetOneKeyLaunch {
        /// Whether the profile runs on a button press.
        enabled: bool,
        /// The pack's chemistry.
        battery: BatteryKind,
        /// Cells in series.
        cell_count: u8,
        /// Per-cell target voltage, in millivolts.
        full_charged_volt_mv: u16,
        /// Charge or discharge current, in milliamps.
        work_current_ma: u32,
    },

    // ---- smart power supply ----------------------------------------------
    /// Reads the identity, ratings and live output of the attached power supply.
    SmartPowerInfo,

    /// Reads the power supply's configured working point.
    SmartPowerParameters,

    /// Writes one power supply setting. `setting` selects the parameter and
    /// `value` its new value; both are passed through verbatim.
    SetSmartPower {
        /// Which parameter to write.
        setting: u8,
        /// The value the charger echoed back. 0xFFFF means it refused.
        value: u16,
    },

    // ---- BattGo smart batteries -------------------------------------------
    /// Reads the identity of the BattGo battery on one channel.
    BattgoInfo {
        /// The channel this concerns, counted from zero.
        channel: u8,
    },

    /// Reads the manufacturer profile of the BattGo battery on one channel.
    BattgoOem {
        /// The channel this concerns, counted from zero.
        channel: u8,
    },

    /// Reads the live cell voltages, currents, temperature and fault counters
    /// of the BattGo battery on one channel.
    BattgoRealState {
        /// The channel this concerns, counted from zero.
        channel: u8,
    },

    /// Reads the charge settings stored in the BattGo battery on one channel.
    BattgoReadParameters {
        /// The channel this concerns, counted from zero.
        channel: u8,
    },

    /// Writes the charge settings stored in the BattGo battery on one channel.
    BattgoWriteParameters {
        /// The channel this concerns, counted from zero.
        channel: u8,
        /// Preferred charge current, in milliamps.
        charging_current_ma: u32,
        /// Per-cell storage voltage, in millivolts.
        store_volt_mv: u16,
        /// Per-cell target voltage, in millivolts.
        full_charged_volt_mv: u16,
        /// Days of rest before the pack drops to storage voltage.
        self_discharging_days: u8,
    },

    // ---- calibration -------------------------------------------------------
    /// Calibrates a six-cell balance port, or restores its factory constants.
    ///
    /// The voltages are the true values measured with a reference meter, in
    /// millivolts. They are ignored when `mode` is
    /// [`CalibrationMode::RestoreDefaults`].
    Calibrate6 {
        /// The channel this concerns, counted from zero.
        channel: u8,
        /// Whether to store new constants or restore the factory set.
        mode: CalibrationMode,
        /// Reference cell voltages, in millivolts.
        cell_mv: [u16; 6],
        /// Reference input voltage, in millivolts.
        input_mv: u16,
        /// Reference output voltage, in millivolts.
        output_mv: u16,
    },

    /// Calibrates an eight-cell balance port, or restores its factory constants.
    Calibrate8 {
        /// The channel this concerns, counted from zero.
        channel: u8,
        /// Whether to store new constants or restore the factory set.
        mode: CalibrationMode,
        /// Reference cell voltages, in millivolts.
        cell_mv: [u16; 8],
        /// Reference input voltage, in millivolts.
        input_mv: u16,
        /// Reference output voltage, in millivolts.
        output_mv: u16,
    },

    // ---- firmware update ---------------------------------------------------
    /// Puts the charger into its bootloader.
    EnterBootloader,

    /// Erases a firmware region.
    EraseApp {
        /// Flash address the region begins at.
        start_address: u32,
        /// Length of the region, in bytes.
        size: u32,
    },

    /// Writes one [`WRITE_APP_BLOCK`] byte firmware block.
    WriteApp {
        /// Flash address the block begins at.
        start_address: u32,
        /// The payload bytes, exactly as sent or received.
        data: Box<[u8; WRITE_APP_BLOCK]>,
    },

    /// Verifies a written firmware region against a checksum.
    ChecksumApp {
        /// Flash address the block begins at.
        start_address: u32,
        /// Length of the region, in bytes.
        size: u32,
        /// Sum of every image byte, taken as a wrapping 32 bit total.
        checksum: u32,
    },

    /// Restarts the charger.
    Reboot,

    /// Sends a command word and payload the library does not model.
    ///
    /// The first byte is the command word. Use this to reach a firmware
    /// revision that speaks something the app did not.
    Raw {
        /// The payload bytes, exactly as sent or received.
        data: Vec<u8>,
    },
}

impl Request {
    /// The command word the charger dispatches on.
    pub fn command_word(&self) -> u8 {
        match self {
            Request::Identify => 0x00,
            Request::Bind { .. } => 0x18,
            Request::BattgoInfo { .. } => 0x40,
            Request::BattgoOem { .. } => 0x42,
            Request::BattgoRealState { .. } => 0x44,
            Request::BattgoWriteParameters { .. } => 0x46,
            Request::BattgoReadParameters { .. } => 0x48,
            Request::SmartPowerInfo => 0xBA,
            Request::SetSmartPower { .. } => 0xBC,
            Request::SmartPowerParameters => 0xBE,
            Request::SetName { .. } => 0xC0,
            Request::SetLimitParameters { .. } => 0xD0,
            Request::SetOneKeyLaunch { .. } => 0xD2,
            Request::OneKeyLaunch => 0xD4,
            Request::Calibrate8 { .. } => 0xDC,
            Request::Calibrate6 { .. } => 0xDE,
            Request::HardwareInfo => 0xE0,
            Request::LimitParameters => 0xE2,
            Request::Electrical { .. } => 0xE4,
            Request::WorkState { .. } => 0xE6,
            Request::Temperature { .. } => 0xE8,
            Request::SetTask { .. } => 0xEA,
            Request::EnterBootloader => 0xF0,
            Request::EraseApp { .. } => 0xF2,
            Request::WriteApp { .. } => 0xF4,
            Request::ChecksumApp { .. } => 0xF6,
            Request::InnerResistance { .. } => 0xFA,
            Request::Reboot => 0xFC,
            Request::Raw { data } => data.first().copied().unwrap_or(0),
        }
    }

    /// True when resending this request cannot do harm.
    ///
    /// A charger silently swallows the first control frame after a bind, and
    /// drops the occasional packet besides. The Android app absorbs both by
    /// keeping a user command at the head of its queue and resending it every
    /// tick until the acknowledgement arrives, so retrying is the protocol's
    /// normal behaviour rather than a workaround.
    ///
    /// Every query is safe to repeat, and so is every setter here, because
    /// each one writes an absolute value rather than applying a delta. Only
    /// the firmware operations and the reboot are excluded, along with
    /// [`Request::Raw`], whose meaning this library does not know.
    pub fn is_retryable(&self) -> bool {
        !matches!(
            self,
            Request::EnterBootloader
                | Request::EraseApp { .. }
                | Request::WriteApp { .. }
                | Request::ChecksumApp { .. }
                | Request::Reboot
                | Request::Raw { .. }
        )
    }

    /// The command word of the reply this request draws, when the reply is a
    /// single predictable packet.
    ///
    /// Replies are the request's command word plus one, except for
    /// [`Request::Identify`], which draws [`crate::Response::Identity`] and,
    /// on units that support it, [`crate::Response::BleMode`].
    pub fn reply_word(&self) -> Option<u8> {
        match self {
            Request::Identify | Request::Raw { .. } => None,
            other => Some(other.command_word().wrapping_add(1)),
        }
    }

    /// Builds the frame `DATA` field: the command word followed by arguments.
    pub fn data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        out.push(self.command_word());
        match self {
            Request::Identify
            | Request::HardwareInfo
            | Request::LimitParameters
            | Request::OneKeyLaunch
            | Request::SmartPowerInfo
            | Request::SmartPowerParameters => {}

            Request::Electrical { channel }
            | Request::WorkState { channel }
            | Request::Temperature { channel }
            | Request::InnerResistance { channel }
            | Request::BattgoInfo { channel }
            | Request::BattgoOem { channel }
            | Request::BattgoRealState { channel }
            | Request::BattgoReadParameters { channel } => out.push(*channel),

            Request::Bind { client_id } => out.extend_from_slice(client_id),

            Request::SetName { name } => {
                let mut padded = [0u8; 16];
                let bytes = name.as_bytes();
                let n = bytes.len().min(padded.len());
                padded[..n].copy_from_slice(&bytes[..n]);
                out.extend_from_slice(&padded);
            }

            Request::SetTask {
                channel,
                task,
                battery,
                link,
                work_current_ma,
                cell_count,
                full_charged_volt_mv,
            } => {
                out.push(*channel);
                out.push(*task as u8);
                out.push(battery.code());
                out.push(*link as u8);
                out.extend_from_slice(&work_current_ma.to_le_bytes());
                out.push(*cell_count);
                out.extend_from_slice(&full_charged_volt_mv.to_le_bytes());
            }

            Request::SetLimitParameters {
                min_input_volt_mv,
                max_input_power_mw,
            } => {
                out.extend_from_slice(&min_input_volt_mv.to_le_bytes());
                out.extend_from_slice(&max_input_power_mw.to_le_bytes());
            }

            Request::SetOneKeyLaunch {
                enabled,
                battery,
                cell_count,
                full_charged_volt_mv,
                work_current_ma,
            } => {
                out.push(u8::from(*enabled));
                out.push(battery.code());
                out.push(*cell_count);
                out.extend_from_slice(&full_charged_volt_mv.to_le_bytes());
                out.extend_from_slice(&work_current_ma.to_le_bytes());
            }

            Request::SetSmartPower { setting, value } => {
                out.extend_from_slice(&SMART_POWER_SAFE_WORD.to_le_bytes());
                out.push(*setting);
                out.extend_from_slice(&value.to_le_bytes());
            }

            Request::BattgoWriteParameters {
                channel,
                charging_current_ma,
                store_volt_mv,
                full_charged_volt_mv,
                self_discharging_days,
            } => {
                out.push(*channel);
                out.extend_from_slice(&charging_current_ma.to_le_bytes());
                out.extend_from_slice(&store_volt_mv.to_le_bytes());
                out.extend_from_slice(&full_charged_volt_mv.to_le_bytes());
                out.push(*self_discharging_days);
            }

            Request::Calibrate6 {
                channel,
                mode,
                cell_mv,
                input_mv,
                output_mv,
            } => {
                out.push(*channel);
                out.push(*mode as u8);
                for mv in cell_mv {
                    out.extend_from_slice(&mv.to_le_bytes());
                }
                out.extend_from_slice(&input_mv.to_le_bytes());
                out.extend_from_slice(&output_mv.to_le_bytes());
            }

            Request::Calibrate8 {
                channel,
                mode,
                cell_mv,
                input_mv,
                output_mv,
            } => {
                out.push(*channel);
                out.push(*mode as u8);
                for mv in cell_mv {
                    out.extend_from_slice(&mv.to_le_bytes());
                }
                out.extend_from_slice(&input_mv.to_le_bytes());
                out.extend_from_slice(&output_mv.to_le_bytes());
            }

            Request::EnterBootloader => out.push(BOOTLOADER_MAGIC),

            Request::EraseApp {
                start_address,
                size,
            } => {
                out.push(0x00); // CPU index; the app only ever targets 0.
                out.extend_from_slice(&start_address.to_le_bytes());
                out.extend_from_slice(&size.to_le_bytes());
            }

            Request::WriteApp {
                start_address,
                data,
            } => {
                out.push(0x00); // CPU index.
                out.extend_from_slice(&start_address.to_le_bytes());
                out.extend_from_slice(data.as_slice());
            }

            Request::ChecksumApp {
                start_address,
                size,
                checksum,
            } => {
                out.push(0x35); // Fixed sub-command the app sends.
                out.push(0x00); // CPU index.
                out.extend_from_slice(&start_address.to_le_bytes());
                out.extend_from_slice(&size.to_le_bytes());
                out.extend_from_slice(&checksum.to_le_bytes());
            }

            Request::Reboot => out.push(REBOOT_MAGIC),

            Request::Raw { data } => {
                out.clear();
                out.extend_from_slice(data);
            }
        }
        out
    }

    /// Builds the complete stuffed frame ready to hand to the transport.
    pub fn encode(&self) -> Result<Vec<u8>, crate::frame::FrameError> {
        crate::frame::encode(&self.data())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app hard-codes the checksum of each fixed request. Reproducing
    /// those literals proves the payload layouts match byte for byte.
    #[test]
    fn matches_the_checksums_baked_into_the_app() {
        let cases: [(Request, u8); 13] = [
            (Request::Identify, 0x13),
            (Request::HardwareInfo, 0xF3),
            (Request::LimitParameters, 0xF5),
            (Request::Electrical { channel: 0 }, 0xF8),
            (Request::WorkState { channel: 0 }, 0xFA),
            (Request::Temperature { channel: 0 }, 0xFC),
            (Request::InnerResistance { channel: 0 }, 0x0E),
            (Request::OneKeyLaunch, 0xE7),
            (Request::SmartPowerInfo, 0xCD),
            (Request::SmartPowerParameters, 0xD1),
            (Request::BattgoInfo { channel: 0 }, 0x54),
            (Request::EnterBootloader, 0xB0),
            (Request::Reboot, 0xDA),
        ];
        for (request, checksum) in cases {
            let frame = request.encode().unwrap();
            assert_eq!(*frame.last().unwrap(), checksum, "checksum for {request:?}");
        }
    }

    /// The app's per-channel requests fold the channel into the checksum.
    #[test]
    fn channel_shifts_the_checksum() {
        for (request, base) in [
            (Request::Electrical { channel: 1 }, 0xF8u8),
            (Request::BattgoOem { channel: 1 }, 0x56),
            (Request::BattgoRealState { channel: 1 }, 0x58),
            (Request::BattgoReadParameters { channel: 1 }, 0x5C),
        ] {
            let frame = request.encode().unwrap();
            assert_eq!(*frame.last().unwrap(), base.wrapping_add(1));
        }
    }

    /// Payload sizes come straight from the length byte the app writes.
    #[test]
    fn payload_lengths_match_the_app() {
        let cases: [(Request, usize); 8] = [
            (
                Request::SetTask {
                    channel: 0,
                    task: TaskType::Charge,
                    battery: BatteryKind::LiPo,
                    link: LinkType::Both,
                    work_current_ma: 1000,
                    cell_count: 4,
                    full_charged_volt_mv: 4200,
                },
                12,
            ),
            (
                Request::SetLimitParameters {
                    min_input_volt_mv: 10_000,
                    max_input_power_mw: 200_000,
                },
                7,
            ),
            (
                Request::SetOneKeyLaunch {
                    enabled: true,
                    battery: BatteryKind::LiPo,
                    cell_count: 4,
                    full_charged_volt_mv: 4200,
                    work_current_ma: 2000,
                },
                10,
            ),
            (
                Request::SetSmartPower {
                    setting: 1,
                    value: 20_000,
                },
                6,
            ),
            (
                Request::BattgoWriteParameters {
                    channel: 0,
                    charging_current_ma: 1000,
                    store_volt_mv: 3800,
                    full_charged_volt_mv: 4200,
                    self_discharging_days: 3,
                },
                11,
            ),
            (
                Request::Calibrate6 {
                    channel: 0,
                    mode: CalibrationMode::Calibrate,
                    cell_mv: [0; 6],
                    input_mv: 0,
                    output_mv: 0,
                },
                19,
            ),
            (
                Request::Calibrate8 {
                    channel: 0,
                    mode: CalibrationMode::Calibrate,
                    cell_mv: [0; 8],
                    input_mv: 0,
                    output_mv: 0,
                },
                23,
            ),
            (
                Request::ChecksumApp {
                    start_address: 0,
                    size: 0,
                    checksum: 0,
                },
                15,
            ),
        ];
        for (request, len) in cases {
            assert_eq!(request.data().len(), len, "length for {request:?}");
        }
    }

    #[test]
    fn firmware_block_is_the_apps_134_byte_payload() {
        let request = Request::WriteApp {
            start_address: 0x0800_4000,
            data: Box::new([0xFF; WRITE_APP_BLOCK]),
        };
        assert_eq!(request.data().len(), 134);
        // 396 is the app's own literal for the fixed part of this checksum:
        // address, length, command word and CPU index.
        let frame = request.encode().unwrap();
        assert_eq!(u16::from(0x12u8) + 0x86 + 0xF4, 396);
        assert_eq!(frame[2], 0x86, "the length byte the app writes");
    }

    #[test]
    fn set_task_lays_out_fields_in_order() {
        let data = Request::SetTask {
            channel: 1,
            task: TaskType::Discharge,
            battery: BatteryKind::LiFe,
            link: LinkType::BalanceOnly,
            work_current_ma: 0x0403_0201,
            cell_count: 6,
            full_charged_volt_mv: 0x0605,
        }
        .data();
        assert_eq!(
            data,
            vec![0xEA, 1, 2, 3, 2, 0x01, 0x02, 0x03, 0x04, 6, 0x05, 0x06]
        );
    }

    #[test]
    fn name_is_padded_and_truncated_to_sixteen_bytes() {
        assert_eq!(Request::SetName { name: "hi".into() }.data().len(), 17);
        assert_eq!(
            Request::SetName {
                name: "0123456789abcdefGGG".into()
            }
            .data()
            .len(),
            17
        );
    }

    #[test]
    fn only_the_irreversible_requests_refuse_a_retry() {
        // A charger swallows the first control frame after a bind, so the
        // ordinary commands must be resendable.
        for request in [
            Request::Identify,
            Request::HardwareInfo,
            Request::Electrical { channel: 0 },
            Request::WorkState { channel: 0 },
            Request::SetTask {
                channel: 0,
                task: TaskType::Charge,
                battery: BatteryKind::LiPo,
                link: LinkType::SerialOnly,
                work_current_ma: 1000,
                cell_count: 4,
                full_charged_volt_mv: 4200,
            },
            Request::SetName { name: "x".into() },
            Request::Bind { client_id: [0; 16] },
        ] {
            assert!(request.is_retryable(), "{request:?} should be retryable");
        }

        // Firmware work and the reboot must happen exactly once, and a raw
        // frame means something this library cannot reason about.
        for request in [
            Request::EnterBootloader,
            Request::EraseApp {
                start_address: 0,
                size: 0,
            },
            Request::WriteApp {
                start_address: 0,
                data: Box::new([0; WRITE_APP_BLOCK]),
            },
            Request::ChecksumApp {
                start_address: 0,
                size: 0,
                checksum: 0,
            },
            Request::Reboot,
            Request::Raw { data: vec![0xE4] },
        ] {
            assert!(!request.is_retryable(), "{request:?} must not be retried");
        }
    }

    #[test]
    fn replies_are_the_command_word_plus_one() {
        assert_eq!(Request::Electrical { channel: 0 }.reply_word(), Some(0xE5));
        assert_eq!(Request::WorkState { channel: 0 }.reply_word(), Some(0xE7));
        assert_eq!(Request::Reboot.reply_word(), Some(0xFD));
        assert_eq!(Request::Identify.reply_word(), None);
    }
}
