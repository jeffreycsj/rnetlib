//! Record encryption helpers shared by adaptive datagram drivers.

use crate::state::map_security_error;
use rnet_core::{ErrorCode, Result, RnetError};
use rnet_protocol::control::{
    decode_protected, encode_protected, ProtectedKind, ProtectedMessage, Record, RecordKind,
    SecurityMode,
};
use rnet_security::transition::{Effect, SecurityController};
use rnet_security::DatagramTransport;

/// Maximum bytes added around a logical frame by the adaptive encrypted UDP wire format.
pub(crate) const ENCRYPTED_UDP_WIRE_OVERHEAD: usize = 5 + 8 + 16 + 20;

pub(crate) fn data_record(
    transport: &mut DatagramTransport,
    controller: &SecurityController,
    data: &[u8],
    max: usize,
) -> Result<Record> {
    match controller.mode() {
        SecurityMode::Plaintext => Ok(Record::new(RecordKind::PlainData, controller.epoch(), data)),
        SecurityMode::Encrypted => protected_record(
            transport,
            controller.epoch(),
            ProtectedKind::Data,
            data,
            max,
        ),
    }
}

pub(crate) fn protected_record(
    transport: &mut DatagramTransport,
    epoch: u64,
    kind: ProtectedKind,
    data: &[u8],
    max: usize,
) -> Result<Record> {
    let inner = encode_protected(&ProtectedMessage::new(kind, data), max)?;
    let cipher = transport.encrypt(&inner).map_err(map_security_error)?;
    Ok(Record::new(RecordKind::Protected, epoch, &cipher))
}

pub(crate) fn decrypt(
    transport: &mut DatagramTransport,
    record: &Record,
    max: usize,
) -> Result<ProtectedMessage> {
    let plain = transport
        .decrypt(&record.payload)
        .map_err(map_security_error)?;
    decode_protected(&plain, max)
}

pub(crate) fn apply(transport: &mut DatagramTransport, effect: Effect) {
    if effect == Effect::Rekey {
        transport.rekey_incoming();
        transport.rekey_outgoing();
    }
}

pub(crate) fn protocol<T>(message: &str) -> Result<T> {
    Err(RnetError::new(ErrorCode::ProtocolError, message))
}
