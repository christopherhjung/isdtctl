//! A desktop window for ISDT battery chargers.
//!
//! ```text
//! isdtgui
//! ```
//!
//! Scans for chargers, binds them, and keeps as many connected at once as you
//! like. Client identifiers are shared with `isdtcli`, so a charger bound in
//! either place is known to both.

mod backend;
mod theme;

use std::time::Duration;

use api::{BatteryKind, ChargerState, TaskType};
use futures::StreamExt;
use gpui::{
    actions, div, prelude::*, px, size, App, Application, Bounds, Context, ElementId, FocusHandle,
    KeyBinding, SharedString, Window, WindowBounds, WindowOptions,
};

use backend::{Backend, Command, DeviceId, Found, Reading, Status, TaskCommand, Update};
use theme::*;

/// How long between polled packets. The vendor application uses 150 ms.
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// Currents the window offers, in milliamps.
const CURRENTS: [u32; 8] = [100, 250, 500, 1000, 1500, 2000, 3000, 5000];

/// Chemistries the window offers, matching the vendor application's picker.
const CHEMISTRIES: [BatteryKind; 4] = [
    BatteryKind::LiHv,
    BatteryKind::LiPo,
    BatteryKind::LiFe,
    BatteryKind::ULiHv,
];

actions!(isdtgui, [CloseWindow]);

/// One charger the window is following.
struct Device {
    id: DeviceId,
    label: SharedString,
    firmware: SharedString,
    status: Status,
    reading: Reading,
    notice: Option<SharedString>,
    /// What the controls will send when a task is started.
    battery: BatteryKind,
    cells: u8,
    current_ma: u32,
}

impl Device {
    fn new(id: DeviceId) -> Self {
        Self {
            id,
            label: "Charger".into(),
            firmware: "".into(),
            status: Status::Connecting,
            reading: Reading::default(),
            notice: None,
            battery: BatteryKind::LiPo,
            cells: 4,
            current_ma: 1000,
        }
    }

    fn running(&self) -> bool {
        self.reading
            .work
            .as_ref()
            .is_some_and(|w| w.state.is_running())
    }

    fn connected(&self) -> bool {
        matches!(self.status, Status::Connected { .. })
    }
}

/// Everything the window draws.
struct ChargerWindow {
    /// Held so the window can receive key bindings such as cmd-w.
    focus: FocusHandle,
    backend: Backend,
    devices: Vec<Device>,
    selected: usize,
    /// Chargers the last scan turned up.
    found: Vec<Found>,
    scanning: bool,
    /// Whether the discovery panel is open.
    discovering: bool,
    message: Option<SharedString>,
}

impl ChargerWindow {
    fn new(
        backend: Backend,
        mut updates: futures::channel::mpsc::UnboundedReceiver<Update>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Set ISDT_GUI_TRACE to watch the charger half from a terminal. A
        // window cannot be read by a script, and this data path is worth being
        // able to check without one.
        let trace = std::env::var_os("ISDT_GUI_TRACE").is_some();

        cx.spawn(async move |this, cx| {
            while let Some(update) = updates.next().await {
                if trace {
                    eprintln!("{}", describe(&update));
                }
                let applied = this.update(cx, |window, cx| {
                    window.apply(update);
                    cx.notify();
                });
                if applied.is_err() {
                    break; // The window is gone.
                }
            }
        })
        .detach();

        // Open on the discovery panel, since with nothing connected there is
        // nothing else to show.
        backend.send(Command::Scan);

        Self {
            focus: cx.focus_handle(),
            backend,
            devices: Vec::new(),
            selected: 0,
            found: Vec::new(),
            scanning: false,
            discovering: true,
            message: None,
        }
    }

    fn device_mut(&mut self, id: &DeviceId) -> &mut Device {
        if let Some(index) = self.devices.iter().position(|d| &d.id == id) {
            return &mut self.devices[index];
        }
        self.devices.push(Device::new(id.clone()));
        self.devices.last_mut().expect("just pushed")
    }

