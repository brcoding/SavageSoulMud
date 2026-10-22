//! Negotiated client metadata and the game-independent out-of-band seam.
use crate::iac::{self, Frame, DO, DONT, IAC, WILL, WONT};
use bevy::prelude::*;
use std::collections::HashMap;
pub const TTYPE: u8 = 24;
pub const EOR: u8 = 25;
pub const NAWS: u8 = 31;
pub const NEW_ENVIRON: u8 = 39;
pub const CHARSET: u8 = 42;
pub const MSDP: u8 = 69;
pub const MSSP: u8 = 70;
pub const MSP: u8 = 90;
pub const MXP: u8 = 91;
pub const ATCP: u8 = 200;
pub const GMCP: u8 = 201;
/// Trusted server-authored markup, not a Telnet option. Never construct this
/// from unescaped remote/player text. IAC bytes are rejected by the transport.
pub const RAW_MARKUP: u8 = 0;

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TelnetCapabilities {
    pub gmcp: bool,
    pub msdp: bool,
    pub mssp: bool,
    pub mxp: bool,
    pub msp: bool,
    pub atcp: bool,
    pub eor: bool,
    pub utf8: bool,
    pub width: u16,
    pub height: u16,
    pub terminal_types: Vec<String>,
    /// Whitelisted terminal properties only; never USER, PASSWORD or IPADDRESS.
    pub environment: HashMap<String, String>,
    pub gmcp_supports: Vec<String>,
}
#[derive(Message, Clone, Debug)]
pub struct TelnetInput {
    pub connection: Entity,
    pub option: u8,
    pub payload: Vec<u8>,
}
#[derive(Message, Clone, Debug)]
pub struct TelnetOutput {
    pub connection: Entity,
    pub option: u8,
    pub payload: Vec<u8>,
}
impl TelnetCapabilities {
    pub fn enabled(&self, option: u8) -> bool {
        match option {
            GMCP => self.gmcp,
            MSDP => self.msdp,
            MSSP => self.mssp,
            MSP => self.msp,
            MXP => self.mxp,
            ATCP => self.atcp,
            EOR => self.eor,
            _ => false,
        }
    }
}
// Grim consumes complete command lines. SGA together with password ECHO makes
// Mudlet identify a character-at-a-time server, so do not negotiate SGA.
const LOCAL: &[u8] = &[EOR, CHARSET, GMCP, MSDP, MSSP, MSP, MXP, ATCP];
const REMOTE: &[u8] = &[TTYPE, NAWS, NEW_ENVIRON, CHARSET];
const ENV_KEYS: &[&str] = &[
    "CHARSET",
    "CLIENT_NAME",
    "CLIENT_VERSION",
    "MTTS",
    "TERMINAL_TYPE",
    "TERM",
    "COLORTERM",
    "LANG",
];

