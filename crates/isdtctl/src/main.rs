//! `isdtctl`: a command line front end for ISDT chargers over Bluetooth.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use isdt_charger::ble::{self, WriteChannel};
use isdt_charger::client::{default_poll_cycle, Telemetry, POLL_INTERVAL};
use isdt_charger::request::WRITE_APP_BLOCK;
use isdt_charger::response::Response;
use isdt_charger::tokens;
use isdt_charger::types::{
    BatteryKind, CalibrationMode, LinkType, TaskType, CM1620_CELLS, MAX_INPUT_POWER_W,
    MIN_INPUT_VOLT_V, WORK_CURRENT_MA,
};
use isdt_charger::{BleClient, Request};

#[derive(Parser)]
#[command(
    name = "isdtctl",
    about = "Talk to an ISDT charger over Bluetooth Low Energy",
    long_about = "Speaks the serial protocol the ISDT Android app uses, reimplemented \
                  from that app's packet classes. Targets the CM1620 and its relatives.",
    version
)]
struct Cli {
    /// Name or address fragment of the charger. Defaults to the strongest signal.
    #[arg(short, long, global = true)]
    device: Option<String>,

    /// How long to scan for a charger, in seconds.
    #[arg(long, global = true, default_value_t = 10)]
    scan_timeout: u64,

    /// How long to wait for each reply, in milliseconds.
    #[arg(long, global = true, default_value_t = 1500)]
    timeout: u64,

    /// Write on the wide FFF7 channel instead of the 20 byte FFF6 channel.
    #[arg(long, global = true)]
    wide: bool,

    /// Print machine-readable JSON instead of a table.
    #[arg(long, global = true)]
    json: bool,

    /// Client identifier to present, as 32 hex digits. Overrides the store.
    #[arg(long, global = true)]
    client_id: Option<String>,

    /// Where client identifiers are kept. Defaults to ~/.config/isdtctl/tokens.
    #[arg(long, global = true)]
    tokens: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List every ISDT charger in range.
    Scan,

    /// Read device identifier, versions, name and part number.
    Info,

    /// Read one full pass of telemetry.
    Status(ChannelArg),

    /// Poll telemetry until interrupted.
    Watch {
        #[command(flatten)]
        channel: ChannelArg,
        /// Milliseconds between packets. The app uses 150.
        #[arg(long, default_value_t = 150)]
        interval: u64,
    },

    /// Read per-cell internal resistance.
    Resistance(ChannelArg),

    /// Start charging.
    Charge(TaskArgs),

    /// Start discharging.
    Discharge(TaskArgs),

    /// Charge or discharge to the storage voltage.
    Storage(TaskArgs),

    /// Stop the task on a channel.
    Stop(ChannelArg),

    /// Read or write the input power ceiling and undervoltage cutoff.
    Limits {
        /// Input undervoltage cutoff, in volts. Writes when given with --power.
        #[arg(long)]
        min_volt: Option<u32>,
        /// Input power ceiling, in watts.
        #[arg(long)]
        power: Option<u32>,
    },

    /// Read or write the profile the charger runs on a button press.
    Onekey {
        /// Write the profile instead of reading it.
        #[arg(long)]
        set: bool,
        /// Whether the profile is active.
        #[arg(long)]
        enabled: bool,
        /// Chemistry: lihv, lipo, liion, life, pb, nimh or ulihv.
        #[arg(long, default_value = "lipo")]
        battery: BatteryKind,
        /// Cells in series.
        #[arg(long, default_value_t = 4)]
        cells: u8,
        /// Per-cell target voltage, in millivolts.
        #[arg(long, default_value_t = 4200)]
        volt_mv: u16,
        /// Charge current, in milliamps.
        #[arg(long, default_value_t = 1000)]
        current_ma: u32,
    },

    /// Rename the charger.
    Name {
        /// The new name, at most sixteen bytes.
        name: String,
    },

