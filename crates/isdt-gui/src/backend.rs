//! The charger half of the application, kept off the interface thread.
//!
//! gpui owns the main thread and runs its own executor; the Bluetooth backend
//! wants tokio. Rather than trying to marry the two, everything to do with
//! chargers lives on one background thread with its own runtime, and the two
//! halves exchange messages.
//!
//! One supervisor task owns the Bluetooth adapter and the token store. Each
//! connected charger gets its own task underneath it, so several can be live
//! at once and a failure on one leaves the others alone.

use std::collections::HashMap;
use std::time::Duration;

use futures::channel::mpsc as gui;
use isdt_charger::ble::{self, Discovered};
use isdt_charger::client::default_poll_cycle;
use isdt_charger::response::{Electrical, HardwareInfo, InnerResistance, Temperature, WorkState};
use isdt_charger::tokens::{self, ClientId};
use isdt_charger::{BatteryKind, BleClient, LinkType, Response, TaskType, WriteChannel};
use tokio::sync::mpsc as bg;

/// How long each scan listens for.
const SCAN_TIME: Duration = Duration::from_secs(4);

/// How long to wait before a dropped charger is retried.
const RECONNECT_DELAY: Duration = Duration::from_secs(3);

/// A charger's platform identifier. On macOS this is a system-assigned UUID.
pub type DeviceId = String;

/// Something the window would like done.
#[derive(Debug, Clone)]
pub enum Command {
    /// Look for chargers in range.
    Scan,
    /// Bind to a charger, then connect to it.
    ///
    /// The charger must be in binding mode. A fresh identifier is generated
    /// unless one is already stored for it.
    Bind(DeviceId),
    /// Connect to a charger this host already has an identifier for.
    Connect(DeviceId),
    /// Drop a charger and stop polling it.
    Disconnect(DeviceId),
    /// Ask one charger to do something.
    Task(DeviceId, TaskCommand),
}

/// Something one charger should do.
#[derive(Debug, Clone)]
pub enum TaskCommand {
    /// Start a task on channel zero.
    Start {
        /// Charge, discharge or storage.
        task: TaskType,
        /// The pack's chemistry.
        battery: BatteryKind,
        /// Cells in series.
        cells: u8,
        /// Working current, in milliamps.
        current_ma: u32,
        /// Per-cell target voltage, in millivolts.
        volt_mv: u16,
    },
    /// Stop whatever channel zero is doing.
    Stop,
}

/// Something the backend has to say.
#[derive(Debug)]
pub enum Update {
    /// A scan started or finished.
    Scanning(bool),
    /// What the last scan turned up.
    Found(Vec<Found>),
    /// A charger's connection state changed.
    Status(DeviceId, Status),
    /// A fresh reading from one charger.
    Reading(DeviceId, Box<Reading>),
    /// Something worth saying about one charger.
    Notice(DeviceId, String),
    /// Something worth saying that belongs to no charger.
    Message(String),
}

/// Where one charger's connection stands.
#[derive(Debug, Clone)]
pub enum Status {
    /// Working on it.
    Connecting,
    /// Up, and this is what answered.
    Connected {
        /// The charger's own name and versions.
        info: Box<HardwareInfo>,
        /// The name it advertises, which the user chose.
        label: String,
    },
    /// Gone. The backend keeps trying.
    Lost(String),
    /// Dropped on purpose, and not being retried.
    Closed,
}

/// A charger seen while scanning.
#[derive(Debug, Clone)]
pub struct Found {
    /// Platform identifier.
    pub id: DeviceId,
    /// The name the owner gave it, or the advertised name if unparsable.
    pub name: String,
    /// The model, such as `CM1620`. Empty when the name did not parse.
    pub model: String,
    /// Whether the charger will accept a new binding.
    ///
    /// `None` when the advertised name did not carry the ISDT structure, which
    /// happens when the name arrives truncated. Binding is still worth
    /// offering then: the charger refuses harmlessly if it is not waiting.
    pub binding_mode: Option<bool>,
    /// Signal strength from the last advertisement.
    pub rssi: Option<i16>,
    /// True when this host already has an identifier for it.
    pub known: bool,
    /// True when it is already connected.
    pub connected: bool,
}

