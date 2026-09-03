//! Enumerations carried on the wire.
//!
//! Every code here is taken from a constant or lookup table in the ISDT
//! Android app. Values the app has no name for are preserved as
//! `Unknown(u8)` rather than dropped, so an unfamiliar firmware still
//! round-trips.

use serde::{Deserialize, Serialize};

/// What a channel should do with the pack attached to it.
///
/// From `IsdtPackBleTaskSetReq.TASK_TYPE_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum TaskType {
    /// Charge to the configured per-cell voltage.
    Charge = 0,
    /// Charge or discharge to the storage voltage for the chemistry.
    Storage = 1,
    /// Discharge to the cutoff voltage.
    Discharge = 2,
    /// Halt the channel.
    Stop = 3,
}

impl TaskType {
    /// Decodes a task code, or `None` if the charger reported one the app
    /// does not define.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => TaskType::Charge,
            1 => TaskType::Storage,
            2 => TaskType::Discharge,
            3 => TaskType::Stop,
            _ => return None,
        })
    }
}

/// Which leads of the pack are connected.
///
/// From `IsdtBleProtocol.BATTERY_LINK_TYPE_*`. A pack wired through both the
/// main leads and the balance connector reports [`LinkType::Both`], which is
/// what per-cell readings and balancing require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum LinkType {
    /// Nothing connected.
    None = 0,
    /// Main leads only, no balance connector.
    SerialOnly = 1,
    /// Balance connector only.
    BalanceOnly = 2,
    /// Main leads and balance connector.
    Both = 3,
}

impl LinkType {
    /// Decodes a link code, or `None` for a value the app does not define.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => LinkType::None,
            1 => LinkType::SerialOnly,
            2 => LinkType::BalanceOnly,
            3 => LinkType::Both,
            _ => return None,
        })
    }
}

/// A battery chemistry as selected when starting a task.
///
/// The codes are the positions in `IsdtPackBase.batteryTypeList`, the list
/// backing the app's chemistry picker. The charger reports chemistry back on
/// the same scale, so this type decodes telemetry as well as writing tasks.
///
/// The app's CM1620 picker offers only LiHv, LiPo, LiFe and ULiHv, so the
/// other codes arrive from the charger rather than going out to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryKind {
    /// Lithium high voltage, 4.35 V per cell.
    LiHv,
    /// Lithium polymer, 4.20 V per cell.
    LiPo,
    /// Lithium ion, 4.10 V per cell.
    LiIon,
    /// Lithium iron phosphate, 3.65 V per cell.
    LiFe,
    /// Lead acid, 2.40 V per cell.
    Pb,
    /// Nickel metal hydride or nickel cadmium.
    NiMhCd,
    /// Ultra high voltage lithium, 4.45 V per cell.
    ULiHv,
    /// A chemistry code this library does not name.
    Unknown(u8),
}

impl BatteryKind {
    /// The wire code for this chemistry.
    pub fn code(self) -> u8 {
        match self {
            BatteryKind::LiHv => 0,
            BatteryKind::LiPo => 1,
            BatteryKind::LiIon => 2,
            BatteryKind::LiFe => 3,
            BatteryKind::Pb => 4,
            BatteryKind::NiMhCd => 5,
            BatteryKind::ULiHv => 6,
            BatteryKind::Unknown(code) => code,
        }
    }