    /// Bind this host to the charger and remember the identifier.
    ///
    /// The charger must be in binding mode: its advertised name carries a 1 at
    /// offset four. Omit the identifier to have one generated.
    Bind {
        /// Thirty-two hex digits. Generated when omitted.
        client_id: Option<String>,
    },

    /// List the client identifiers this host has stored.
    Tokens,

    /// Open an interactive session that stays connected and bound.
    ///
    /// Every other command works inside it, without reconnecting each time.
    Shell,

    /// Read a BattGo smart battery.
    Battgo {
        #[command(subcommand)]
        what: BattgoCommand,
    },

    /// Read or write an attached ISDT smart power supply.
    Smartpower {
        #[command(subcommand)]
        what: SmartPowerCommand,
    },

    /// Write balance-port calibration constants, or restore the factory set.
    Calibrate {
        #[command(flatten)]
        channel: ChannelArg,
        /// Restore the factory constants and ignore the measured voltages.
        #[arg(long)]
        restore: bool,
        /// Measured cell voltages in millivolts. Six or eight values.
        #[arg(long, num_args = 6..=8, value_delimiter = ',')]
        cells_mv: Vec<u16>,
        /// Measured input voltage, in millivolts.
        #[arg(long, default_value_t = 0)]
        input_mv: u16,
        /// Measured output voltage, in millivolts.
        #[arg(long, default_value_t = 0)]
        output_mv: u16,
    },

    /// Write a firmware image. This can leave the charger unbootable.
    Flash {
        /// Path to the raw firmware image.
        image: std::path::PathBuf,
        /// Flash address to write at.
        #[arg(long, value_parser = parse_u32)]
        address: u32,
        /// Required. Confirms you accept the risk of an interrupted write.
        #[arg(long)]
        yes: bool,
    },

    /// Restart the charger.
    Reboot,

    /// Send a command word and payload the tool does not model.
    Raw {
        /// Hex bytes, command word first, for example "e4 00".
        bytes: String,
        /// Wait this many milliseconds for frames and print everything heard.
        #[arg(long, default_value_t = 1500)]
        listen: u64,
    },
}

#[derive(Args, Clone)]
struct ChannelArg {
    /// Which channel, counted from zero.
    #[arg(short, long, default_value_t = 0)]
    channel: u8,
}

#[derive(Args)]
struct TaskArgs {
    #[command(flatten)]
    channel: ChannelArg,

    /// Chemistry: lihv, lipo, liion, life, pb, nimh or ulihv.
    #[arg(short, long)]
    battery: BatteryKind,

    /// Cells in series.
    #[arg(short = 's', long)]
    cells: u8,

    /// Charge or discharge current, in milliamps.
    #[arg(short = 'i', long)]
    current_ma: u32,

    /// Per-cell target voltage, in millivolts. Defaults to the chemistry's own.
    #[arg(short = 'v', long)]
    volt_mv: Option<u16>,

    /// Which leads are connected: none, serial, balance or both. The app always
    /// sends serial.
    #[arg(long, default_value = "serial")]
    link: LinkArg,

    /// Skip the range checks the app applies.
    #[arg(long)]
    force: bool,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum LinkArg {
    None,
    Serial,
    Balance,
    Both,
}

impl From<LinkArg> for LinkType {
    fn from(value: LinkArg) -> Self {
        match value {
            LinkArg::None => LinkType::None,
            LinkArg::Serial => LinkType::SerialOnly,
            LinkArg::Balance => LinkType::BalanceOnly,
            LinkArg::Both => LinkType::Both,
        }
    }
}

#[derive(Subcommand)]
enum BattgoCommand {
    /// Read the pack's identity.
    Info(ChannelArg),
    /// Read the manufacturer profile.
    Oem(ChannelArg),
    /// Read live cell voltages, currents and fault counters.
    State(ChannelArg),
    /// Read the charge settings stored in the pack.
    Read(ChannelArg),
    /// Write the charge settings stored in the pack.
    Write {
        #[command(flatten)]
        channel: ChannelArg,
        /// Preferred charge current, in milliamps.
        #[arg(long)]
        current_ma: u32,
        /// Per-cell storage voltage, in millivolts.
        #[arg(long)]
        store_mv: u16,
        /// Per-cell fully charged voltage, in millivolts.
        #[arg(long)]
        full_mv: u16,
        /// Days of rest before the pack drops itself to storage voltage.
        #[arg(long)]
        rest_days: u8,
    },
}

#[derive(Subcommand)]
enum SmartPowerCommand {
    /// Read identity, ratings and live output.
    Info,
    /// Read the configured working point.
    Parameters,
    /// Write one setting. Setting 1 is output voltage in tenths of a volt.
    Set {
        /// Which parameter to write.
        setting: u8,
        /// The new value.
        value: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            // btleplug logs its own disconnect teardown at error level, which
            // is noise on every clean exit. RUST_LOG still overrides this.
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,btleplug=off".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let token_path = match &cli.tokens {
        Some(path) => path.clone(),
        None => tokens::default_path()?,
    };
    let store = tokens::Store::load(&token_path)?;