pub(crate) struct ProtocolState {
    // Disabled/requested/enabled/refused. Duplicate ACKs and rejected options
    // never cause a negotiation loop. ECHO remains controlled by login output.
    local: [u8; 256],
    remote: [u8; 256],
    pub(crate) capabilities: TelnetCapabilities,
    terminal_requests: usize,
}
impl Default for ProtocolState {
    fn default() -> Self {
        Self {
            local: [0; 256],
            remote: [0; 256],
            capabilities: TelnetCapabilities::default(),
            terminal_requests: 0,
        }
    }
}
impl ProtocolState {
    pub(crate) fn handshake(&mut self, resumed: bool) -> Vec<u8> {
        let mut out = Vec::new();
        // Reset previously negotiated optional protocols on hot restart. The
        // client's profile stays connected and sends fresh capability data.
        for &option in LOCAL {
            if resumed {
                out.extend([IAC, WONT, option]);
            }
            self.local[option as usize] = 1;
            out.extend([IAC, WILL, option]);
        }
        for &option in REMOTE {
            if resumed {
                out.extend([IAC, DONT, option]);
            }
            self.remote[option as usize] = 1;
            out.extend([IAC, DO, option]);
        }
        out
    }
    pub(crate) fn accept(&mut self, frame: Frame) -> (Vec<u8>, Option<(u8, Vec<u8>)>) {
        let mut reply = Vec::new();
        let mut input = None;
        match frame {
            Frame::Negotiate(verb, option) => {
                // Password masking is authoritative on the server. Never
                // countermand it in response to an unsolicited ECHO request.
                if option == 1 {
                    return (reply, input);
                }
                let local = matches!(verb, DO | DONT);
                let supported = if local {
                    LOCAL.contains(&option)
                } else {
                    REMOTE.contains(&option)
                };
                let states = if local {
                    &mut self.local
                } else {
                    &mut self.remote
                };
                let old = states[option as usize];
                let positive = matches!(verb, DO | WILL);
                if positive && supported {
                    states[option as usize] = 2;
                    if old == 0 || old == 3 {
                        reply.extend([IAC, if local { WILL } else { DO }, option]);
                    }
                    if old != 2 {
                        self.update_capability(option, true);
                        match option {
                            TTYPE => {
                                self.terminal_requests = 1;
                                reply.extend(iac::subnegotiation(TTYPE, &[1]));
                            }
                            NEW_ENVIRON => {
                                let mut data = vec![1];
                                for key in ENV_KEYS {
                                    data.push(0);
                                    data.extend(key.as_bytes());
                                }
                                reply.extend(iac::subnegotiation(NEW_ENVIRON, &data));
                            }
                            CHARSET => reply.extend(iac::subnegotiation(CHARSET, b"\x01;UTF-8")),
                            GMCP | MSDP | MSSP | MXP | MSP | ATCP => {
                                input = Some((option, Vec::new()))
                            }
                            _ => {}
                        }
                    }
                } else {
                    states[option as usize] = if positive { 3 } else { 0 };
                    if (positive && old != 3) || (!positive && old == 2) {
                        reply.extend([IAC, if local { WONT } else { DONT }, option]);
                    }
                    self.update_capability(option, false);
                }
            }
            Frame::Sub(option, payload) => {
                if self.local[option as usize] != 2 && self.remote[option as usize] != 2 {
                    return (reply, input);
                }
                match option {
                    NAWS if payload.len() == 4 => {
                        self.capabilities.width = u16::from_be_bytes([payload[0], payload[1]]);
                        self.capabilities.height = u16::from_be_bytes([payload[2], payload[3]]);
                    }
                    TTYPE if payload.first() == Some(&0) => {
                        if let Some(name) = safe_text(&payload[1..], 128) {
                            let repeated = self.capabilities.terminal_types.contains(&name);
                            if let Some(bits) = name
                                .strip_prefix("MTTS ")
                                .and_then(|v| v.parse::<u32>().ok())
                            {
                                self.capabilities.utf8 |= bits & 4 != 0;
                            }
                            if !repeated && self.capabilities.terminal_types.len() < 3 {
                                self.capabilities.terminal_types.push(name);
                            }
                            if !repeated && self.terminal_requests < 3 {
                                self.terminal_requests += 1;
                                reply.extend(iac::subnegotiation(TTYPE, &[1]));
                            }
                        }
                    }
                    CHARSET => match payload.first() {
                        Some(2) => {
                            self.capabilities.utf8 = payload
                                .get(1..)
                                .is_some_and(|s| s.eq_ignore_ascii_case(b"UTF-8"))
                        }
                        Some(1) if payload.len() > 2 => {
                            let separator = payload[1];
                            let utf8 = payload[2..]
                                .split(|b| *b == separator)
                                .any(|v| v.eq_ignore_ascii_case(b"UTF-8"));
                            self.capabilities.utf8 = utf8;
                            reply.extend(iac::subnegotiation(
                                CHARSET,
                                if utf8 { b"\x02UTF-8" } else { b"\x03" },
                            ));
                        }
                        _ => {}
                    },
                    NEW_ENVIRON => self.environment(&payload),
                    GMCP => {
                        if let Some(text) = safe_text(&payload, 8192) {
                            self.gmcp_core(&text);
                            input = Some((option, payload));
                        }
                    }
                    MSDP | ATCP | MXP | MSP => input = Some((option, payload)),
                    _ => {}
                }
            }
            _ => {}
        }
        (reply, input)
    }
    fn update_capability(&mut self, option: u8, enabled: bool) {
        match option {
            GMCP => {
                self.capabilities.gmcp = enabled;
                if !enabled {
                    self.capabilities.gmcp_supports.clear();
                }
            }
            MSDP => self.capabilities.msdp = enabled,
            MSSP => self.capabilities.mssp = enabled,
            MXP => self.capabilities.mxp = enabled,
            MSP => self.capabilities.msp = enabled,
            ATCP => self.capabilities.atcp = enabled,
            EOR => self.capabilities.eor = enabled,
            _ => {}
        }
    }
    fn gmcp_core(&mut self, text: &str) {
        let Some((package, json)) = text.split_once(' ') else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            return;
        };
        if package == "Core.Hello" {
            for (field, key) in [("client", "CLIENT_NAME"), ("version", "CLIENT_VERSION")] {
                if let Some(value) = value
                    .get(field)
                    .and_then(|v| v.as_str())
                    .filter(|v| v.len() <= 128 && !v.chars().any(char::is_control))
                {
                    self.capabilities
                        .environment
                        .insert(key.into(), value.into());
                }
            }
        } else if matches!(
            package,
            "Core.Supports.Set" | "Core.Supports.Add" | "Core.Supports.Remove"
        ) {
            if let Some(values) = value.as_array() {
                if package.ends_with("Set") {
                    self.capabilities.gmcp_supports.clear();
                }
                for entry in values
                    .iter()
                    .take(128)
                    .filter_map(|v| v.as_str())
                    .filter(|v| v.len() <= 128 && !v.chars().any(char::is_control))
                {
                    if package.ends_with("Remove") {
                        self.capabilities.gmcp_supports.retain(|v| v != entry);
                    } else if self.capabilities.gmcp_supports.len() < 128
                        && !self.capabilities.gmcp_supports.iter().any(|v| v == entry)
                    {
                        self.capabilities.gmcp_supports.push(entry.into());
                    }
                }
            }
        }
    }
    fn environment(&mut self, payload: &[u8]) {
        if !matches!(payload.first(), Some(0 | 2)) {
            return;
        }
        let mut key = Vec::new();
        let mut value = Vec::new();
        let mut in_value = false;
        let mut escaped = false;
        let save = |key: &[u8], value: &[u8], caps: &mut TelnetCapabilities| {
            if let (Some(key), Some(value)) = (safe_text(key, 64), safe_text(value, 256)) {
                if ENV_KEYS.contains(&key.as_str()) {
                    if key == "CHARSET" {
                        caps.utf8 = value.eq_ignore_ascii_case("UTF-8");
                    }
                    if key == "MTTS" {
                        if let Ok(bits) = value.parse::<u32>() {
                            caps.utf8 |= bits & 4 != 0;
                        }
                    }
                    caps.environment.insert(key, value);
                }
            }
        };
        for &byte in &payload[1..] {
            if escaped {
                if in_value {
                    value.push(byte)
                } else {
                    key.push(byte)
                };
                escaped = false;
                continue;
            }
            match byte {
                2 => escaped = true,
                0 | 3 => {
                    save(&key, &value, &mut self.capabilities);
                    key.clear();
                    value.clear();
                    in_value = false;
                }
                1 => in_value = true,
                _ => {
                    if in_value {
                        value.push(byte)
                    } else {
                        key.push(byte)
                    }
                }
            }
        }
        save(&key, &value, &mut self.capabilities);
    }
}
fn safe_text(bytes: &[u8], limit: usize) -> Option<String> {
    if bytes.len() > limit {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    if text.chars().any(char::is_control) {
        None
    } else {
        Some(text.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_ack_and_refusal_do_not_loop() {
        let mut state = ProtocolState::default();
        state.handshake(false);
        let (reply, event) = state.accept(Frame::Negotiate(DO, GMCP));
        assert!(reply.is_empty());
        assert!(event.is_some());
        assert!(state.capabilities.gmcp);
        assert_eq!(state.accept(Frame::Negotiate(DO, GMCP)), (vec![], None));
        assert_eq!(
            state.accept(Frame::Negotiate(WILL, 99)).0,
            vec![IAC, DONT, 99]
        );
        assert!(state.accept(Frame::Negotiate(WILL, 99)).0.is_empty());
    }
    #[test]
    fn line_input_never_advertises_character_mode_sga() {
        for resumed in [false, true] {
            let mut state = ProtocolState::default();
            let handshake = state.handshake(resumed);
            assert!(!handshake.chunks_exact(3).any(|frame| frame[2] == 3));
            assert_eq!(state.accept(Frame::Negotiate(DO, 3)).0, vec![IAC, WONT, 3]);
            assert_eq!(state.accept(Frame::Negotiate(WILL, 3)).0, vec![IAC, DONT, 3]);
        }
    }
    #[test]
    fn real_size_and_bounded_terminal_rounds() {
        let mut state = ProtocolState::default();
        state.handshake(false);
        state.accept(Frame::Negotiate(WILL, NAWS));
        state.accept(Frame::Sub(NAWS, vec![0, 120, 0, 40]));
        assert_eq!(
            (state.capabilities.width, state.capabilities.height),
            (120, 40)
        );
        state.accept(Frame::Negotiate(WILL, TTYPE));
        state.accept(Frame::Sub(TTYPE, b"\0Mudlet 4".to_vec()));
        state.accept(Frame::Sub(TTYPE, b"\0XTERM".to_vec()));
        let (reply, _) = state.accept(Frame::Sub(TTYPE, b"\0MTTS 269".to_vec()));
        assert!(reply.is_empty());
        assert!(state.capabilities.utf8);
        assert_eq!(state.capabilities.terminal_types.len(), 3);
    }
    #[test]
    fn core_supports_and_environment_privacy() {
        let mut state = ProtocolState::default();
        state.handshake(false);
        state.accept(Frame::Negotiate(DO, GMCP));
        state.accept(Frame::Sub(
            GMCP,
            br#"Core.Supports.Set ["Room 1","Char 1"]"#.to_vec(),
        ));
        assert_eq!(state.capabilities.gmcp_supports, vec!["Room 1", "Char 1"]);
        state.accept(Frame::Negotiate(WILL, NEW_ENVIRON));
        state.accept(Frame::Sub(
            NEW_ENVIRON,
            b"\0\0USER\x01secret\0CLIENT_NAME\x01Mudlet".to_vec(),
        ));
        assert!(!state.capabilities.environment.contains_key("USER"));
        assert_eq!(state.capabilities.environment["CLIENT_NAME"], "Mudlet");
    }
    #[test]
    fn charset_accepts_utf8_and_rejects_others() {
        let mut state = ProtocolState::default();
        state.handshake(false);
        state.accept(Frame::Negotiate(DO, CHARSET));
        assert_eq!(
            state
                .accept(Frame::Sub(CHARSET, b"\x01;UTF-8;ASCII".to_vec()))
                .0,
            iac::subnegotiation(CHARSET, b"\x02UTF-8")
        );
        assert!(state.capabilities.utf8);
        assert_eq!(
            state.accept(Frame::Sub(CHARSET, b"\x01;LATIN1".to_vec())).0,
            iac::subnegotiation(CHARSET, b"\x03")
        );
        assert!(!state.capabilities.utf8);
    }
}
