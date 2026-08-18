//! Explicit version/verack handshake state machine.

use std::time::Duration;

use crate::constants::HANDSHAKE_TIMEOUT;
use crate::error::NetError;
use crate::message::{Message, MessagePayload, VersionMessage};

/// Whether this peer connection was initiated locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionDirection {
    /// The local node opened the TCP connection.
    Outbound,
    /// The remote node connected to the local listener.
    Inbound,
}

/// Configuration for handshake timing and nonce checks.
#[derive(Debug, Clone)]
pub struct HandshakeConfig {
    /// Random nonce sent in our `version` message.
    pub local_nonce: u64,
    /// Maximum time allowed to complete the handshake.
    pub timeout: Duration,
}

impl Default for HandshakeConfig {
    fn default() -> Self {
        Self {
            local_nonce: rand_nonce(),
            timeout: HANDSHAKE_TIMEOUT,
        }
    }
}

/// Handshake progress tracked per peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakePhase {
    /// Waiting for the first `version` message.
    AwaitingVersion,
    /// Our `version` was sent; waiting for peer `version` (outbound path).
    AwaitingPeerVersion,
    /// Peer `version` accepted; waiting for `verack` exchange to finish.
    AwaitingVerack,
    /// Handshake completed successfully.
    Established,
}

/// Stateful validator for Bitcoin's version/verack sequence.
#[derive(Debug, Clone)]
pub struct HandshakeState {
    direction: ConnectionDirection,
    config: HandshakeConfig,
    phase: HandshakePhase,
    peer_version: Option<VersionMessage>,
}

impl HandshakeState {
    /// Creates a new handshake tracker for `direction`.
    #[must_use]
    pub fn new(direction: ConnectionDirection, config: HandshakeConfig) -> Self {
        let phase = match direction {
            ConnectionDirection::Outbound => HandshakePhase::AwaitingPeerVersion,
            ConnectionDirection::Inbound => HandshakePhase::AwaitingVersion,
        };
        Self {
            direction,
            config,
            phase,
            peer_version: None,
        }
    }

    /// Returns the configured handshake timeout.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.config.timeout
    }

    /// Returns the current handshake phase.
    #[must_use]
    pub const fn phase(&self) -> HandshakePhase {
        self.phase
    }

    /// Returns the peer `version` message once received.
    #[must_use]
    pub fn peer_version(&self) -> Option<&VersionMessage> {
        self.peer_version.as_ref()
    }

    /// Returns messages that should be sent immediately after an outbound connect.
    #[must_use]
    pub fn initial_outbound_messages(&self, version: VersionMessage) -> Vec<Message> {
        if self.direction == ConnectionDirection::Outbound
            && self.phase == HandshakePhase::AwaitingPeerVersion
        {
            vec![Message::version(version)]
        } else {
            Vec::new()
        }
    }

    /// Processes an inbound decoded message during handshake.
    ///
    /// For inbound connections, pass `local_version` so the node can reply with
    /// its own `version` plus `verack` after accepting the peer's `version`.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::HandshakeViolation`] or [`NetError::SelfConnection`].
    pub fn on_message(
        &mut self,
        message: &Message,
        local_version: Option<VersionMessage>,
    ) -> Result<Vec<Message>, NetError> {
        match (&self.phase, &message.payload) {
            (HandshakePhase::AwaitingVersion, MessagePayload::Version(version)) => {
                self.validate_peer_version(version)?;
                self.peer_version = Some(version.clone());
                self.phase = HandshakePhase::AwaitingVerack;
                let local = local_version.ok_or(NetError::HandshakeViolation(
                    "missing local version for inbound handshake",
                ))?;
                Ok(vec![Message::version(local), Message::verack()])
            }
            (HandshakePhase::AwaitingPeerVersion, MessagePayload::Version(version)) => {
                self.validate_peer_version(version)?;
                self.peer_version = Some(version.clone());
                self.phase = HandshakePhase::AwaitingVerack;
                Ok(vec![Message::verack()])
            }
            (HandshakePhase::AwaitingVerack, MessagePayload::Verack) => {
                self.phase = HandshakePhase::Established;
                Ok(Vec::new())
            }
            (HandshakePhase::Established, _) => Ok(Vec::new()),
            _ => Err(NetError::HandshakeViolation("unexpected handshake message")),
        }
    }

    fn validate_peer_version(&self, version: &VersionMessage) -> Result<(), NetError> {
        if version.nonce == self.config.local_nonce {
            return Err(NetError::SelfConnection);
        }
        Ok(())
    }
}

fn rand_nonce() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{ConnectionDirection, HandshakeConfig, HandshakePhase, HandshakeState};
    use crate::codec::default_version_message;
    use crate::message::Message;

    fn local_version(nonce: u64) -> crate::message::VersionMessage {
        default_version_message(nonce, 1_700_000_000, 0)
    }

    #[test]
    fn outbound_handshake_requires_version_before_verack() {
        let config = HandshakeConfig {
            local_nonce: 42,
            timeout: std::time::Duration::from_secs(1),
        };
        let mut state = HandshakeState::new(ConnectionDirection::Outbound, config);
        assert_eq!(state.phase(), HandshakePhase::AwaitingPeerVersion);

        assert_eq!(
            state.on_message(&Message::verack(), None),
            Err(crate::error::NetError::HandshakeViolation(
                "unexpected handshake message"
            ))
        );

        let peer = local_version(99);
        let replies = state
            .on_message(&Message::version(peer), None)
            .expect("version");
        assert_eq!(replies, vec![Message::verack()]);
        assert_eq!(state.phase(), HandshakePhase::AwaitingVerack);

        state.on_message(&Message::verack(), None).expect("verack");
        assert_eq!(state.phase(), HandshakePhase::Established);
    }

    #[test]
    fn inbound_handshake_sends_version_and_verack() {
        let config = HandshakeConfig {
            local_nonce: 7,
            timeout: std::time::Duration::from_secs(1),
        };
        let mut state = HandshakeState::new(ConnectionDirection::Inbound, config);
        let peer = local_version(11);
        let replies = state
            .on_message(&Message::version(peer), Some(local_version(7)))
            .expect("peer version");
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0].command, "version");
        assert_eq!(replies[1].command, "verack");
    }

    #[test]
    fn matching_nonce_is_self_connection() {
        let config = HandshakeConfig {
            local_nonce: 55,
            timeout: std::time::Duration::from_secs(1),
        };
        let mut state = HandshakeState::new(ConnectionDirection::Outbound, config);
        let peer = local_version(55);
        assert_eq!(
            state.on_message(&Message::version(peer), None),
            Err(crate::error::NetError::SelfConnection)
        );
    }
}