    match &cli.command {
        Command::Scan => return run_scan(&cli).await,
        Command::Tokens => return run_tokens(&store, &token_path),
        _ => {}
    }

    // Find the charger before deciding what to present, so the identifier can
    // be looked up by the peripheral the scan actually matched.
    let adapter = ble::adapter().await?;
    let device = ble::find(
        &adapter,
        cli.device.as_deref(),
        Duration::from_secs(cli.scan_timeout),
    )
    .await
    .context("could not reach a charger")?;
    eprintln!("connecting to {} ({})", device.label(), device.id);

    let explicit = cli.client_id.as_deref().map(tokens::parse).transpose()?;
    let stored = store.get(&device.id);

    // `bind` establishes the identifier itself, so it must not be bound for.
    let binding = matches!(cli.command, Command::Bind { .. });
    let client_id = match (binding, explicit, stored) {
        (true, _, _) => None,
        (_, Some(id), _) => Some(id),
        (_, None, Some(id)) => Some(id),
        _ => {
            bail!(
                "no client identifier known for this charger.\n\
                 A charger disconnects any client that has not bound within about \
                 five seconds, so every command needs one.\n\
                 Bind it with `isdtctl -d {} bind`, or pass one you already have \
                 with --client-id.",
                cli.device.as_deref().unwrap_or(&device.id)
            );
        }
    };

    let mut client = BleClient::connect(&device, write_channel(cli.wide))
        .await
        .context("could not connect")?;
    client.set_timeout(Duration::from_millis(cli.timeout));

    if let Some(id) = client_id {
        client.bind(id).await.context("binding failed")?;
        eprintln!("bound");
    }

    let result = match cli.command {
        Command::Shell => shell(&cli, &mut client, &device, &store, &token_path).await,
        _ => {
            run(
                &cli,
                &cli.command,
                cli.json,
                &mut client,
                &device,
                &store,
                &token_path,
            )
            .await
        }
    };
    let _ = client.disconnect().await;
    result
}

fn run_tokens(store: &tokens::Store, path: &std::path::Path) -> Result<()> {
    if store.is_empty() {
        println!("No client identifiers stored in {}.", path.display());
        return Ok(());
    }
    println!("{:<40} CLIENT ID", "CHARGER");
    for (id, token) in store.iter() {
        println!("{:<40} {}", id, tokens::to_hex(token));
    }
    Ok(())
}

async fn run_scan(cli: &Cli) -> Result<()> {
    let adapter = ble::adapter().await?;
    let found = ble::scan(&adapter, Duration::from_secs(cli.scan_timeout)).await?;
    if found.is_empty() {
        println!("No ISDT charger found.");
        return Ok(());
    }
    println!(
        "{:<10} {:<18} {:>5}  {:<9} ADDRESS",
        "MODEL", "NAME", "RSSI", "BINDING"
    );
    for device in found {
        let rssi = device
            .rssi
            .map(|r| r.to_string())
            .unwrap_or_else(|| "-".into());
        let parsed = device.isdt_name();
        let model = parsed.as_ref().map(|p| p.model.clone()).unwrap_or_default();
        let name = parsed
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| device.label());
        let binding = match parsed.as_ref().map(|p| p.binding_mode) {
            Some(true) => "waiting",
            Some(false) => "bound",
            None => "-",
        };
        println!(
            "{model:<10} {name:<18} {rssi:>5}  {binding:<9} {}",
            device.id
        );
    }
    Ok(())
}