/// The most recent value of each thing the window shows for one charger.
///
/// Readings arrive one packet at a time, so this accumulates rather than being
/// replaced wholesale, and a charger that never answers one query still leaves
/// the rest usable.
#[derive(Debug, Default, Clone)]
pub struct Reading {
    /// Voltages and currents.
    pub electrical: Option<Electrical>,
    /// Task state and progress.
    pub work: Option<WorkState>,
    /// Charger and probe temperatures.
    pub temperature: Option<Temperature>,
    /// Per-cell internal resistance.
    pub resistance: Option<InnerResistance>,
}

/// A handle to the charger thread.
pub struct Backend {
    commands: bg::UnboundedSender<Command>,
}

impl Backend {
    /// Asks the backend to do something. Fails only once the thread is gone.
    pub fn send(&self, command: Command) -> bool {
        self.commands.send(command).is_ok()
    }
}

/// Starts the charger thread and returns the handle and the update stream.
pub fn start(poll_interval: Duration) -> (Backend, gui::UnboundedReceiver<Update>) {
    let (commands, command_rx) = bg::unbounded_channel();
    let (updates, update_rx) = gui::unbounded();

    std::thread::Builder::new()
        .name("isdt-chargers".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    let _ = updates.unbounded_send(Update::Message(format!(
                        "could not start the charger runtime: {e}"
                    )));
                    return;
                }
            };
            runtime.block_on(supervise(poll_interval, command_rx, updates));
        })
        .expect("the charger thread should start");

    (Backend { commands }, update_rx)
}

/// One live charger, from the supervisor's point of view.
struct Live {
    /// Where to send this charger's commands.
    commands: bg::UnboundedSender<TaskCommand>,
    /// Dropping this asks the charger's task to stop.
    _task: tokio::task::JoinHandle<()>,
}

/// Owns the adapter and the token store, and runs one task per charger.
async fn supervise(
    poll_interval: Duration,
    mut commands: bg::UnboundedReceiver<Command>,
    updates: gui::UnboundedSender<Update>,
) {
    let adapter = match ble::adapter().await {
        Ok(adapter) => adapter,
        Err(e) => {
            let _ = updates.unbounded_send(Update::Message(e.to_string()));
            return;
        }
    };

    let token_path = tokens::default_path().ok();
    let mut store = token_path
        .as_deref()
        .and_then(|p| tokens::Store::load(p).ok())
        .unwrap_or_default();

    // Scan results are kept so a charger can be connected without rescanning.
    let mut seen: HashMap<DeviceId, Discovered> = HashMap::new();
    let mut live: HashMap<DeviceId, Live> = HashMap::new();

    while let Some(command) = commands.recv().await {
        match command {
            Command::Scan => {
                let _ = updates.unbounded_send(Update::Scanning(true));
                match ble::scan(&adapter, SCAN_TIME).await {
                    Ok(found) => {
                        seen.clear();
                        let mut list = Vec::new();
                        for device in found {
                            let parsed = device.isdt_name();
                            list.push(Found {
                                id: device.id.clone(),
                                name: parsed
                                    .as_ref()
                                    .map(|n| n.name.clone())
                                    .filter(|n| !n.is_empty())
                                    .unwrap_or_else(|| device.label()),
                                model: parsed.as_ref().map(|n| n.model.clone()).unwrap_or_default(),
                                binding_mode: parsed.map(|n| n.binding_mode),
                                rssi: device.rssi,
                                known: store.get(&device.id).is_some(),
                                connected: live.contains_key(&device.id),
                            });
                            seen.insert(device.id.clone(), device);
                        }
                        let _ = updates.unbounded_send(Update::Found(list));
                    }
                    Err(e) => {
                        let _ = updates.unbounded_send(Update::Message(e.to_string()));
                    }
                }
                let _ = updates.unbounded_send(Update::Scanning(false));
            }

            Command::Bind(id) => {
                let Some(device) = seen.get(&id).cloned() else {
                    let _ = updates.unbounded_send(Update::Message(
                        "that charger is no longer in the scan results; scan again".into(),
                    ));
                    continue;
                };
                // Reuse a stored identifier when there is one, so rebinding a
                // charger this host already knows does not orphan the old one.
                let client_id = store.get(&id).unwrap_or_else(tokens::generate);

                match bind(&device, client_id).await {
                    Ok(()) => {
                        store.set(&id, client_id);
                        let saved = match token_path.as_deref() {
                            Some(path) => match store.save(path) {
                                Ok(()) => format!("bound, identifier saved to {}", path.display()),
                                Err(e) => {
                                    format!("bound, but the identifier could not be saved: {e}")
                                }
                            },
                            None => "bound, but there is nowhere to save the identifier".into(),
                        };
                        let _ = updates.unbounded_send(Update::Notice(id.clone(), saved));
                        connect(&mut live, &device, client_id, poll_interval, &updates);
                    }
                    Err(e) => {
                        let _ = updates.unbounded_send(Update::Notice(id, e.to_string()));
                    }
                }
            }

            Command::Connect(id) => {
                let Some(device) = seen.get(&id).cloned() else {
                    let _ = updates.unbounded_send(Update::Message(
                        "that charger is no longer in the scan results; scan again".into(),
                    ));
                    continue;
                };
                let Some(client_id) = store.get(&id) else {
                    let _ = updates.unbounded_send(Update::Notice(
                        id,
                        "no identifier stored for this charger; bind it first".into(),
                    ));
                    continue;
                };
                connect(&mut live, &device, client_id, poll_interval, &updates);
            }

            Command::Disconnect(id) => {
                if let Some(entry) = live.remove(&id) {
                    entry._task.abort();
                }
                let _ = updates.unbounded_send(Update::Status(id, Status::Closed));
            }

            Command::Task(id, task) => {
                let Some(entry) = live.get(&id) else {
                    continue;
                };
                if entry.commands.send(task).is_err() {
                    live.remove(&id);
                }
            }
        }
    }
}

