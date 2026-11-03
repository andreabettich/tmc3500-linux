//! Low-level command writer.
use crate::decode::ESC;

#[derive(Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(70_000),
        }
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    pub fn nuls3(&mut self) {
        self.bytes(&[0, 0, 0]);
    }
    /// ESC 01 "@EJL 1284.4\n@EJL     \n" (enter IEEE 1284.4 / packet mode).
    pub fn ejl_1284(&mut self) {
        self.bytes(b"\x1b\x01@EJL 1284.4\n@EJL     \n");
    }
    pub fn init(&mut self) {
        self.bytes(&[ESC, b'@']);
    }
    /// ESC ( cmd <len u16le> payload. Payloads are produced internally and are < 64 KiB.
    pub fn paren(&mut self, cmd: u8, payload: &[u8]) {
        let n = u16::try_from(payload.len()).expect("ESC( payload < 64 KiB (internal invariant)");
        self.bytes(&[ESC, b'(', cmd]);
        self.bytes(&n.to_le_bytes());
        self.bytes(payload);
    }
    pub fn remote_enter(&mut self) {
        self.paren(b'R', b"\0REMOTE1");
    }
    pub fn remote(&mut self, cmd: &[u8; 2], payload: &[u8]) {
        let n =
            u16::try_from(payload.len()).expect("REMOTE1 payload < 64 KiB (internal invariant)");
        self.bytes(cmd);
        self.bytes(&n.to_le_bytes());
        self.bytes(payload);
    }
    pub fn remote_exit(&mut self) {
        self.bytes(&[ESC, 0, 0, 0]);
    }
    pub fn short(&mut self, cmd: u8, arg: u8) {
        self.bytes(&[ESC, cmd, arg]);
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}