async fn run(
    cli: &Cli,
    command: &Command,
    json: bool,
    client: &mut BleClient,
    device: &isdt_charger::Discovered,
    store: &tokens::Store,
    token_path: &std::path::Path,
) -> Result<()> {
    match command {
        Command::Scan | Command::Tokens | Command::Shell => {
            unreachable!("handled outside run")
        }

        Command::Info => {
            let info = client.hardware_info().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("Device        {}", info.device_id_string());
                if let Some(name) = info.device_name_string() {
                    println!("Name          {name}");
                }
                if let Some(pn) = info.part_number_string() {
                    println!("Part number   {pn}");
                }
                println!("Hardware      {}", version(&info.hardware_version));
                println!("Bootloader    {}", version(&info.bootloader_version));
                println!("Firmware      {}", version(&info.firmware_version));
            }
        }

        Command::Status(arg) => {
            let telemetry = client.telemetry(arg.channel).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&telemetry)?);
            } else {
                print_telemetry(&telemetry);
            }
        }

        Command::Watch { channel, interval } => {
            watch(
                client,
                channel.channel,
                Duration::from_millis(*interval),
                json,
            )
            .await?;
        }

        Command::Resistance(arg) => {
            let reading = client.inner_resistance(arg.channel).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&reading)?);
            } else {
                for (index, cell) in reading.cells_mohm().iter().enumerate() {
                    match cell {
                        Some(mohm) => println!("cell {}   {mohm:>7.1} mOhm", index + 1),
                        None => println!("cell {}         no reading", index + 1),
                    }
                }
            }
        }

        Command::Charge(args) => start(client, TaskType::Charge, args).await?,
        Command::Discharge(args) => start(client, TaskType::Discharge, args).await?,
        Command::Storage(args) => start(client, TaskType::Storage, args).await?,

        Command::Stop(arg) => {
            client.stop(arg.channel).await?;
            println!("Channel {} stopped.", arg.channel);
        }

        Command::Limits { min_volt, power } => match (min_volt, power) {
            (Some(volt), Some(watts)) => {
                if !MIN_INPUT_VOLT_V.contains(volt) {
                    bail!(
                        "input cutoff must be {} to {} V",
                        MIN_INPUT_VOLT_V.start(),
                        MIN_INPUT_VOLT_V.end()
                    );
                }
                if !MAX_INPUT_POWER_W.contains(watts) {
                    bail!(
                        "input power ceiling must be {} to {} W",
                        MAX_INPUT_POWER_W.start(),
                        MAX_INPUT_POWER_W.end()
                    );
                }
                client
                    .set_limits((volt * 1000) as u16, watts * 1000)
                    .await?;
                println!("Limits set to {volt} V cutoff, {watts} W ceiling.");
            }
            (None, None) => {
                let limits = client.limits().await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&limits)?);
                } else {
                    println!(
                        "Max input    {:>8.1} W  {:>8.2} A",
                        limits.max_input_power_mw as f64 / 1000.0,
                        limits.max_input_current_ma as f64 / 1000.0
                    );
                    println!(
                        "Max output   {:>8.1} W  {:>8.2} A",
                        limits.max_output_power_mw as f64 / 1000.0,
                        limits.max_output_current_ma as f64 / 1000.0
                    );
                }
            }
            _ => bail!("--min-volt and --power must be given together"),
        },

        Command::Onekey {
            set,
            enabled,
            battery,
            cells,
            volt_mv,
            current_ma,
        } => {
            if *set {
                client
                    .set_one_key_launch(*enabled, *battery, *cells, *volt_mv, *current_ma)
                    .await?;
                println!("One-key launch profile written.");
            } else {
                let profile = client.one_key_launch().await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&profile)?);
                } else {
                    println!("Enabled       {}", profile.enabled);
                    println!(
                        "Chemistry     {}",
                        BatteryKind::from_code(profile.battery_type).label()
                    );
                    println!("Cells         {}", profile.cell_count);
                    println!("Target        {} mV per cell", profile.full_charged_mv);
                    println!("Current       {} mA", profile.work_current_ma);
                }
            }
        }

        Command::Name { name } => {
            if name.len() > 16 {
                bail!(
                    "a charger name is at most sixteen bytes, this one is {}",
                    name.len()
                );
            }
            client.set_name(name).await?;
            println!("Renamed to {name}.");
        }

        Command::Bind { client_id } => {
            // Most specific source wins: the positional argument, then the
            // global override, then whatever is already stored for this
            // charger, and only then a fresh one.
            let (id, source) = match client_id
                .as_deref()
                .map(tokens::parse)
                .transpose()?
                .or(cli.client_id.as_deref().map(tokens::parse).transpose()?)
                .or_else(|| store.get(&device.id))
            {
                Some(id) => (id, "supplied"),
                None => (tokens::generate(), "generated"),
            };

            if source == "generated" {
                println!("Generated client identifier {}.", tokens::to_hex(&id));
            }

            // Nothing is stored unless the charger actually accepted it, so a
            // refusal cannot leave a token behind that does not work.
            client.bind(id).await?;
            println!("Bound.");

            let mut updated = store.clone();
            updated.set(&device.id, id);
            updated.save(token_path)?;
            println!("Saved {} to {}.", tokens::to_hex(&id), token_path.display());
        }

        Command::Battgo { what } => {
            let request = match what {
                BattgoCommand::Info(a) => Request::BattgoInfo { channel: a.channel },
                BattgoCommand::Oem(a) => Request::BattgoOem { channel: a.channel },
                BattgoCommand::State(a) => Request::BattgoRealState { channel: a.channel },
                BattgoCommand::Read(a) => Request::BattgoReadParameters { channel: a.channel },
                BattgoCommand::Write {
                    channel,
                    current_ma,
                    store_mv,
                    full_mv,
                    rest_days,
                } => Request::BattgoWriteParameters {
                    channel: channel.channel,
                    charging_current_ma: *current_ma,
                    store_volt_mv: *store_mv,
                    full_charged_volt_mv: *full_mv,
                    self_discharging_days: *rest_days,
                },
            };
            let reply = client.call(request).await?;
            println!("{}", serde_json::to_string_pretty(&reply)?);
        }

        Command::Smartpower { what } => {
            let request = match what {
                SmartPowerCommand::Info => Request::SmartPowerInfo,
                SmartPowerCommand::Parameters => Request::SmartPowerParameters,
                SmartPowerCommand::Set { setting, value } => Request::SetSmartPower {
                    setting: *setting,
                    value: *value,
                },
            };
            let reply = client.call(request).await?;
            println!("{}", serde_json::to_string_pretty(&reply)?);
        }

        Command::Calibrate {
            channel,
            restore,
            cells_mv,
            input_mv,
            output_mv,
        } => {
            let mode = if *restore {
                CalibrationMode::RestoreDefaults
            } else {
                CalibrationMode::Calibrate
            };
            let request = match cells_mv.len() {
                6 => Request::Calibrate6 {
                    channel: channel.channel,
                    mode,
                    cell_mv: cells_mv[..6].try_into().unwrap(),
                    input_mv: *input_mv,
                    output_mv: *output_mv,
                },
                8 => Request::Calibrate8 {
                    channel: channel.channel,
                    mode,
                    cell_mv: cells_mv[..8].try_into().unwrap(),
                    input_mv: *input_mv,
                    output_mv: *output_mv,
                },
                n => bail!("give six or eight cell voltages, not {n}"),
            };
            let reply = client.call(request).await?;
            println!("{}", serde_json::to_string_pretty(&reply)?);
        }

        Command::Flash {
            image,
            address,
            yes,
        } => {
            if !yes {
                bail!("writing firmware can leave the charger unbootable; pass --yes to proceed");
            }
            let bytes = std::fs::read(image)
                .with_context(|| format!("could not read {}", image.display()))?;
            if bytes.is_empty() || !bytes.len().is_multiple_of(WRITE_APP_BLOCK) {
                bail!(
                    "the image must be a whole number of {WRITE_APP_BLOCK} byte blocks, \
                     this one is {} bytes",
                    bytes.len()
                );
            }
            client
                .flash_firmware(*address, &bytes, |done, total| {
                    eprint!("\rwriting block {done} of {total}");
                })
                .await?;
            eprintln!();
            println!("Firmware written and verified.");
        }

        Command::Reboot => {
            client.reboot().await?;
            println!("Reboot sent.");
        }

        Command::Raw { bytes, listen } => {
            let data = parse_hex(bytes)?;
            if data.is_empty() {
                bail!("give at least a command word");
            }
            client.send(&Request::Raw { data }).await?;
            let deadline = tokio::time::Instant::now() + Duration::from_millis(*listen);
            loop {
                let left = deadline.saturating_duration_since(tokio::time::Instant::now());
                if left.is_zero() {
                    break;
                }
                match client.next_frame(left).await? {
                    Some(frame) => println!("{}", serde_json::to_string(&frame)?),
                    None => break,
                }
            }
        }
    }
    Ok(())
}

