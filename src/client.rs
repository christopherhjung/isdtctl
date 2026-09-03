//! Request and reply on top of the byte pipe.
//!
//! [`Client`] sends a [`Request`], waits for the frame whose command word is
//! the request's plus one, and hands it back typed. Unsolicited frames that
//! arrive while it waits are kept and can be drained with
//! [`Client::take_pending`], because a charger keeps talking once polling has
//! started.

use std::collections::VecDeque;
use std::time::Duration;

use crate::request::{Request, WRITE_APP_BLOCK};
use crate::response::{
    Electrical, HardwareInfo, InnerResistance, LimitParameters, OneKeyLaunch, Response,
    Temperature, WorkState,
};
use crate::tokens::ClientId;
use crate::transport::{self, Discovered, Transport, TransportError, WriteChannel};
use crate::types::{BatteryKind, LinkType, TaskType};

/// How long to wait for a reply before giving up.
///
/// The app allows a second per exchange (`BleMng3.COMMUNICATION_TIMEOUT`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1500);

/// How long the app leaves between polled packets (`BLE_DO_SLEEP_TIME`).
pub const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// How many extra times to resend a request that drew no reply.
///
/// A charger swallows the first control frame after a bind, and drops the odd
/// packet otherwise. The Android app resends a pending command on every tick
/// until it is acknowledged; this is the bounded version of that.
pub const RETRIES: usize = 3;

/// The rotation the app's CM1620 screen polls, in order.
///
/// One packet goes out per tick, so a full pass takes six ticks.
pub fn default_poll_cycle(channel: u8) -> Vec<Request> {
    vec![
        Request::Electrical { channel },
        Request::Temperature { channel },
        Request::WorkState { channel },
        Request::LimitParameters,
        Request::InnerResistance { channel: 0 },
        Request::InnerResistance { channel: 1 },
    ]
}

/// Something that went wrong talking to a charger.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The link failed.
    #[error(transparent)]
    Transport(#[from] TransportError),

    /// No reply arrived before the deadline.
    #[error("no reply to command 0x{command:02X} within {timeout:?}")]
    Timeout {
        /// The command word that went unanswered.
        command: u8,
        /// How long the client waited.
        timeout: Duration,
    },

    /// A reply arrived but was not the shape the request expected. This means
    /// the firmware disagrees with the app the protocol was read from.
    #[error("command 0x{command:02X} answered with an unexpected packet: {got:?}")]
    Unexpected {
        /// The command word that was sent.
        command: u8,
        /// What came back.
        got: Box<Response>,
    },

    /// The charger refused a task.
    #[error("charger rejected the task on channel {channel} with error code {code}")]
    TaskRejected {
        /// The channel the task was for.
        channel: u8,
        /// The charger's code. The app has no table for these beyond zero
        /// meaning success.
        code: u8,
    },

    /// The charger refused a setting write.
    #[error("charger rejected {what} with state {state}")]
    Rejected {
        /// Which write was refused.
        what: &'static str,
        /// The charger's status byte. Zero means success.
        state: u8,
    },

    /// The charger refused the client identifier.
    #[error(
        "the charger refused this client identifier; it is bound to a different \
         one. Put the charger into binding mode and bind again, or supply the \
         identifier it was originally bound with"
    )]
    BindRefused,

    /// A firmware image did not divide into whole blocks.
    #[error("firmware image must be a multiple of {WRITE_APP_BLOCK} bytes, got {0}")]
    BadImageLength(usize),
}

/// A connected charger.
pub struct Client {
    transport: Transport,
    timeout: Duration,
    pending: VecDeque<Response>,
}

impl Client {
    /// Scans for a charger and connects to the first match.
    ///
    /// `needle` matches case-insensitively against the advertised name and the
    /// peripheral identifier. Pass `None` to take the strongest signal.
    pub async fn discover(needle: Option<&str>, timeout: Duration) -> Result<Self, ClientError> {
        Self::discover_on(needle, timeout, WriteChannel::default()).await
    }

    /// Scans for a charger and connects on a chosen write channel.
    ///
    /// The wide channel falls back to the narrow one when the charger does not
    /// expose it.
    pub async fn discover_on(
        needle: Option<&str>,
        timeout: Duration,
        channel: WriteChannel,
    ) -> Result<Self, ClientError> {
        let adapter = transport::adapter().await?;
        let device = transport::find(&adapter, needle, timeout).await?;
        Self::connect(&device, channel).await
    }

    /// Connects to a charger found by [`transport::scan`].
    ///
    /// This does not bind. A charger drops an unbound client after about five
    /// seconds and answers almost nothing in the meantime, so prefer
    /// [`Client::connect_bound`] unless you are deliberately probing.
    pub async fn connect(device: &Discovered, channel: WriteChannel) -> Result<Self, ClientError> {
        Ok(Self {
            transport: Transport::connect(device, channel).await?,
            timeout: DEFAULT_TIMEOUT,
            pending: VecDeque::new(),
        })
    }

