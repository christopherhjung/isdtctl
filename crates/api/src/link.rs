//! The seam between the protocol and whatever carries it.
//!
//! [`Client`](crate::Client) speaks in whole frames and knows nothing about
//! Bluetooth. Implement [`Link`] and every command, query and helper in this
//! crate works over your transport: a serial port, a TCP bridge, a test double
//! that replays a capture.
//!
//! A [`Link`] deals in the frame `DATA` field, meaning the command word plus
//! its arguments. Framing, byte stuffing, checksums and packet chunking belong
//! to the implementation, and [`crate::frame`] provides all of it.
//!
//! # Implementing one
//!
//! ```
//! use std::time::Duration;
//! use std::collections::VecDeque;
//!
//! use api::link::{Link, LinkError};
//!
//! /// A link that replays canned replies, for tests.
//! struct Replay {
//!     replies: VecDeque<Vec<u8>>,
//! }
//!
//! #[async_trait::async_trait]
//! impl Link for Replay {
//!     async fn send(&self, _data: &[u8]) -> Result<(), LinkError> {
//!         Ok(())
//!     }
//!
//!     async fn recv(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>, LinkError> {
//!         Ok(self.replies.pop_front())
//!     }
//! }
//! ```

use std::time::Duration;

use crate::FrameError;

/// Something that went wrong carrying a frame.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    /// The link is gone and will not carry anything else.
    #[error("the link to the charger is closed")]
    Closed,

    /// The frame could not be built.
    #[error(transparent)]
    Frame(#[from] FrameError),

    /// The transport failed for a reason of its own.
    #[error(transparent)]
    Transport(Box<dyn std::error::Error + Send + Sync>),
}

impl LinkError {
    /// Wraps a transport's own error.
    pub fn transport<E>(error: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        LinkError::Transport(Box::new(error))
    }
}

/// A bidirectional carrier for protocol frames.
///
/// Implementations must be cancel-safe at `await` points: [`Client`] races
/// [`Link::recv`] against deadlines and will drop the future.
///
/// [`Client`]: crate::Client
#[async_trait::async_trait]
pub trait Link: Send {
    /// Sends one frame's `DATA` field, which starts with the command word.
    ///
    /// Implementations frame, stuff and chunk as their transport requires.
    async fn send(&self, data: &[u8]) -> Result<(), LinkError>;

    /// Waits up to `timeout` for the next frame, returning its `DATA` field.
    ///
    /// `Ok(None)` means the deadline passed with nothing to report, which is
    /// ordinary. Return [`LinkError::Closed`] once the link is gone for good,
    /// so callers stop rather than retrying into a void.
    async fn recv(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, LinkError>;

    /// Discards frames that have already arrived.
    ///
    /// Called before a request so a stale reply cannot be mistaken for a fresh
    /// one. The default does nothing, which is correct for a link that does no
    /// buffering of its own.
    fn drain(&mut self) {}

    /// The largest `DATA` field this link will carry, if it is limited.
    ///
    /// Only the firmware write commands come close, and the protocol's own
    /// ceiling is 255 bytes either way.
    fn max_data_len(&self) -> usize {
        crate::frame::MAX_DATA_LEN
    }
}