async fn start(client: &mut BleClient, task: TaskType, args: &TaskArgs) -> Result<()> {
    let volt_mv = match args.volt_mv {
        Some(mv) => mv,
        None => match task {
            TaskType::Storage => args.battery.store_cell_mv(),
            TaskType::Discharge => args.battery.discharge_cell_mv(),
            _ => args.battery.max_cell_mv(),
        }
        .with_context(|| {
            format!(
                "{} has no default voltage for this task, pass --volt-mv",
                args.battery.label()
            )
        })?,
    };

    if !args.force {
        if !WORK_CURRENT_MA.contains(&args.current_ma) {
            bail!(
                "current must be {} to {} mA, or pass --force",
                WORK_CURRENT_MA.start(),
                WORK_CURRENT_MA.end()
            );
        }
        if !CM1620_CELLS.contains(&args.cells) {
            bail!(
                "cell count must be {} to {}, or pass --force",
                CM1620_CELLS.start(),
                CM1620_CELLS.end()
            );
        }
    }

    client
        .start_task(
            args.channel.channel,
            task,
            args.battery,
            args.link.into(),
            args.current_ma,
            args.cells,
            volt_mv,
        )
        .await?;

    println!(
        "Channel {}: {} {}S {} at {:.2} A to {} mV per cell.",
        args.channel.channel,
        match task {
            TaskType::Charge => "charging",
            TaskType::Discharge => "discharging",
            TaskType::Storage => "storing",
            TaskType::Stop => "stopping",
        },
        args.cells,
        args.battery.label(),
        args.current_ma as f64 / 1000.0,
        volt_mv
    );
    Ok(())
}