    fn apply(&mut self, update: Update) {
        match update {
            Update::Scanning(active) => self.scanning = active,
            Update::Found(found) => self.found = found,
            Update::Message(text) => self.message = Some(text.into()),

            Update::Status(id, status) => {
                if matches!(status, Status::Closed) {
                    self.devices.retain(|d| d.id != id);
                    self.selected = self.selected.min(self.devices.len().saturating_sub(1));
                    if self.devices.is_empty() {
                        self.discovering = true;
                    }
                    return;
                }

                let first = self.devices.is_empty();
                let device = self.device_mut(&id);
                if let Status::Connected { info, label } = &status {
                    let v = info.firmware_version;
                    device.label = label.clone().into();
                    device.firmware = format!(
                        "{} firmware {}.{}.{}.{}",
                        info.device_id_string(),
                        v[0],
                        v[1],
                        v[2],
                        v[3]
                    )
                    .into();
                }
                device.status = status;

                if first {
                    // The first charger to arrive gets the foreground, and the
                    // discovery panel steps out of the way.
                    self.selected = 0;
                    self.discovering = false;
                }
            }

            Update::Reading(id, reading) => {
                let device = self.device_mut(&id);
                // Follow the charger rather than fighting it: when it reports a
                // task, the controls show what is actually running.
                if let Some(work) = &reading.work {
                    if work.state.is_running() {
                        device.battery = work.battery_type;
                        device.cells = work.cell_count;
                        device.current_ma = work.work_current_ma;
                    }
                }
                device.reading = *reading;
            }

            Update::Notice(id, text) => {
                self.device_mut(&id).notice = Some(text.into());
            }
        }
    }

    fn start(&mut self, task: TaskType) {
        let Some(device) = self.devices.get_mut(self.selected) else {
            return;
        };
        let volt_mv = match task {
            TaskType::Storage => device.battery.store_cell_mv(),
            TaskType::Discharge => device.battery.discharge_cell_mv(),
            _ => device.battery.max_cell_mv(),
        };
        let Some(volt_mv) = volt_mv else {
            device.notice = Some(
                format!(
                    "{} has no target voltage for that task",
                    device.battery.label()
                )
                .into(),
            );
            return;
        };
        let command = Command::Task(
            device.id.clone(),
            TaskCommand::Start {
                task,
                battery: device.battery,
                cells: device.cells,
                current_ma: device.current_ma,
                volt_mv,
            },
        );
        self.backend.send(command);
    }
}

