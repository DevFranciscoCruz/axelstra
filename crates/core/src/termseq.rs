//! Small terminal-stream helpers for the pty drain thread: knowing where a stream is "between" escape sequences, and
//! catching the startup cursor-position request even when it is split across two reads.

/// Follows the terminal escape grammar just enough to know whether the stream is at a point where it can be cut
/// and resumed (not inside CSI/OSC/DCS/... or right after a lone ESC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EscTracker {
    state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Ground,
    Esc,
    Csi,
    /// OSC: ends with BEL or ST
    Osc,
    /// DCS, SOS, PM, APC: ends with ST
    Str,
    /// ESC seen inside Osc/Str: `\` completes ST
    StrEsc,
}

const ESC: u8 = 0x1b;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const BEL: u8 = 0x07;

impl EscTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when no escape sequence is open at this point of the stream.
    pub fn is_ground(&self) -> bool {
        self.state == State::Ground
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state = self.step(b);
        }
    }

    fn step(&self, b: u8) -> State {
        use State::*;
        if b == CAN || b == SUB {
            return Ground;
        }
        match self.state {
            Ground => {
                if b == ESC {
                    Esc
                } else {
                    Ground
                }
            }
            Esc => match b {
                b'[' => Csi,
                b']' => Osc,
                b'P' | b'X' | b'^' | b'_' => Str,
                ESC => Esc,
                // intermediate bytes (charset selection `ESC ( B`, ...) keep the sequence open
                0x20..=0x2f => Esc,
                0x30..=0x7e => Ground,
                _ => Esc,
            },
            Csi => match b {
                ESC => Esc,
                0x40..=0x7e => Ground,
                _ => Csi,
            },
            Osc => match b {
                BEL => Ground,
                ESC => StrEsc,
                _ => Osc,
            },
            Str => match b {
                ESC => StrEsc,
                _ => Str,
            },
            StrEsc => match b {
                b'\\' => Ground,
                ESC => StrEsc,
                // anything else aborts the string and starts a new escape
                _ => Esc.step_from(b),
            },
        }
    }
}

impl State {
    /// The state after `b` when the previous state was `Esc`.
    fn step_from(self, b: u8) -> State {
        EscTracker { state: self }.step(b)
    }
}

/// The reply a program gets for the startup cursor-position request, and the request itself.
const DSR: &[u8] = b"\x1b[6n";

/// Finds the first cursor-position request (`ESC [ 6 n`) in a stream read in arbitrary pieces, removes it, and says
/// so. A piece that ends with the beginning of a request is held back until the next piece shows what it was.
#[derive(Debug, Default)]
pub struct DsrFilter {
    carry: Vec<u8>,
    done: bool,
}

impl DsrFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// (bytes to pass on, true when the request was found and removed in this call)
    pub fn push(&mut self, chunk: &[u8]) -> (Vec<u8>, bool) {
        if self.done {
            return (chunk.to_vec(), false);
        }
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(chunk);
        if let Some(i) = data.windows(DSR.len()).position(|w| w == DSR) {
            data.drain(i..i + DSR.len());
            self.done = true;
            return (data, true);
        }
        // hold back the longest tail that could still become the request
        let keep = (1..DSR.len()).rev().find(|&k| data.len() >= k && data[data.len() - k..] == DSR[..k]).unwrap_or(0);
        self.carry = data.split_off(data.len() - keep);
        (data, false)
    }

    /// What is still held back when the stream ends.
    pub fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.carry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground_after(s: &[u8]) -> bool {
        let mut t = EscTracker::new();
        t.feed(s);
        t.is_ground()
    }

    #[test]
    fn tracks_open_sequences() {
        assert!(ground_after(b"plain text\r\n"));
        assert!(!ground_after(b"abc\x1b"));
        assert!(!ground_after(b"abc\x1b[31"));
        assert!(ground_after(b"abc\x1b[31m"));
        assert!(!ground_after(b"\x1b]0;title"));
        assert!(ground_after(b"\x1b]0;title\x07"));
        assert!(ground_after(b"\x1b]0;title\x1b\\"));
        assert!(!ground_after(b"\x1bP1;2|data"));
        assert!(!ground_after(b"\x1b("));
        assert!(ground_after(b"\x1b(B"));
        assert!(ground_after(b"\x1b[31\x18"), "CAN aborts a sequence");
    }

    #[test]
    fn an_escape_inside_a_string_starts_a_new_sequence() {
        assert!(ground_after(b"\x1b]0;t\x1b[0m"));
        assert!(!ground_after(b"\x1b]0;t\x1b[0"));
    }

    #[test]
    fn dsr_in_one_piece() {
        let mut f = DsrFilter::new();
        assert_eq!(f.push(b"a\x1b[6nb"), (b"ab".to_vec(), true));
        assert_eq!(f.push(b"\x1b[6n"), (b"\x1b[6n".to_vec(), false), "only the first one is answered");
    }

    #[test]
    fn dsr_split_at_every_point() {
        let whole = b"hi\x1b[6nthere";
        for cut in 1..whole.len() {
            let mut f = DsrFilter::new();
            let (a, x) = f.push(&whole[..cut]);
            let (b, y) = f.push(&whole[cut..]);
            let mut out = a;
            out.extend(b);
            assert_eq!(out, b"hithere", "cut at {cut}");
            assert!(x ^ y, "answered exactly once, cut at {cut}");
        }
    }

    #[test]
    fn held_back_prefix_is_released_when_it_was_not_a_request() {
        let mut f = DsrFilter::new();
        let (a, _) = f.push(b"x\x1b[");
        assert_eq!(a, b"x");
        let (b, hit) = f.push(b"31mred");
        assert!(!hit);
        assert_eq!(b, b"\x1b[31mred");
        let mut g = DsrFilter::new();
        let (a, _) = g.push(b"x\x1b");
        assert_eq!(a, b"x");
        assert_eq!(g.finish(), b"\x1b");
    }
}