    /// Decodes a chemistry code.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => BatteryKind::LiHv,
            1 => BatteryKind::LiPo,
            2 => BatteryKind::LiIon,
            3 => BatteryKind::LiFe,
            4 => BatteryKind::Pb,
            5 => BatteryKind::NiMhCd,
            6 => BatteryKind::ULiHv,
            other => BatteryKind::Unknown(other),
        }
    }

    /// The fully charged voltage per cell in millivolts, from
    /// `IsdtPackBase.batteryType2MaxVoltMap`.
    ///
    /// Nickel chemistries terminate on voltage delta rather than a fixed
    /// ceiling, so they have none.
    pub fn max_cell_mv(self) -> Option<u16> {
        Some(match self {
            BatteryKind::LiHv => 4350,
            BatteryKind::LiPo => 4200,
            BatteryKind::LiIon => 4100,
            BatteryKind::LiFe => 3650,
            BatteryKind::Pb => 2400,
            BatteryKind::ULiHv => 4450,
            BatteryKind::NiMhCd | BatteryKind::Unknown(_) => return None,
        })
    }

    /// The storage voltage per cell in millivolts, from
    /// `IsdtPackBase.batteryType2StoreVoltMap`.
    pub fn store_cell_mv(self) -> Option<u16> {
        Some(match self {
            BatteryKind::LiHv => 3850,
            BatteryKind::LiPo => 3800,
            BatteryKind::LiIon => 3700,
            BatteryKind::LiFe => 3300,
            BatteryKind::Pb | BatteryKind::NiMhCd | BatteryKind::ULiHv => return None,
            BatteryKind::Unknown(_) => return None,
        })
    }

    /// The discharge cutoff per cell in millivolts, from
    /// `IsdtPackBase.batteryType2DisVoltMap`.
    pub fn discharge_cell_mv(self) -> Option<u16> {
        Some(match self {
            BatteryKind::LiHv => 3400,
            BatteryKind::LiPo => 3300,
            BatteryKind::LiIon => 3200,
            BatteryKind::LiFe => 2900,
            BatteryKind::Pb => 1800,
            BatteryKind::NiMhCd => 900,
            BatteryKind::ULiHv => return None,
            BatteryKind::Unknown(_) => return None,
        })
    }

    /// The largest cell count the app offers for this chemistry, from
    /// `IsdtPackBase.batteryType2CellsNum`. Nickel packs are not series
    /// limited there and report none.
    pub fn max_cells(self) -> Option<u8> {
        Some(match self {
            BatteryKind::LiHv
            | BatteryKind::LiPo
            | BatteryKind::LiIon
            | BatteryKind::LiFe
            | BatteryKind::ULiHv => 6,
            BatteryKind::Pb => 12,
            BatteryKind::NiMhCd | BatteryKind::Unknown(_) => return None,
        })
    }

    /// The name the app shows for this chemistry.
    pub fn label(self) -> String {
        match self {
            BatteryKind::LiHv => "LiHv".into(),
            BatteryKind::LiPo => "LiPo".into(),
            BatteryKind::LiIon => "LiIon".into(),
            BatteryKind::LiFe => "LiFe".into(),
            BatteryKind::Pb => "Pb".into(),
            BatteryKind::NiMhCd => "NiMH/Cd".into(),
            BatteryKind::ULiHv => "ULiHv".into(),
            BatteryKind::Unknown(code) => format!("unknown ({code})"),
        }
    }
}

impl std::str::FromStr for BatteryKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(
            match s.to_ascii_lowercase().replace(['/', '-', '_'], "").as_str() {
                "lihv" => BatteryKind::LiHv,
                "lipo" => BatteryKind::LiPo,
                "liion" => BatteryKind::LiIon,
                "life" => BatteryKind::LiFe,
                "pb" | "lead" => BatteryKind::Pb,
                "nimh" | "nicd" | "nimhcd" => BatteryKind::NiMhCd,
                "ulihv" => BatteryKind::ULiHv,
                other => {
                    return Err(format!(
                        "unknown chemistry {other:?}, expected one of \
                     lihv, lipo, liion, life, pb, nimh, ulihv"
                    ))
                }
            },
        )
    }
}

/// A second chemistry table the app carries, `IsdtBleProtocol.batteryTypeMap`.
///
/// This is a different scale from [`BatteryKind`]: it starts at `Auto` and
/// carries a no-battery sentinel. The CM1620 screens never consult it, and
/// telemetry from a charger decodes with [`BatteryKind`], so reach for this
/// only if a device turns out to report on this scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportedBattery {
    /// The charger is detecting the chemistry itself.
    Auto,
    /// Lithium high voltage.
    LiHv,
    /// Lithium ion.
    LiIon,
    /// Lithium iron phosphate.
    LiFe,
    /// Nickel zinc.
    NiZn,
    /// Nothing attached.
    None,
    /// Nickel metal hydride or nickel cadmium.
    NiMhCd,
    /// A chemistry code this library does not name.
    Unknown(u8),
}