async fn watch(client: &mut BleClient, channel: u8, interval: Duration, json: bool) -> Result<()> {
    let cycle = default_poll_cycle(channel);
    let mut index = 0usize;
    let mut shutdown = std::pin::pin!(tokio::signal::ctrl_c());

    loop {
        let request = cycle[index % cycle.len()].clone();
        index += 1;

        tokio::select! {
            _ = &mut shutdown => {
                eprintln!();
                return Ok(());
            }
            reply = client.call(request) => match reply {
                Ok(frame) => {
                    if json {
                        println!("{}", serde_json::to_string(&frame)?);
                    } else if let Some(line) = summarise(&frame) {
                        println!("{line}");
                    }
                }
                // A charger that does not implement a packet simply stays
                // quiet, so keep the rotation turning.
                Err(isdt_charger::ClientError::Timeout { .. }) => {}
                Err(e) => return Err(e.into()),
            }
        }

        tokio::time::sleep(interval.max(POLL_INTERVAL / 5)).await;
    }
}

fn summarise(frame: &Response) -> Option<String> {
    Some(match frame {
        Response::Electrical(e) => format!(
            "in {:.2} V {:.2} A   out {:.2} V {:.2} A   cells {}",
            e.input_mv as f64 / 1000.0,
            e.input_ma as f64 / 1000.0,
            e.output_mv as f64 / 1000.0,
            e.current_ma as f64 / 1000.0,
            {
                // Stop at the last connected cell, not the first gap.
                let last = e
                    .cell_mv
                    .iter()
                    .rposition(|mv| *mv > 0)
                    .map_or(0, |i| i + 1);
                e.cell_mv[..last]
                    .iter()
                    .map(|mv| {
                        if *mv == 0 {
                            "  -  ".to_string()
                        } else {
                            format!("{:.3}", *mv as f64 / 1000.0)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        ),
        Response::WorkState(w) => format!(
            "{} {}%  {} mAh  {} mWh  {}  faults: {}",
            w.state.label(),
            w.capacity_percent,
            w.capacity_done_mah,
            w.energy_done_mwh,
            format_elapsed(w.elapsed()),
            w.errors
                .map(|f| f.to_string())
                .unwrap_or_else(|| "n/a".into())
        ),
        Response::Temperature(t) => {
            format!("charger {} C   probe {} C", t.device_c, t.battery_c)
        }
        _ => return None,
    })
}

fn print_telemetry(t: &Telemetry) {
    let e = &t.electrical;
    let w = &t.work_state;
    println!("State         {}", w.state.label());
    println!("Chemistry     {} {}S", w.battery_type.label(), w.cell_count);
    println!("Progress      {}%", w.capacity_percent);
    println!("Elapsed       {}", format_elapsed(w.elapsed()));
    println!(
        "Delivered     {} mAh, {} mWh",
        w.capacity_done_mah, w.energy_done_mwh
    );
    println!(
        "Input         {:.2} V  {:.2} A  {:.1} W",
        e.input_mv as f64 / 1000.0,
        e.input_ma as f64 / 1000.0,
        e.input_power_mw() as f64 / 1000.0
    );
    println!(
        "Output        {:.2} V  {:.2} A  {:.1} W",
        e.output_mv as f64 / 1000.0,
        e.current_ma as f64 / 1000.0,
        e.output_power_mw() as f64 / 1000.0
    );
    println!(
        "Temperature   charger {} C, probe {} C",
        t.temperature.device_c, t.temperature.battery_c
    );
    println!(
        "Faults        {}",
        w.errors
            .map(|f| f.to_string())
            .unwrap_or_else(|| "not reported".into())
    );

    let resistance = t.resistance.cells_mohm();
    println!();
    println!("{:<6} {:>10} {:>12}", "CELL", "VOLTAGE", "RESISTANCE");
    // Show every cell position, so a gap reads as an unconnected cell rather
    // than looking like the numbering is off.
    let last_connected = e
        .cell_mv
        .iter()
        .rposition(|mv| *mv > 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    if last_connected == 0 {
        println!("(no cells detected)");
        return;
    }
    for (index, mv) in e.cell_mv.iter().take(last_connected).enumerate() {
        let ohm = resistance
            .get(index)
            .and_then(|r| *r)
            .map(|r| format!("{r:.1} mOhm"))
            .unwrap_or_else(|| "-".into());
        let volts = if *mv == 0 {
            "       -  ".to_string()
        } else {
            format!("{:>8.3} V", *mv as f64 / 1000.0)
        };
        println!("{:<6} {volts} {ohm:>12}", index + 1);
    }
}

fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

fn version(v: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3])
}

fn parse_u32(s: &str) -> Result<u32, String> {
    let s = s.trim();
    let parsed = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => s.parse(),
    };
    parsed.map_err(|e| e.to_string())
}

fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != ':')
        .collect();
    if !cleaned.len().is_multiple_of(2) {
        bail!("hex input must have an even number of digits");
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&cleaned[i..i + 2], 16)
                .with_context(|| format!("{:?} is not a hex byte", &cleaned[i..i + 2]))
        })
        .collect()
}

