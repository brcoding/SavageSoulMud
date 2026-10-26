//! Bounded streaming Telnet decoder. Reads have no relation to line endings.
pub(crate) const IAC: u8 = 255;
pub(crate) const NO_OP: [u8; 2] = [IAC, 241];
pub(crate) const WILL: u8 = 251;
pub(crate) const WONT: u8 = 252;
pub(crate) const DO: u8 = 253;
pub(crate) const DONT: u8 = 254;
pub(crate) const SB: u8 = 250;
pub(crate) const SE: u8 = 240;
pub(crate) const WILL_ECHO: [u8; 3] = [IAC, WILL, 1];
pub(crate) const WONT_ECHO: [u8; 3] = [IAC, WONT, 1];
pub(crate) const MAX_SUBNEGOTIATION: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Frame {
    Data(u8),
    Negotiate(u8, u8),
    Sub(u8, Vec<u8>),
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Decoder {
    state: u8,
    command: u8,
    option: u8,
    sub: Vec<u8>,
}
impl Decoder {
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, ()> {
        let mut frames = Vec::new();
        for &byte in bytes {
            match self.state {
                0 if byte == IAC => self.state = 1,
                0 => frames.push(Frame::Data(byte)),
                1 => match byte {
                    IAC => {
                        frames.push(Frame::Data(IAC));
                        self.state = 0;
                    }
                    WILL | WONT | DO | DONT => {
                        self.command = byte;
                        self.state = 2;
                    }
                    SB => self.state = 3,
                    _ => self.state = 0,
                },
                2 => {
                    frames.push(Frame::Negotiate(self.command, byte));
                    self.state = 0;
                }
                3 => {
                    self.option = byte;
                    self.sub.clear();
                    self.state = 4;
                }
                4 if byte == IAC => self.state = 5,
                4 => self.sub.push(byte),
                5 => match byte {
                    IAC => {
                        self.sub.push(IAC);
                        self.state = 4;
                    }
                    SE => {
                        frames.push(Frame::Sub(self.option, std::mem::take(&mut self.sub)));
                        self.state = 0;
                    }
                    _ => return Err(()),
                },
                _ => return Err(()),
            }
            if self.sub.len() > MAX_SUBNEGOTIATION {
                return Err(());
            }
        }
        Ok(frames)
    }
}
pub(crate) fn subnegotiation(option: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![IAC, SB, option];
    for &byte in payload {
        out.push(byte);
        if byte == IAC {
            out.push(IAC);
        }
    }
    out.extend([IAC, SE]);
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragments_subnegotiation_without_leaking_newlines() {
        let mut d = Decoder::default();
        assert_eq!(
            d.feed(b"lo\xff").unwrap(),
            vec![Frame::Data(b'l'), Frame::Data(b'o')]
        );
        assert!(d.feed(&[SB, 201, b'x', b'\n', IAC]).unwrap().is_empty());
        assert_eq!(
            d.feed(&[IAC, IAC, SE, b'o', b'k', b'\n']).unwrap(),
            vec![
                Frame::Sub(201, vec![b'x', b'\n', IAC]),
                Frame::Data(b'o'),
                Frame::Data(b'k'),
                Frame::Data(b'\n')
            ]
        );
    }
    #[test]
    fn fragmented_negotiation_and_single_byte_commands() {
        let mut d = Decoder::default();
        assert!(d.feed(&[IAC, WILL]).unwrap().is_empty());
        assert_eq!(
            d.feed(&[31, IAC, 249, b'a']).unwrap(),
            vec![Frame::Negotiate(WILL, 31), Frame::Data(b'a')]
        );
    }
    #[test]
    fn rejects_unbounded_payload() {
        let mut d = Decoder::default();
        d.feed(&[IAC, SB, 201]).unwrap();
        assert!(d.feed(&vec![b'a'; MAX_SUBNEGOTIATION + 1]).is_err());
    }
    #[test]
    fn escapes_iac() {
        assert_eq!(
            subnegotiation(31, &[255, 1]),
            vec![255, 250, 31, 255, 255, 1, 255, 240]
        );
    }
}
