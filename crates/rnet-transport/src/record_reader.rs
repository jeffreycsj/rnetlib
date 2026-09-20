//! Cancellation-safe record framing for established TCP sessions.

use bytes::{Buf, Bytes, BytesMut};
use rnet_core::{ErrorCode, Result, RnetError};

pub(crate) struct RecordReader {
    buffered: BytesMut,
    max_len: usize,
}

impl RecordReader {
    pub(crate) fn new(max_len: usize) -> Self {
        Self {
            buffered: BytesMut::with_capacity(8192),
            max_len,
        }
    }

    /// `read_buf` may be cancelled by `select!`; only completed reads append bytes here.
    /// Partial prefixes and bodies remain buffered across subsequent loop iterations.
    pub(crate) fn buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.buffered
    }

    pub(crate) fn take(&mut self) -> Result<Option<Bytes>> {
        if self.buffered.len() < 4 {
            return Ok(None);
        }
        let length =
            u32::from_be_bytes(self.buffered[..4].try_into().expect("prefix present")) as usize;
        if length == 0 {
            return Err(RnetError::new(ErrorCode::ProtocolError, "empty TCP record"));
        }
        if length > self.max_len {
            return Err(RnetError::new(
                ErrorCode::MessageTooLarge,
                "TCP record exceeds the configured limit",
            ));
        }
        let framed_len = length.checked_add(4).ok_or_else(|| {
            RnetError::new(ErrorCode::MessageTooLarge, "TCP record length overflows")
        })?;
        if self.buffered.len() < framed_len {
            return Ok(None);
        }
        self.buffered.advance(4);
        Ok(Some(self.buffered.split_to(length).freeze()))
    }
}

#[cfg(test)]
mod tests {
    use super::RecordReader;
    use bytes::Bytes;
    use rnet_core::ErrorCode;

    #[test]
    fn retains_split_header_and_body_across_reads() {
        let mut reader = RecordReader::new(16);
        reader.buffer_mut().extend_from_slice(&[0, 0]);
        assert_eq!(reader.take().unwrap(), None);
        reader.buffer_mut().extend_from_slice(&[0, 3, 10]);
        assert_eq!(reader.take().unwrap(), None);
        reader.buffer_mut().extend_from_slice(&[11, 12]);
        assert_eq!(
            reader.take().unwrap(),
            Some(Bytes::from_static(&[10, 11, 12]))
        );
        assert_eq!(reader.take().unwrap(), None);
    }

    #[test]
    fn returns_coalesced_records_in_order() {
        let mut reader = RecordReader::new(16);
        reader
            .buffer_mut()
            .extend_from_slice(&[0, 0, 0, 1, 10, 0, 0, 0, 1, 11]);
        assert_eq!(reader.take().unwrap(), Some(Bytes::from_static(&[10])));
        assert_eq!(reader.take().unwrap(), Some(Bytes::from_static(&[11])));
    }

    #[test]
    fn rejects_oversized_prefix_without_waiting_for_body() {
        let mut reader = RecordReader::new(16);
        reader.buffer_mut().extend_from_slice(&[0, 0, 0, 17]);
        assert_eq!(
            reader.take().unwrap_err().code(),
            ErrorCode::MessageTooLarge
        );
    }

    #[test]
    fn rejects_empty_record_before_processing_a_following_record() {
        let mut reader = RecordReader::new(16);
        reader
            .buffer_mut()
            .extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1, 42]);
        assert_eq!(reader.take().unwrap_err().code(), ErrorCode::ProtocolError);
    }
}