impl ReportedBattery {
    /// Decodes a reported chemistry code.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => ReportedBattery::Auto,
            1 => ReportedBattery::LiHv,
            2 => ReportedBattery::LiIon,
            3 => ReportedBattery::LiFe,
            4 => ReportedBattery::NiZn,
            5 => ReportedBattery::None,
            6 => ReportedBattery::NiMhCd,
            other => ReportedBattery::Unknown(other),
        }
    }

    /// The name the app shows for this chemistry.
    pub fn label(self) -> String {
        match self {
            ReportedBattery::Auto => "auto".into(),
            ReportedBattery::LiHv => "LiHv".into(),
            ReportedBattery::LiIon => "LiIon".into(),
            ReportedBattery::LiFe => "LiFe".into(),
            ReportedBattery::NiZn => "NiZn".into(),
            ReportedBattery::None => "no battery".into(),
            ReportedBattery::NiMhCd => "NiMH/Cd".into(),
            ReportedBattery::Unknown(code) => format!("unknown ({code})"),
        }
    }
}

/// What a charger channel is doing right now.
///
/// These are the twelve cases of `CM1620Activity.updateUIWorkState`, which the
/// app feeds straight from `IsdtPackBleWorkState.getWorkState()`. Several
/// distinct codes share a user-facing label, so [`ChargerState::label`] returns
/// the app's wording and the variant keeps the distinction.
///
/// Cell-slot chargers report on a different scale. See [`ChannelState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargerState {
    /// Idle.
    Standby,
    /// Waking a deeply discharged pack.
    Activating,
    /// Ramping the charge current up.
    CurrentRise,
    /// Constant current charging.
    ConstantCurrent,
    /// Constant voltage charging.
    ConstantVoltage,
    /// Balancing at the target voltage. The app calls this fast charge complete.
    ConstantVoltageBalance,
    /// Trickle balancing. The app calls this charge complete.
    TrickleBalance,
    /// Charging up to the storage voltage.
    StorageCharging,
    /// Discharging down to the storage voltage.
    StorageDischarging,
    /// Storage voltage reached.
    Stored,
    /// Discharging.
    Discharging,
    /// Discharge finished.
    Discharged,
    /// A state code this library does not name.
    Unknown(u8),
}

impl ChargerState {
    /// Decodes a charger state code.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => ChargerState::Standby,
            1 => ChargerState::Activating,
            2 => ChargerState::CurrentRise,
            3 => ChargerState::ConstantCurrent,
            4 => ChargerState::ConstantVoltage,
            5 => ChargerState::ConstantVoltageBalance,
            6 => ChargerState::TrickleBalance,
            7 => ChargerState::StorageCharging,
            8 => ChargerState::StorageDischarging,
            9 => ChargerState::Stored,
            10 => ChargerState::Discharging,
            11 => ChargerState::Discharged,
            other => ChargerState::Unknown(other),
        }
    }

    /// True while the channel is moving energy.
    ///
    /// These are the states in which the app shows a stop button rather than a
    /// start button.
    pub fn is_running(self) -> bool {
        !matches!(
            self,
            ChargerState::Standby
                | ChargerState::Stored
                | ChargerState::Discharged
                | ChargerState::Unknown(_)
        )
    }

    /// True once the task has finished and the charger has stopped working.
    pub fn is_complete(self) -> bool {
        matches!(
            self,
            ChargerState::ConstantVoltageBalance
                | ChargerState::TrickleBalance
                | ChargerState::Stored
                | ChargerState::Discharged
        )
    }

    /// The wording the app shows for this state.
    pub fn label(self) -> String {
        match self {
            ChargerState::Standby => "Standby".into(),
            ChargerState::Activating => "Activating".into(),
            ChargerState::CurrentRise
            | ChargerState::ConstantCurrent
            | ChargerState::ConstantVoltage
            | ChargerState::StorageCharging => "Charging".into(),
            ChargerState::ConstantVoltageBalance => "Fast charge completed".into(),
            ChargerState::TrickleBalance => "Charge completed".into(),
            ChargerState::StorageDischarging => "Storing".into(),
            ChargerState::Stored => "Stored".into(),
            ChargerState::Discharging => "Discharging".into(),
            ChargerState::Discharged => "Discharged".into(),
            ChargerState::Unknown(code) => format!("unknown ({code})"),
        }
    }
}

/// The other work-state table the app carries, `IsdtBleProtocol.stateMap`,
/// with the `channel_state_*` labels.
///
/// ISDT's cell-slot chargers report on this scale. A CM1620 does not, so
/// decode its work state with [`ChargerState`] and treat this as a fallback
/// reading for a device that turns out to disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelState {
    /// Nothing attached.
    NoBattery,
    /// A pack is attached and idle.
    BatteryPresent,
    /// A pack is attached with reversed polarity.
    BatteryReversed,
    /// Charging.
    Charging,
    /// Charge finished.
    Charged,
    /// Discharging.
    Discharging,
    /// Discharge finished.
    Discharged,
    /// Moving the pack to its storage voltage.
    Storing,
    /// Storage voltage reached.
    Stored,
    /// Running a charge and discharge cycle.
    Cycling,
    /// Cycling finished.
    Cycled,
    /// A state code this library does not name.
    Unknown(u8),
}