/// Which characteristic carries writes, per `--wide`.
fn write_channel(wide: bool) -> WriteChannel {
    if wide {
        WriteChannel::Wide
    } else {
        WriteChannel::Narrow
    }
}

// One line typed at the interactive prompt. `no_binary_name` lets clap parse
// the words directly, so every subcommand available on the command line is
// available here with the same syntax.
#[derive(Parser)]
#[command(
    name = "",
    no_binary_name = true,
    about = "Commands available in this session. Add --json to any read \
             command for machine-readable output.",
    disable_version_flag = true
)]
struct ShellLine {
    /// Print machine-readable JSON for this command.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

/// How long the session may sit idle before it pokes the charger.
///
/// The Android app polls continuously. Nothing here proves a bound charger
/// drops an idle client, but a cheap query costs little and keeps the link
/// demonstrably alive.
const KEEPALIVE: Duration = Duration::from_secs(20);

/// Runs commands against one connection until the user leaves.
async fn shell(
    cli: &Cli,
    client: &mut BleClient,
    device: &isdt_charger::Discovered,
    store: &tokens::Store,
    token_path: &std::path::Path,
) -> Result<()> {
    let name = device
        .isdt_name()
        .map(|n| format!("{} {}", n.model, n.name))
        .unwrap_or_else(|| device.label());

    println!("Connected to {name}, bound. The link stays open until you leave.");
    println!("Type a command, `help` for the list, or `exit`. Ctrl-D also leaves.");

    let mut editor = match rustyline::DefaultEditor::new() {
        Ok(editor) => Some(editor),
        Err(e) => bail!("could not start the interactive prompt: {e}"),
    };
    let prompt = format!("{name}> ");

    loop {
        // Read on a blocking thread while the keepalive runs on this one. The
        // editor travels with the task and comes back so its history survives.
        let mut read = tokio::task::spawn_blocking({
            let prompt = prompt.clone();
            let mut editor = editor.take().expect("editor is returned each pass");
            move || {
                let line = editor.readline(&prompt);
                (editor, line)
            }
        });

        let line = loop {
            tokio::select! {
                finished = &mut read => {
                    let (returned, line) = finished?;
                    editor = Some(returned);
                    break line;
                }
                _ = tokio::time::sleep(KEEPALIVE) => {
                    // A failure here means the link is gone, and there is no
                    // point letting the user type into a dead session.
                    if let Err(e) = client.work_state(0).await {
                        read.abort();
                        bail!("lost the charger while idle: {e}");
                    }
                }
            }
        };

        let line = match line {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => bail!("input error: {e}"),
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(editor) = editor.as_mut() {
            let _ = editor.add_history_entry(trimmed);
        }

        if matches!(trimmed, "exit" | "quit" | "q") {
            break;
        }

        let words = match shell_words::split(trimmed) {
            Ok(words) => words,
            Err(e) => {
                println!("could not read that line: {e}");
                continue;
            }
        };

        let parsed = match ShellLine::try_parse_from(words) {
            Ok(parsed) => parsed,
            // Covers help and version too, which clap reports as errors.
            Err(e) => {
                let _ = e.print();
                continue;
            }
        };

        match &parsed.command {
            Command::Shell => println!("Already in a session."),
            Command::Scan => {
                println!("Scanning would drop this connection. Leave the session first.")
            }
            Command::Tokens => run_tokens(store, token_path)?,
            // A failed command must not end the session.
            other => {
                let json = cli.json || parsed.json;
                if let Err(e) = run(cli, other, json, client, device, store, token_path).await {
                    println!("{e:#}");
                }
            }
        }
    }

    println!("Leaving the session.");
    Ok(())
}