impl Render for ChargerWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if self.discovering {
            self.discovery(cx).into_any_element()
        } else {
            self.detail(cx).into_any_element()
        };

        div()
            .track_focus(&self.focus)
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .flex()
            .flex_col()
            .size_full()
            .bg(BACKGROUND)
            .text_color(TEXT)
            .font_family("system-ui")
            .child(self.tabs(cx))
            .child(
                div()
                    .id("body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .p(px(16.))
                    .child(body),
            )
            .children(self.message.clone().map(|text| {
                div()
                    .mx(px(16.))
                    .mb(px(12.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .bg(PANEL)
                    .text_size(px(12.))
                    .text_color(MUTED)
                    .child(text)
            }))
    }
}

impl ChargerWindow {
    /// The device tab strip, with the button that opens discovery.
    fn tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let discovering = self.discovering;
        let selected = self.selected;

        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(6.))
            .px(px(16.))
            .py(px(10.))
            .bg(PANEL)
            .border_b_1()
            .border_color(BORDER)
            .children(
                self.devices
                    .iter()
                    .enumerate()
                    .map(|(index, device)| {
                        let dot = match &device.status {
                            Status::Connected { .. } if device.running() => ACCENT,
                            Status::Connected { .. } => GOOD,
                            Status::Connecting => WARN,
                            _ => BAD,
                        };
                        div()
                            .id(ElementId::Name(format!("tab-{}", device.id).into()))
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .bg(if !discovering && index == selected {
                                TRACK
                            } else {
                                PANEL
                            })
                            .hover(|s| s.bg(HOVER))
                            .child(div().size(px(8.)).rounded_full().bg(dot))
                            .child(device.label.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected = index;
                                this.discovering = false;
                                cx.notify();
                            }))
                    })
                    .collect::<Vec<_>>(),
            )
            .child(
                div()
                    .id("add-device")
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .bg(if discovering { ACCENT_DIM } else { TRACK })
                    .text_color(if discovering { ACCENT } else { TEXT })
                    .hover(|s| s.bg(HOVER))
                    .child("+ Add charger")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.discovering = true;
                        this.backend.send(Command::Scan);
                        cx.notify();
                    })),
            )
    }

    /// The panel that finds chargers and binds them.
    fn discovery(&self, cx: &mut Context<Self>) -> gpui::Div {
        let empty = self.found.is_empty() && !self.scanning;

        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Chargers in range"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(MUTED)
                            .child(if self.scanning {
                                "scanning".to_string()
                            } else {
                                format!("{} found", self.found.len())
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        button("scan-again", "Scan again", ACCENT, !self.scanning).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.backend.send(Command::Scan);
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(px(6.))
                    .bg(PANEL)
                    .text_size(px(11.))
                    .text_color(MUTED)
                    .child(
                        "A charger only accepts a new identifier while it is in binding \
                         mode. Put it there from the charger itself; the badge below says \
                         which ones are waiting.",
                    ),
            )
            .children(
                self.found
                    .iter()
                    .cloned()
                    .map(|found| self.found_row(found, cx))
                    .collect::<Vec<_>>(),
            )
            .when(empty, |this| {
                this.child(
                    div()
                        .p(px(14.))
                        .rounded(px(8.))
                        .bg(PANEL)
                        .text_size(px(12.))
                        .text_color(MUTED)
                        .child("Nothing in range. Check the charger is powered on and close by."),
                )
            })
    }

    /// One row of the discovery list.
    fn found_row(&self, found: Found, cx: &mut Context<Self>) -> gpui::Div {
        let connect_id = found.id.clone();
        let bind_id = found.id.clone();
        let signal = found
            .rssi
            .map(|r| format!("{r} dBm"))
            .unwrap_or_else(|| "-".into());

        let (badge_text, badge_colour) = if found.connected {
            ("connected", GOOD)
        } else if found.binding_mode == Some(true) {
            ("waiting to bind", WARN)
        } else if found.known {
            ("known", MUTED)
        } else if found.binding_mode.is_none() {
            ("name unreadable", MUTED)
        } else {
            ("not bound", MUTED)
        };
        // A charger whose name did not parse might still be waiting, so offer
        // the option rather than hiding it. One that is not waiting refuses,
        // and the refusal says so.
        let offer_bind = found.binding_mode != Some(false) && !found.connected;

        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .p(px(12.))
            .rounded(px(8.))
            .bg(PANEL)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().text_size(px(13.)).child(found.name.clone()))
                            .child(
                                div()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(TRACK)
                                    .text_size(px(10.))
                                    .text_color(badge_colour)
                                    .child(badge_text),
                            ),
                    )
                    .child(div().text_size(px(11.)).text_color(MUTED).child(
                        if found.model.is_empty() {
                            signal.clone()
                        } else {
                            format!("{} · {signal}", found.model)
                        },
                    )),
            )
            // Binding is only offered when the charger is actually waiting for
            // it, because one that is not will simply refuse.
            .when(offer_bind, |this| {
                this.child(
                    button(format!("bind-{}", found.id), "Bind", ACCENT, true).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.backend.send(Command::Bind(bind_id.clone()));
                            cx.notify();
                        }),
                    ),
                )
            })
            .when(found.known && !found.connected, |this| {
                this.child(
                    button(format!("connect-{}", found.id), "Connect", STORE, true).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.backend.send(Command::Connect(connect_id.clone()));
                            cx.notify();
                        }),
                    ),
                )
            })
    }

    /// Everything about the selected charger.
    fn detail(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(device) = self.devices.get(self.selected) else {
            return div().child(
                div()
                    .p(px(14.))
                    .rounded(px(8.))
                    .bg(PANEL)
                    .text_size(px(12.))
                    .text_color(MUTED)
                    .child("No charger connected. Use Add charger to find one."),
            );
        };

        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(self.title_row(device, cx))
            .child(state_row(device))
            .child(meters(device))
            .child(cells_panel(device))
            .child(self.controls(device, cx))
            .children(device.notice.clone().map(|text| {
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .bg(PANEL)
                    .text_size(px(12.))
                    .text_color(MUTED)
                    .child(text)
            }))
    }

    fn title_row(&self, device: &Device, cx: &mut Context<Self>) -> gpui::Div {
        let id = device.id.clone();
        let subtitle: SharedString = match &device.status {
            Status::Connected { .. } => device.firmware.clone(),
            Status::Connecting => "connecting".into(),
            Status::Lost(why) => why.clone().into(),
            Status::Closed => "disconnected".into(),
        };

        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(device.label.clone()),
                    )
                    .child(div().text_size(px(11.)).text_color(MUTED).child(subtitle)),
            )
            .child(
                button("disconnect", "Disconnect", NEUTRAL, true).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.backend.send(Command::Disconnect(id.clone()));
                        cx.notify();
                    },
                )),
            )
    }

    fn controls(&self, device: &Device, cx: &mut Context<Self>) -> gpui::Div {
        let running = device.running();
        let live = device.connected();

        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .rounded(px(8.))
            .bg(PANEL)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .children(CHEMISTRIES.iter().copied().map(|kind| {
                        chip(kind.label(), device.battery == kind, running).on_click(cx.listener(
                            move |this, _, _, cx| {
                                if let Some(d) = this.devices.get_mut(this.selected) {
                                    d.battery = kind;
                                }
                                cx.notify();
                            },
                        ))
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(label_cell("Cells"))
                    .children((1u8..=8).map(|n| {
                        chip(n.to_string(), device.cells == n, running).on_click(cx.listener(
                            move |this, _, _, cx| {
                                if let Some(d) = this.devices.get_mut(this.selected) {
                                    d.cells = n;
                                }
                                cx.notify();
                            },
                        ))
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(label_cell("Current"))
                    .children(CURRENTS.iter().copied().map(|ma| {
                        chip(
                            format!("{:.1}A", ma as f32 / 1000.0),
                            device.current_ma == ma,
                            running,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(d) = this.devices.get_mut(this.selected) {
                                d.current_ma = ma;
                            }
                            cx.notify();
                        }))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(
                        button("charge", "Charge", ACCENT, live && !running).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.start(TaskType::Charge);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        button("storage", "Storage", STORE, live && !running).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.start(TaskType::Storage);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button("discharge", "Discharge", WARN, live && !running).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.start(TaskType::Discharge);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button("stop", "Stop", BAD, live && running).on_click(cx.listener(
                            |this, _, _, cx| {
                                let id = this.devices.get(this.selected).map(|d| d.id.clone());
                                if let Some(id) = id {
                                    this.backend.send(Command::Task(id, TaskCommand::Stop));
                                }
                                cx.notify();
                            },
                        )),
                    ),
            )
    }
}

fn state_row(device: &Device) -> gpui::Div {
    let work = device.reading.work.as_ref();
    let state = work.map(|w| w.state).unwrap_or(ChargerState::Standby);
    let percent = work.map(|w| w.capacity_percent.min(100)).unwrap_or(0);
    let elapsed = work
        .map(|w| {
            let s = w.elapsed().as_secs();
            format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
        })
        .unwrap_or_else(|| "--:--:--".into());
    let faults = work
        .and_then(|w| w.errors)
        .filter(|f| !f.is_clear())
        .map(|f| f.to_string());

    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(24.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(if state.is_running() { ACCENT } else { TEXT })
                        .child(state.label()),
                )
                .child(div().text_size(px(13.)).text_color(MUTED).child(elapsed)),
        )
        // A progress bar reads faster than a number for something that moves
        // slowly over an hour.
        .child(
            div().w_full().h(px(8.)).rounded_full().bg(TRACK).child(
                div()
                    .h_full()
                    .w(gpui::relative(percent as f32 / 100.0))
                    .rounded_full()
                    .bg(if state.is_running() { ACCENT } else { MUTED }),
            ),
        )
        .children(faults.map(|text| {
            div()
                .px(px(10.))
                .py(px(6.))
                .rounded(px(6.))
                .bg(BAD_BG)
                .text_size(px(12.))
                .text_color(BAD)
                .child(text)
        }))
}

fn meters(device: &Device) -> gpui::Div {
    let e = device.reading.electrical.as_ref();
    let w = device.reading.work.as_ref();
    let t = device.reading.temperature.as_ref();

    let volts = |mv: Option<u32>| {
        mv.map(|mv| format!("{:.2} V", mv as f32 / 1000.0))
            .unwrap_or_else(|| "--".into())
    };
    let amps = |ma: Option<u32>| {
        ma.map(|ma| format!("{:.2} A", ma as f32 / 1000.0))
            .unwrap_or_else(|| "--".into())
    };

    div()
        .flex()
        .flex_wrap()
        .gap(px(10.))
        .child(meter(
            "Input",
            volts(e.map(|e| e.input_mv)),
            amps(e.map(|e| e.input_ma)),
        ))
        .child(meter(
            "Output",
            volts(e.map(|e| e.output_mv)),
            amps(e.map(|e| e.current_ma)),
        ))
        .child(meter(
            "Delivered",
            w.map(|w| format!("{} mAh", w.capacity_done_mah))
                .unwrap_or_else(|| "--".into()),
            w.map(|w| format!("{} mWh", w.energy_done_mwh))
                .unwrap_or_else(|| "--".into()),
        ))
        .child(meter(
            "Temperature",
            t.map(|t| format!("{} C", t.device_c))
                .unwrap_or_else(|| "--".into()),
            t.map(|t| format!("probe {} C", t.battery_c))
                .unwrap_or_else(|| "--".into()),
        ))
}

fn cells_panel(device: &Device) -> gpui::Div {
    let cells: Vec<u16> = device
        .reading
        .electrical
        .as_ref()
        .map(|e| e.cell_mv.clone())
        .unwrap_or_default();
    // Show up to the last connected cell, so a gap reads as an empty position
    // rather than as a numbering mistake.
    let last = cells.iter().rposition(|mv| *mv > 0).map_or(0, |i| i + 1);
    let resistance = device.reading.resistance.as_ref().map(|r| r.cells_mohm());

    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(12.))
        .rounded(px(8.))
        .bg(PANEL)
        .child(
            div()
                .text_size(px(11.))
                .text_color(MUTED)
                .child(if last == 0 {
                    "No cells detected".to_string()
                } else {
                    format!("{last} cells")
                }),
        )
        .children((0..last).map(|i| {
            let mv = cells[i];
            let ohm = resistance
                .and_then(|r| r.get(i).copied().flatten())
                .map(|r| format!("{r:.1} mOhm"))
                .unwrap_or_default();
            // 3.0 V empty to 4.3 V full covers every lithium chemistry here.
            let fill = ((mv as f32 - 3000.0) / 1300.0).clamp(0.0, 1.0);

            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .child(
                    div()
                        .w(px(20.))
                        .text_color(MUTED)
                        .child(format!("{}", i + 1)),
                )
                .child(
                    div().flex_1().h(px(6.)).rounded_full().bg(TRACK).child(
                        div()
                            .h_full()
                            .w(gpui::relative(if mv == 0 { 0.0 } else { fill }))
                            .rounded_full()
                            .bg(ACCENT),
                    ),
                )
                .child(div().w(px(62.)).child(if mv == 0 {
                    "--".to_string()
                } else {
                    format!("{:.3} V", mv as f32 / 1000.0)
                }))
                .child(div().w(px(70.)).text_color(MUTED).child(ohm))
        }))
}

/// A fixed-width label at the start of a control row.
fn label_cell(text: &'static str) -> gpui::Div {
    div()
        .w(px(48.))
        .text_size(px(11.))
        .text_color(MUTED)
        .child(text)
}

/// One labelled reading with a headline and a smaller second line.
fn meter(label: &'static str, primary: String, secondary: String) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .flex_1()
        .min_w(px(115.))
        .p(px(10.))
        .rounded(px(8.))
        .bg(PANEL)
        .child(div().text_size(px(11.)).text_color(MUTED).child(label))
        .child(div().text_size(px(17.)).child(primary))
        .child(div().text_size(px(11.)).text_color(MUTED).child(secondary))
}

/// A small toggle. Disabled while a task runs, since the charger owns the
/// settings then and changing them here would be a lie.
fn chip(label: String, selected: bool, disabled: bool) -> gpui::Stateful<gpui::Div> {
    let id: SharedString = format!("chip-{label}-{selected}").into();
    div()
        .id(ElementId::Name(id))
        .px(px(9.))
        .py(px(4.))
        .rounded(px(5.))
        .text_size(px(12.))
        .bg(if selected { ACCENT_DIM } else { TRACK })
        .text_color(if disabled {
            MUTED
        } else if selected {
            ACCENT
        } else {
            TEXT
        })
        .when(!disabled, |this| {
            this.cursor_pointer().hover(|s| s.bg(HOVER))
        })
        .child(label)
}

/// A button. Every one needs an identity of its own, because several rows of
/// the discovery list carry buttons with the same label.
fn button(
    id: impl Into<SharedString>,
    label: &'static str,
    colour: gpui::Rgba,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(ElementId::Name(id.into()))
        .px(px(13.))
        .py(px(6.))
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .bg(if enabled { colour } else { TRACK })
        .text_color(if !enabled {
            MUTED
        } else if colour == NEUTRAL {
            TEXT
        } else {
            BACKGROUND
        })
        .when(enabled, |this| {
            this.cursor_pointer().hover(|s| s.opacity(0.85))
        })
        .child(label)
}

/// A one-line summary of an update, for the trace.
fn describe(update: &Update) -> String {
    match update {
        Update::Scanning(true) => "scanning".into(),
        Update::Scanning(false) => "scan finished".into(),
        Update::Found(found) => format!(
            "found {}: [{}]",
            found.len(),
            found
                .iter()
                .map(|f| format!(
                    "{} {}{}{}",
                    f.name,
                    f.model,
                    match f.binding_mode {
                        Some(true) => " waiting",
                        Some(false) => "",
                        None => " name-unreadable",
                    },
                    if f.known { " known" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Update::Status(id, Status::Connected { info, label }) => format!(
            "{}: connected as {label}, {} firmware {:?}",
            short(id),
            info.device_id_string(),
            info.firmware_version
        ),
        Update::Status(id, Status::Connecting) => format!("{}: connecting", short(id)),
        Update::Status(id, Status::Lost(why)) => format!("{}: lost, {why}", short(id)),
        Update::Status(id, Status::Closed) => format!("{}: closed", short(id)),
        Update::Reading(id, r) => {
            let state = r
                .work
                .as_ref()
                .map(|w| w.state.label())
                .unwrap_or_else(|| "?".into());
            let out = r
                .electrical
                .as_ref()
                .map(|e| {
                    format!(
                        "{:.2} V {:.2} A",
                        e.output_mv as f32 / 1000.0,
                        e.current_ma as f32 / 1000.0
                    )
                })
                .unwrap_or_else(|| "no reading".into());
            format!("{}: {state}, out {out}", short(id))
        }
        Update::Notice(id, text) => format!("{}: {text}", short(id)),
        Update::Message(text) => text.clone(),
    }
}

/// The first field of a peripheral identifier, which is enough to tell two
/// chargers apart in a trace.
fn short(id: &str) -> &str {
    id.split('-').next().unwrap_or(id)
}

fn main() {
    let (backend, updates) = backend::start(POLL_INTERVAL);

    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([KeyBinding::new("cmd-w", CloseWindow, None)]);

        // Closing the last window leaves a macOS application running by
        // default. This is a single-window tool, so closing it should end the
        // process and with it the Bluetooth connections.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(560.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("ISDT Control".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let this = ChargerWindow::new(backend, updates, cx);
                    this.focus.focus(window);
                    this
                })
            },
        )
        .expect("the window should open");
        cx.activate(true);
    });
}