    /// Connects and immediately presents a client identifier.
    ///
    /// The Android app binds on every connection, between enabling
    /// notifications and its first query, and a charger will not answer much
    /// until it has. Without this the link is dropped mid-conversation.
    pub async fn connect_bound(
        device: &Discovered,
        channel: WriteChannel,
        client_id: ClientId,
    ) -> Result<Self, ClientError> {
        let mut client = Self::connect(device, channel).await?;
        client.bind(client_id).await?;
        Ok(client)
    }

    /// Scans for a charger, connects and binds in one step.
    pub async fn discover_bound(
        needle: Option<&str>,
        timeout: Duration,
        channel: WriteChannel,
        client_id: ClientId,
    ) -> Result<Self, ClientError> {
        let adapter = transport::adapter().await?;
        let device = transport::find(&adapter, needle, timeout).await?;
        Self::connect_bound(&device, channel, client_id).await
    }

    /// Presents a client identifier and returns once the charger accepts it.
    ///
    /// A charger that already holds a different token refuses, and the only
    /// way back is to put it into binding mode and bind again.
    pub async fn bind(&mut self, client_id: ClientId) -> Result<(), ClientError> {
        match self.call(Request::Bind { client_id }).await? {
            Response::Bind { bound: true } => Ok(()),
            Response::Bind { bound: false } => Err(ClientError::BindRefused),
            got => Err(unexpected(0x18, got)),
        }
    }

    /// Sets how long to wait for a reply.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Sends a request without waiting for a reply.
    pub async fn send(&self, request: &Request) -> Result<(), ClientError> {
        Ok(self.transport.send(&request.data()).await?)
    }

    /// Sends a request and waits for the frame that answers it.
    ///
    /// Frames that arrive first are queued for [`Client::take_pending`]. A
    /// request that draws no reply is resent up to [`RETRIES`] times, unless
    /// it is one of the few that must not be repeated. See
    /// [`Request::is_retryable`].
    pub async fn call(&mut self, request: Request) -> Result<Response, ClientError> {
        let attempts = if request.is_retryable() {
            RETRIES + 1
        } else {
            1
        };
        for attempt in 1..attempts {
            match self.call_once(request.clone()).await {
                Err(ClientError::Timeout { command, .. }) => {
                    tracing::debug!(
                        "no reply to command 0x{command:02X} on attempt {attempt}, resending"
                    );
                }
                other => return other,
            }
        }
        self.call_once(request).await
    }

