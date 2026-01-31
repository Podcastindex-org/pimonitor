//! Integration tests to cover the Rodio/Symphonia initialization panic scenario.
//!
//! Context:
//! The upstream `rodio::decoder::symphonia` path contains an `unreachable!()`
//! assertion that assumes seeks during decoder initialization cannot fail.
//! On some systems (notably Windows with certain FS/share modes), a seek can
//! error and this triggers a panic like:
//!
//!   "entered unreachable code: Seek errors should not occur during initialization"
//!
//! These tests ensure we have:
//! - A deterministic way to reproduce the panic by providing a reader whose
//!   `Seek` always errors during initialization.
//! - A sanity check that using an in-memory `Cursor<Vec<u8>>` (our workaround)
//!   does not panic (it may return a decode error for invalid data, but must not panic).

use std::io::{Error as IoError, ErrorKind, Read, Seek, SeekFrom};
use std::panic;

/// A reader that can provide bytes but always fails any seek operation.
/// This simulates environments where `seek` fails during decoder init.
struct PoisonSeek {
    buf: Vec<u8>,
    pos: usize,
}

impl PoisonSeek {
    fn new(size: usize) -> Self {
        Self {
            buf: vec![0u8; size],
            pos: 0,
        }
    }
}

impl Read for PoisonSeek {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.buf.len().saturating_sub(self.pos);
        let n = remaining.min(out.len());
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Seek for PoisonSeek {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(IoError::new(ErrorKind::Other, "Poisoned seek"))
    }
}

#[test]
fn rodio_decoder_panics_when_seek_errors_during_init() {
    // This verifies the panic path still exists upstream so we can catch it if it regresses.
    // We expect a panic inside `rodio::Decoder::new` when the underlying reader returns
    // an error on seek during initialization.
    let result = panic::catch_unwind(|| {
        let reader = PoisonSeek::new(4096);
        let _decoder = rodio::Decoder::new(reader).expect("Decoder unexpectedly did not panic");
    });

    assert!(
        result.is_err(),
        "Expected a panic when seek fails during decoder init"
    );
}

#[test]
fn rodio_decoder_with_cursor_does_not_panic() {
    // Our workaround in the app uses a `Cursor<Vec<u8>>`. Creating a decoder from a Cursor
    // should not panic. It may return `Err` for invalid audio data, which is fine for this test.
    use std::io::Cursor;

    let result = panic::catch_unwind(|| {
        let cursor = Cursor::new(vec![0u8; 128]); // not valid audio, but fully seekable
        let _ = rodio::Decoder::new(cursor); // Should not panic; may return Err
    });

    assert!(
        result.is_ok(),
        "Decoder creation with Cursor should not panic"
    );
}