/// Binds one charger, without keeping the connection.
async fn bind(device: &Discovered, client_id: ClientId) -> Result<(), isdt_charger::ClientError> {
    let mut client = BleClient::connect(device, WriteChannel::default()).await?;
    client.bind(client_id).await?;
    let _ = client.disconnect().await;
    Ok(())
}

/// Starts a task for one charger, replacing any task already running for it.
fn connect(
    live: &mut HashMap<DeviceId, Live>,
    device: &Discovered,
    client_id: ClientId,
    poll_interval: Duration,
    updates: &gui::UnboundedSender<Update>,
) {
    if let Some(previous) = live.remove(&device.id) {
        previous._task.abort();
    }

    let (commands, command_rx) = bg::unbounded_channel();
    let device = device.clone();
    let updates = updates.clone();
    let id = device.id.clone();

    let task = tokio::spawn(async move {
        run_device(device, client_id, poll_interval, command_rx, updates).await;
    });

    live.insert(
        id,
        Live {
            commands,
            _task: task,
        },
    );
}

/// Connects, polls and reconnects one charger until its task is dropped.
async fn run_device(
    device: Discovered,
    client_id: ClientId,
    poll_interval: Duration,
    mut commands: bg::UnboundedReceiver<TaskCommand>,
    updates: gui::UnboundedSender<Update>,
) {
    let id = device.id.clone();
    let label = device
        .isdt_name()
        .map(|n| n.name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| device.label());

    loop {
        if updates
            .unbounded_send(Update::Status(id.clone(), Status::Connecting))
            .is_err()
        {
            return;
        }

        let client = BleClient::connect_bound(&device, WriteChannel::default(), client_id).await;

        let mut client = match client {
            Ok(client) => client,
            Err(e) => {
                if updates
                    .unbounded_send(Update::Status(id.clone(), Status::Lost(e.to_string())))
                    .is_err()
                {
                    return;
                }
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
        };

        match client.hardware_info().await {
            Ok(info) => {
                let status = Status::Connected {
                    info: Box::new(info),
                    label: label.clone(),
                };
                if updates
                    .unbounded_send(Update::Status(id.clone(), status))
                    .is_err()
                {
                    return;
                }
            }
            Err(e) => {
                let _ =
                    updates.unbounded_send(Update::Status(id.clone(), Status::Lost(e.to_string())));
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
        }

        if poll(&id, &mut client, poll_interval, &mut commands, &updates).await {
            return; // The window is gone.
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// Polls telemetry and services commands until the link fails.
///
/// Returns true when the window has closed and the task should stop for good.
async fn poll(
    id: &DeviceId,
    client: &mut BleClient,
    interval: Duration,
    commands: &mut bg::UnboundedReceiver<TaskCommand>,
    updates: &gui::UnboundedSender<Update>,
) -> bool {
    // The vendor application's rotation, minus the limits query, which is
    // static, and the second channel, which a single-channel charger ignores.
    let cycle: Vec<_> = default_poll_cycle(0)
        .into_iter()
        .filter(|r| r.command_word() != 0xE2)
        .collect();
    let mut next = 0usize;
    let mut reading = Reading::default();

    loop {
        // A command jumps the queue, exactly as it does in the application.
        if let Ok(command) = commands.try_recv() {
            match apply(client, command).await {
                Ok(message) => {
                    if updates
                        .unbounded_send(Update::Notice(id.clone(), message))
                        .is_err()
                    {
                        return true;
                    }
                }
                Err(e) => {
                    if updates
                        .unbounded_send(Update::Notice(id.clone(), e.to_string()))
                        .is_err()
                    {
                        return true;
                    }
                    if is_fatal(&e) {
                        let _ = updates.unbounded_send(Update::Status(
                            id.clone(),
                            Status::Lost(e.to_string()),
                        ));
                        return false;
                    }
                }
            }
            continue;
        }

        let request = cycle[next % cycle.len()].clone();
        next += 1;

        match client.call(request).await {
            Ok(response) => {
                absorb(&mut reading, response);
                // Send a snapshot once each full pass, so the window redraws at
                // a readable rate rather than per packet.
                if next.is_multiple_of(cycle.len())
                    && updates
                        .unbounded_send(Update::Reading(id.clone(), Box::new(reading.clone())))
                        .is_err()
                {
                    return true;
                }
            }
            Err(e) if is_fatal(&e) => {
                let _ =
                    updates.unbounded_send(Update::Status(id.clone(), Status::Lost(e.to_string())));
                return false;
            }
            // A charger that does not implement a packet simply stays quiet.
            Err(_) => {}
        }

        tokio::time::sleep(interval).await;
    }
}

/// True when an error means the link is gone rather than one lost frame.
fn is_fatal(error: &isdt_charger::ClientError) -> bool {
    matches!(
        error,
        isdt_charger::ClientError::Link(
            isdt_charger::LinkError::Closed | isdt_charger::LinkError::Transport(_)
        )
    )
}

/// Files a response into the accumulated reading.
fn absorb(reading: &mut Reading, response: Response) {
    match response {
        Response::Electrical(e) => reading.electrical = Some(e),
        Response::WorkState(w) => reading.work = Some(*w),
        Response::Temperature(t) => reading.temperature = Some(t),
        Response::InnerResistance(r) => reading.resistance = Some(r),
        _ => {}
    }
}

/// Carries out one command and describes what happened.
async fn apply(
    client: &mut BleClient,
    command: TaskCommand,
) -> Result<String, isdt_charger::ClientError> {
    match command {
        TaskCommand::Start {
            task,
            battery,
            cells,
            current_ma,
            volt_mv,
        } => {
            client
                .start_task(
                    0,
                    task,
                    battery,
                    // The vendor application sends this on every chemistry.
                    LinkType::SerialOnly,
                    current_ma,
                    cells,
                    volt_mv,
                )
                .await?;
            let verb = match task {
                TaskType::Charge => "charging",
                TaskType::Discharge => "discharging",
                TaskType::Storage => "storing",
                TaskType::Stop => "stopping",
            };
            Ok(format!(
                "{verb} {cells}S {} at {:.2} A",
                battery.label(),
                current_ma as f32 / 1000.0
            ))
        }
        TaskCommand::Stop => {
            client.stop(0).await?;
            Ok("stopped".into())
        }
    }
}