impl ChannelState {
    /// Decodes a channel state code.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => ChannelState::NoBattery,
            1 => ChannelState::BatteryPresent,
            2 => ChannelState::BatteryReversed,
            3 => ChannelState::Charging,
            4 => ChannelState::Charged,
            5 => ChannelState::Discharging,
            6 => ChannelState::Discharged,
            7 => ChannelState::Storing,
            8 => ChannelState::Stored,
            9 => ChannelState::Cycling,
            10 => ChannelState::Cycled,
            other => ChannelState::Unknown(other),
        }
    }

    /// True while the channel is moving energy.
    pub fn is_running(self) -> bool {
        matches!(
            self,
            ChannelState::Charging
                | ChannelState::Discharging
                | ChannelState::Storing
                | ChannelState::Cycling
        )
    }

    /// The label the app shows for this state.
    pub fn label(self) -> String {
        match self {
            ChannelState::NoBattery => "no battery".into(),
            ChannelState::BatteryPresent => "battery exist".into(),
            ChannelState::BatteryReversed => "battery reversed".into(),
            ChannelState::Charging => "charging".into(),
            ChannelState::Charged => "charged".into(),
            ChannelState::Discharging => "discharging".into(),
            ChannelState::Discharged => "discharged".into(),
            ChannelState::Storing => "storing".into(),
            ChannelState::Stored => "stored".into(),
            ChannelState::Cycling => "cycling".into(),
            ChannelState::Cycled => "cycled".into(),
            ChannelState::Unknown(code) => format!("unknown ({code})"),
        }
    }
}

/// Whether a calibration frame writes new constants or restores the factory set.
///
/// From `IsdtPackCalibrationReq.CALIBRATION_TYPE_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum CalibrationMode {
    /// Store the supplied reference voltages as the new constants.
    Calibrate = 0,
    /// Discard the stored constants and return to the factory set.
    RestoreDefaults = 255,
}

/// What kind of supply is feeding the charger.
///
/// From `IsdtPackBleSmartPowerInfo.POWER_TYPE_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerType {
    /// A BattGo smart pack.
    Battgo,
    /// An ISDT smart power supply.
    SmartPower,
    /// A supply code this library does not name.
    Unknown(u8),
}

impl PowerType {
    /// Decodes a supply code.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => PowerType::Battgo,
            1 => PowerType::SmartPower,
            other => PowerType::Unknown(other),
        }
    }
}

/// The sixteen fault bits a charger reports in its work state.
///
/// `CM1620Activity.errorCode2String` walks bits 0 through 15 and joins the
/// matching entries of the app's `work_state_errors` string array, so the
/// field is a mask and not an enumeration. Several bits can be set at once.
///
/// ISDT's other product families use different arrays for the same field.
/// These labels are the charger set.
pub const ERROR_FLAG_LABELS: [&str; 16] = [
    "output overcurrent",
    "output overvoltage",
    "input overvoltage",
    "input undervoltage",
    "input unstable voltage",
    "temperature anomaly",
    "charging timeout",
    "linking state destroyed",
    "battery cell overvoltage",
    "battery reversed",
    "balance charging unsupported",
    "battery node linking error",
    "output no battery",
    "unknown error",
    "unknown error 1",
    "unknown error 2",
];

/// A charger fault mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ErrorFlags(pub u16);

impl ErrorFlags {
    /// True when the charger reported no fault.
    pub fn is_clear(self) -> bool {
        self.0 == 0
    }

    /// The label of every set bit.
    pub fn labels(self) -> Vec<&'static str> {
        (0..16)
            .filter(|bit| self.0 & (1 << bit) != 0)
            .map(|bit| ERROR_FLAG_LABELS[bit])
            .collect()
    }
}

impl std::fmt::Display for ErrorFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_clear() {
            return write!(f, "none");
        }
        write!(f, "{}", self.labels().join(", "))
    }
}

/// Cell counts the app offers, one through eight.
pub const SERIES_COUNTS: std::ops::RangeInclusive<u8> = 1..=8;