    async fn call_once(&mut self, request: Request) -> Result<Response, ClientError> {
        let command = request.command_word();
        let want = request.reply_word();
        self.transport.send(&request.data()).await?;

        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return Err(ClientError::Timeout {
                    command,
                    timeout: self.timeout,
                });
            }
            let Some(frame) = self.transport.recv_timeout(left).await else {
                return Err(ClientError::Timeout {
                    command,
                    timeout: self.timeout,
                });
            };
            let Some(parsed) = Response::parse(&frame) else {
                continue;
            };
            match want {
                // Identify draws whatever the charger feels like answering.
                None => return Ok(parsed),
                Some(word) if parsed.command_word() == word => return Ok(parsed),
                Some(_) => self.pending.push_back(parsed),
            }
        }
    }

    /// Waits for the next unsolicited frame, up to `timeout`.
    pub async fn next_frame(&mut self, timeout: Duration) -> Option<Response> {
        if let Some(queued) = self.pending.pop_front() {
            return Some(queued);
        }
        let frame = self.transport.recv_timeout(timeout).await?;
        Response::parse(&frame)
    }

    /// Takes every frame received while waiting for replies.
    pub fn take_pending(&mut self) -> Vec<Response> {
        self.pending.drain(..).collect()
    }

    /// Closes the link.
    pub async fn disconnect(self) -> Result<(), ClientError> {
        Ok(self.transport.disconnect().await?)
    }

    // ---- typed queries ---------------------------------------------------

    /// Reads device identifier, versions, name and part number.
    pub async fn hardware_info(&mut self) -> Result<HardwareInfo, ClientError> {
        match self.call(Request::HardwareInfo).await? {
            Response::HardwareInfo(info) => Ok(*info),
            got => Err(unexpected(0xE0, got)),
        }
    }

    /// Reads live voltages and currents for a channel.
    pub async fn electrical(&mut self, channel: u8) -> Result<Electrical, ClientError> {
        match self.call(Request::Electrical { channel }).await? {
            Response::Electrical(e) => Ok(e),
            got => Err(unexpected(0xE4, got)),
        }
    }

    /// Reads task state and progress for a channel.
    pub async fn work_state(&mut self, channel: u8) -> Result<WorkState, ClientError> {
        match self.call(Request::WorkState { channel }).await? {
            Response::WorkState(w) => Ok(*w),
            got => Err(unexpected(0xE6, got)),
        }
    }

    /// Reads temperatures for a channel.
    pub async fn temperature(&mut self, channel: u8) -> Result<Temperature, ClientError> {
        match self.call(Request::Temperature { channel }).await? {
            Response::Temperature(t) => Ok(t),
            got => Err(unexpected(0xE8, got)),
        }
    }

    /// Reads per-cell internal resistance for a channel.
    pub async fn inner_resistance(&mut self, channel: u8) -> Result<InnerResistance, ClientError> {
        match self.call(Request::InnerResistance { channel }).await? {
            Response::InnerResistance(ir) => Ok(ir),
            got => Err(unexpected(0xFA, got)),
        }
    }

    /// Reads the power and current ceilings.
    pub async fn limits(&mut self) -> Result<LimitParameters, ClientError> {
        match self.call(Request::LimitParameters).await? {
            Response::LimitParameters(l) => Ok(l),
            got => Err(unexpected(0xE2, got)),
        }
    }

    /// Reads the button-press profile.
    pub async fn one_key_launch(&mut self) -> Result<OneKeyLaunch, ClientError> {
        match self.call(Request::OneKeyLaunch).await? {
            Response::OneKeyLaunch(o) => Ok(o),
            got => Err(unexpected(0xD4, got)),
        }
    }

    /// Reads everything the app's charger screen shows in one pass.
    pub async fn telemetry(&mut self, channel: u8) -> Result<Telemetry, ClientError> {
        Ok(Telemetry {
            electrical: self.electrical(channel).await?,
            temperature: self.temperature(channel).await?,
            work_state: self.work_state(channel).await?,
            limits: self.limits().await?,
            resistance: self.inner_resistance(channel).await?,
        })
    }

    // ---- typed commands ---------------------------------------------------

    /// Starts a task on a channel and checks the charger accepted it.
    ///
    /// `link` is [`LinkType::SerialOnly`] in the app for every task it starts,
    /// on every chemistry. Pass something else only if you have reason to.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_task(
        &mut self,
        channel: u8,
        task: TaskType,
        battery: BatteryKind,
        link: LinkType,
        work_current_ma: u32,
        cell_count: u8,
        full_charged_volt_mv: u16,
    ) -> Result<(), ClientError> {
        let reply = self
            .call(Request::SetTask {
                channel,
                task,
                battery,
                link,
                work_current_ma,
                cell_count,
                full_charged_volt_mv,
            })
            .await?;
        match reply {
            Response::TaskAck { error_code: 0, .. } => Ok(()),
            Response::TaskAck {
                channel,
                error_code,
            } => Err(ClientError::TaskRejected {
                channel,
                code: error_code,
            }),
            got => Err(unexpected(0xEA, got)),
        }
    }

    /// Stops the task on a channel.
    ///
    /// The remaining task fields are ignored by the charger for a stop, so
    /// this sends the same placeholders the app does.
    pub async fn stop(&mut self, channel: u8) -> Result<(), ClientError> {
        self.start_task(
            channel,
            TaskType::Stop,
            BatteryKind::LiHv,
            LinkType::SerialOnly,
            0,
            0,
            0,
        )
        .await
    }

    /// Renames the charger.
    pub async fn set_name(&mut self, name: &str) -> Result<(), ClientError> {
        match self
            .call(Request::SetName {
                name: name.to_string(),
            })
            .await?
        {
            Response::NameAck { state: 0 } => Ok(()),
            Response::NameAck { state } => Err(ClientError::Rejected {
                what: "the new name",
                state,
            }),
            got => Err(unexpected(0xC0, got)),
        }
    }

    /// Writes the input power ceiling and undervoltage cutoff.
    pub async fn set_limits(
        &mut self,
        min_input_volt_mv: u16,
        max_input_power_mw: u32,
    ) -> Result<(), ClientError> {
        match self
            .call(Request::SetLimitParameters {
                min_input_volt_mv,
                max_input_power_mw,
            })
            .await?
        {
            Response::LimitParametersAck { state: 0 } => Ok(()),
            Response::LimitParametersAck { state } => Err(ClientError::Rejected {
                what: "the power limits",
                state,
            }),
            got => Err(unexpected(0xD0, got)),
        }
    }

    /// Writes the button-press profile.
    pub async fn set_one_key_launch(
        &mut self,
        enabled: bool,
        battery: BatteryKind,
        cell_count: u8,
        full_charged_volt_mv: u16,
        work_current_ma: u32,
    ) -> Result<(), ClientError> {
        match self
            .call(Request::SetOneKeyLaunch {
                enabled,
                battery,
                cell_count,
                full_charged_volt_mv,
                work_current_ma,
            })
            .await?
        {
            Response::OneKeyLaunchAck { state: 0 } => Ok(()),
            Response::OneKeyLaunchAck { state } => Err(ClientError::Rejected {
                what: "the one-key launch profile",
                state,
            }),
            got => Err(unexpected(0xD2, got)),
        }
    }

    /// Restarts the charger. The link drops immediately afterwards.
    pub async fn reboot(&mut self) -> Result<(), ClientError> {
        match self.call(Request::Reboot).await {
            Ok(_) | Err(ClientError::Timeout { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Writes a firmware image: enter the bootloader, erase, write, verify.
    ///
    /// The image must be a whole number of [`WRITE_APP_BLOCK`] byte blocks.
    /// `progress` is called with the number of blocks written so far and the
    /// total. This rewrites the charger's firmware and a failure part way
    /// through can leave it unbootable.
    pub async fn flash_firmware(
        &mut self,
        start_address: u32,
        image: &[u8],
        mut progress: impl FnMut(usize, usize),
    ) -> Result<(), ClientError> {
        if image.is_empty() || !image.len().is_multiple_of(WRITE_APP_BLOCK) {
            return Err(ClientError::BadImageLength(image.len()));
        }

        match self.call(Request::EnterBootloader).await? {
            Response::BootloaderAck { state: 0 } => {}
            Response::BootloaderAck { state } => {
                return Err(ClientError::Rejected {
                    what: "entering the bootloader",
                    state,
                })
            }
            got => return Err(unexpected(0xF0, got)),
        }

        let size = image.len() as u32;
        match self
            .call(Request::EraseApp {
                start_address,
                size,
            })
            .await?
        {
            Response::EraseAck { result: 0, .. } => {}
            Response::EraseAck { result, .. } => {
                return Err(ClientError::Rejected {
                    what: "erasing the firmware region",
                    state: result,
                })
            }
            got => return Err(unexpected(0xF2, got)),
        }

        let total = image.len() / WRITE_APP_BLOCK;
        for (index, block) in image.chunks_exact(WRITE_APP_BLOCK).enumerate() {
            let offset = (index * WRITE_APP_BLOCK) as u32;
            let mut data = Box::new([0u8; WRITE_APP_BLOCK]);
            data.copy_from_slice(block);
            match self
                .call(Request::WriteApp {
                    start_address: start_address + offset,
                    data,
                })
                .await?
            {
                Response::WriteAck { state: 0, .. } => {}
                Response::WriteAck { state, .. } => {
                    return Err(ClientError::Rejected {
                        what: "writing a firmware block",
                        state,
                    })
                }
                got => return Err(unexpected(0xF4, got)),
            }
            progress(index + 1, total);
        }

        let checksum = image
            .iter()
            .fold(0u32, |acc, b| acc.wrapping_add(u32::from(*b)));
        match self
            .call(Request::ChecksumApp {
                start_address,
                size,
                checksum,
            })
            .await?
        {
            Response::ChecksumAck { state: 0, .. } => Ok(()),
            Response::ChecksumAck { state, .. } => Err(ClientError::Rejected {
                what: "the firmware checksum",
                state,
            }),
            got => Err(unexpected(0xF6, got)),
        }
    }
}

/// One pass of everything the app's charger screen shows.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Telemetry {
    /// Live voltages and currents.
    pub electrical: Electrical,
    /// Charger and probe temperatures.
    pub temperature: Temperature,
    /// Task state and progress.
    pub work_state: WorkState,
    /// Power and current ceilings.
    pub limits: LimitParameters,
    /// Per-cell internal resistance.
    pub resistance: InnerResistance,
}

fn unexpected(command: u8, got: Response) -> ClientError {
    ClientError::Unexpected {
        command,
        got: Box::new(got),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_poll_cycle_matches_the_apps_rotation() {
        let cycle = default_poll_cycle(0);
        let words: Vec<u8> = cycle.iter().map(Request::command_word).collect();
        assert_eq!(words, vec![0xE4, 0xE8, 0xE6, 0xE2, 0xFA, 0xFA]);
        assert_eq!(
            cycle[4],
            Request::InnerResistance { channel: 0 },
            "the app polls resistance on both channels"
        );
        assert_eq!(cycle[5], Request::InnerResistance { channel: 1 });
    }

    #[test]
    fn a_firmware_image_must_divide_into_blocks() {
        // Checked before any BLE traffic, so it is testable without a charger.
        assert!(matches!(
            std::mem::discriminant(&ClientError::BadImageLength(7)),
            d if d == std::mem::discriminant(&ClientError::BadImageLength(0))
        ));
    }
}
