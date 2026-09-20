//! Deterministic server-led security transition barriers.

use rnet_core::{ErrorCode, Result, RnetError};
use rnet_protocol::control::{Control, ControlKind, SecurityMode};

/// Identifies which side owns security-policy changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// May initiate mode switches and rekeys.
    Server,
    /// Automatically follows authenticated server proposals.
    Client,
}

/// Describes a cryptographic or mode change the I/O driver must apply at a barrier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    /// No local channel change is required.
    None,
    /// Start encoding and accepting business data with the selected mode.
    SetMode(SecurityMode),
    /// Rekey both Noise directions before processing the next protected record.
    Rekey,
    /// Commit the already-applied rekey epoch without changing keys again.
    AdvanceEpoch,
}

/// Result of consuming one authenticated control message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transition {
    /// Apply this effect after receiving the input and before encoding `response`.
    pub before_response: Effect,
    /// Optional barrier to send to the peer.
    pub response: Option<Control>,
    /// Apply this effect only after the response has been successfully written.
    pub after_response: Effect,
}

impl Transition {
    fn response(control: Control) -> Self {
        Self {
            before_response: Effect::None,
            response: Some(control),
            after_response: Effect::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pending {
    ServerSwitchReady { epoch: u64, mode: SecurityMode },
    ServerSwitchAck { epoch: u64, mode: SecurityMode },
    ClientSwitchCommit { epoch: u64, mode: SecurityMode },
    ServerRekeyReady { epoch: u64 },
    ServerRekeyAck { epoch: u64 },
    ClientRekeyCommit { epoch: u64 },
}

/// Tracks one session's mode, epoch and at most one in-flight server-led transition.
///
/// This type does not perform I/O or encryption. Drivers must honor the returned effect ordering;
/// in particular, a rekey commit is encrypted with the old key and its acknowledgement with the
/// new key.
#[derive(Debug)]
pub struct SecurityController {
    role: Role,
    mode: SecurityMode,
    epoch: u64,
    pending: Option<Pending>,
}

impl SecurityController {
    /// Creates an established controller. `epoch` must be nonzero.
    pub fn new(role: Role, mode: SecurityMode, epoch: u64) -> Self {
        Self {
            role,
            mode,
            epoch,
            pending: None,
        }
    }

    /// Returns the active application-data mode.
    pub fn mode(&self) -> SecurityMode {
        self.mode
    }

    /// Returns the active security generation.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Returns whether business sends must wait for an in-flight transition barrier.
    pub fn is_transitioning(&self) -> bool {
        self.pending.is_some()
    }

    /// Starts a server-led application-data mode switch.
    pub fn begin_switch(&mut self, mode: SecurityMode) -> Result<Control> {
        self.ensure_server_idle()?;
        if mode == self.mode {
            return invalid_state("requested security mode is already active");
        }
        let epoch = self.next_epoch()?;
        self.pending = Some(Pending::ServerSwitchReady { epoch, mode });
        Ok(Control::switch(ControlKind::SwitchPropose, epoch, mode))
    }

    /// Starts a server-led Noise rekey without changing application-data mode.
    pub fn begin_rekey(&mut self) -> Result<Control> {
        self.ensure_server_idle()?;
        let epoch = self.next_epoch()?;
        self.pending = Some(Pending::ServerRekeyReady { epoch });
        Ok(Control::barrier(ControlKind::RekeyPropose, epoch))
    }

    /// Consumes one authenticated peer control and advances the local barrier state.
    pub fn handle(&mut self, control: Control) -> Result<Transition> {
        match (self.role, self.pending, control.kind) {
            (Role::Client, None, ControlKind::SwitchPropose) => {
                let mode = control
                    .mode
                    .ok_or_else(|| state_error("switch proposal has no target mode"))?;
                self.validate_next_epoch(control.epoch)?;
                self.pending = Some(Pending::ClientSwitchCommit {
                    epoch: control.epoch,
                    mode,
                });
                Ok(Transition::response(Control::barrier(
                    ControlKind::SwitchReady,
                    control.epoch,
                )))
            }
            (
                Role::Server,
                Some(Pending::ServerSwitchReady { epoch, mode }),
                ControlKind::SwitchReady,
            ) if control.epoch == epoch => {
                self.pending = Some(Pending::ServerSwitchAck { epoch, mode });
                Ok(Transition::response(Control::barrier(
                    ControlKind::SwitchCommit,
                    epoch,
                )))
            }
            (
                Role::Client,
                Some(Pending::ClientSwitchCommit { epoch, mode }),
                ControlKind::SwitchCommit,
            ) if control.epoch == epoch => {
                self.mode = mode;
                self.epoch = epoch;
                self.pending = None;
                Ok(Transition {
                    before_response: Effect::SetMode(mode),
                    response: Some(Control::barrier(ControlKind::SwitchAck, epoch)),
                    after_response: Effect::None,
                })
            }
            (
                Role::Server,
                Some(Pending::ServerSwitchAck { epoch, mode }),
                ControlKind::SwitchAck,
            ) if control.epoch == epoch => {
                self.mode = mode;
                self.epoch = epoch;
                self.pending = None;
                Ok(Transition {
                    before_response: Effect::SetMode(mode),
                    response: None,
                    after_response: Effect::None,
                })
            }
            (Role::Client, None, ControlKind::RekeyPropose) => {
                self.validate_next_epoch(control.epoch)?;
                self.pending = Some(Pending::ClientRekeyCommit {
                    epoch: control.epoch,
                });
                Ok(Transition::response(Control::barrier(
                    ControlKind::RekeyReady,
                    control.epoch,
                )))
            }
            (Role::Server, Some(Pending::ServerRekeyReady { epoch }), ControlKind::RekeyReady)
                if control.epoch == epoch =>
            {
                self.pending = Some(Pending::ServerRekeyAck { epoch });
                Ok(Transition {
                    before_response: Effect::None,
                    response: Some(Control::barrier(ControlKind::RekeyCommit, epoch)),
                    after_response: Effect::Rekey,
                })
            }
            (
                Role::Client,
                Some(Pending::ClientRekeyCommit { epoch }),
                ControlKind::RekeyCommit,
            ) if control.epoch == epoch => {
                self.epoch = epoch;
                self.pending = None;
                Ok(Transition {
                    before_response: Effect::Rekey,
                    response: Some(Control::barrier(ControlKind::RekeyAck, epoch)),
                    after_response: Effect::None,
                })
            }
            (Role::Server, Some(Pending::ServerRekeyAck { epoch }), ControlKind::RekeyAck)
                if control.epoch == epoch =>
            {
                self.epoch = epoch;
                self.pending = None;
                Ok(Transition {
                    before_response: Effect::AdvanceEpoch,
                    response: None,
                    after_response: Effect::None,
                })
            }
            _ => invalid_state("unexpected or stale security transition barrier"),
        }
    }

    fn ensure_server_idle(&self) -> Result<()> {
        if self.role != Role::Server {
            return invalid_state("only the server may initiate security transitions");
        }
        if self.pending.is_some() {
            return invalid_state("a security transition is already in progress");
        }
        if self.epoch == 0 {
            return invalid_state("established security epoch must be nonzero");
        }
        Ok(())
    }

    fn next_epoch(&self) -> Result<u64> {
        self.epoch
            .checked_add(1)
            .ok_or_else(|| state_error("security epoch exhausted"))
    }

    fn validate_next_epoch(&self, epoch: u64) -> Result<()> {
        if epoch != self.next_epoch()? {
            return invalid_state("security transition skipped or reused an epoch");
        }
        Ok(())
    }
}

fn invalid_state<T>(message: &str) -> Result<T> {
    Err(state_error(message))
}

fn state_error(message: &str) -> RnetError {
    RnetError::new(ErrorCode::InvalidState, message)
}