/// Charge and discharge current the CM1620 screen accepts, in milliamps.
///
/// The app clamps the echoed current into this window before showing it.
pub const WORK_CURRENT_MA: std::ops::RangeInclusive<u32> = 100..=5000;

/// Cell counts the CM1620 charge sheet accepts.
pub const CM1620_CELLS: std::ops::RangeInclusive<u8> = 2..=16;

/// Input power ceiling the CM1620 settings screen accepts, in watts,
/// adjustable in steps of 50.
pub const MAX_INPUT_POWER_W: std::ops::RangeInclusive<u32> = 100..=1100;

/// Input undervoltage cutoff the CM1620 settings screen accepts, in volts.
pub const MIN_INPUT_VOLT_V: std::ops::RangeInclusive<u32> = 11..=70;

/// Largest internal resistance the app treats as a real measurement, in
/// milliohms. From `IsdtBleProtocol.MAX_INNER_RESISTANCE`.
pub const MAX_INNER_RESISTANCE: u16 = 6553;

/// The nickel chemistry termination deltas the app offers, in millivolts.
/// From `IsdtPackBase.nimhCutoffVoltList`.
pub const NIMH_CUTOFF_DELTAS_MV: [u8; 8] = [3, 4, 5, 6, 7, 8, 9, 10];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chemistry_codes_follow_the_apps_picker_order() {
        assert_eq!(BatteryKind::LiHv.code(), 0);
        assert_eq!(BatteryKind::NiMhCd.code(), 5);
        assert_eq!(BatteryKind::ULiHv.code(), 6);
        assert_eq!(BatteryKind::from_code(9), BatteryKind::Unknown(9));
    }

    #[test]
    fn reported_chemistry_uses_the_other_scale() {
        assert_eq!(ReportedBattery::from_code(0), ReportedBattery::Auto);
        assert_eq!(ReportedBattery::from_code(5), ReportedBattery::None);
    }

    #[test]
    fn charger_states_carry_the_apps_labels() {
        assert_eq!(ChargerState::from_code(0).label(), "Standby");
        assert_eq!(ChargerState::from_code(3).label(), "Charging");
        assert_eq!(ChargerState::from_code(6).label(), "Charge completed");
        assert_eq!(ChargerState::from_code(11).label(), "Discharged");
        assert!(ChargerState::from_code(3).is_running());
        assert!(!ChargerState::from_code(0).is_running());
        assert!(ChargerState::from_code(6).is_complete());
        assert_eq!(ChargerState::from_code(99), ChargerState::Unknown(99));
    }

    #[test]
    fn slot_charger_states_use_the_other_table() {
        assert_eq!(ChannelState::from_code(3).label(), "charging");
        assert_eq!(ChannelState::from_code(10).label(), "cycled");
    }

    #[test]
    fn fault_flags_decode_bit_by_bit() {
        assert!(ErrorFlags(0).is_clear());
        assert_eq!(ErrorFlags(0).to_string(), "none");
        // Bits 0 and 5 together, the way the app joins them.
        let flags = ErrorFlags(0b0010_0001);
        assert_eq!(
            flags.labels(),
            vec!["output overcurrent", "temperature anomaly"]
        );
        assert_eq!(ErrorFlags(1 << 9).to_string(), "battery reversed");
        assert_eq!(ErrorFlags(u16::MAX).labels().len(), 16);
    }

    #[test]
    fn chemistry_parses_from_the_command_line() {
        assert_eq!("LiPo".parse::<BatteryKind>().unwrap(), BatteryKind::LiPo);
        assert_eq!("nimh".parse::<BatteryKind>().unwrap(), BatteryKind::NiMhCd);
        assert_eq!(
            "NiMH/Cd".parse::<BatteryKind>().unwrap(),
            BatteryKind::NiMhCd
        );
        assert!("lifepo4".parse::<BatteryKind>().is_err());
    }

    #[test]
    fn cell_ceilings_match_the_apps_tables() {
        assert_eq!(BatteryKind::LiPo.max_cell_mv(), Some(4200));
        assert_eq!(BatteryKind::NiMhCd.max_cell_mv(), None);
        assert_eq!(BatteryKind::LiFe.store_cell_mv(), Some(3300));
        assert_eq!(BatteryKind::NiMhCd.discharge_cell_mv(), Some(900));
        assert_eq!(BatteryKind::Pb.max_cells(), Some(12));
    }
}
