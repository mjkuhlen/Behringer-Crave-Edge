//! Behringer Edge SysEx protocol (protocol family 3).
//!
//! Frame: F0 00 20 32 00 01 22 00 <cmd> <data...> F7
//!   (00 20 32 = Music Tribe mfr, 00 01 22 = Edge model id, 00 = device id;
//!    protocol >= 3 inserts the device-id byte, so cmd lives at index 8)
//!
//! Settings map from SynthTribe's Devices/EDGE/config.json, wire behavior per
//! SynthHUB's generic driver (MIT): write = header + pkt + value byte(s);
//! readback = 0x75 request -> 0x76 dump, config "index" = payload byte offset.
//!   pkt 0x11  pitch bend semitones 0..12        (globals index 2)
//!   pkt 0x12  key priority 0..2                 (globals index 3)
//!   pkt 0x28  MIDI through [m2u, mst, u2m] 0/1  (globals indices 5, 6, 7)
//! Key-priority labels (Low/High/Last) follow the Behringer convention and
//! are marked for hardware confirmation.
//!
//! The Edge's dual 8-step sequencer is NOT exposed through documented SysEx
//! (neither SynthTribe nor SynthHUB has an Edge sequencer function) — it is
//! panel-only until probing shows otherwise. No pattern dump/write here yet.

use serde::{Deserialize, Serialize};

pub const SYSEX_START: u8 = 0xF0;
pub const SYSEX_END: u8 = 0xF7;
/// F0 00 20 32 00 01 22 00
pub const HEADER: [u8; 8] = [0xF0, 0x00, 0x20, 0x32, 0x00, 0x01, 0x22, 0x00];

pub const CMD_PITCH_BEND: u8 = 0x11;
pub const CMD_KEY_PRIORITY: u8 = 0x12;
pub const CMD_MIDI_THROUGH: u8 = 0x28;
pub const CMD_GET_GLOBALS: u8 = 0x75;
pub const CMD_ENTIRE_PARAMS: u8 = 0x76;

pub const KEY_PRIORITIES: [&str; 3] = ["Low", "High", "Last"];

pub fn frame(command: u8, data: &[u8]) -> Vec<u8> {
    let mut v = HEADER.to_vec();
    v.push(command);
    v.extend_from_slice(data);
    v.push(SYSEX_END);
    v
}

/// If `msg` is an Edge SysEx frame, return (command, payload).
pub fn parse(msg: &[u8]) -> Option<(u8, Vec<u8>)> {
    if msg.len() >= 10 && msg.starts_with(&HEADER) && msg.last() == Some(&SYSEX_END) {
        Some((msg[8], msg[9..msg.len() - 1].to_vec()))
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeSettings {
    /// pitch bend range in semitones, 0..12
    pub pitch_bend: u8,
    /// 0..2 (Low/High/Last — labels pending hardware confirmation)
    pub key_priority: u8,
    /// MIDI DIN -> USB throughput, 0/1
    pub through_m2u: u8,
    /// MIDI DIN -> DIN thru, 0/1
    pub through_mst: u8,
    /// USB -> MIDI DIN out, 0/1
    pub through_u2m: u8,
    /// app-side MIDI channel for note messages (Edge channel is fixed by
    /// default; kept app-side like the Crave's DIP-switch channel)
    pub midi_channel: u8,
}

impl Default for EdgeSettings {
    fn default() -> Self {
        Self {
            pitch_bend: 12,
            key_priority: 0,
            through_m2u: 0,
            through_mst: 0,
            through_u2m: 0,
            midi_channel: 1,
        }
    }
}

impl EdgeSettings {
    pub fn validate(&mut self) {
        self.pitch_bend = self.pitch_bend.min(12);
        self.key_priority = self.key_priority.min(2);
        self.through_m2u = self.through_m2u.min(1);
        self.through_mst = self.through_mst.min(1);
        self.through_u2m = self.through_u2m.min(1);
        self.midi_channel = self.midi_channel.clamp(1, 16);
    }

    /// SysEx frames needed to move from `old` to `self`.
    pub fn diff_commands(&self, old: &EdgeSettings) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        if self.pitch_bend != old.pitch_bend {
            // subtype-1 pitchbend wire format: pkt 0x11 <value> 0x00
            out.push(("pitch bend range".into(), frame(CMD_PITCH_BEND, &[self.pitch_bend, 0])));
        }
        if self.key_priority != old.key_priority {
            out.push(("key priority".into(), frame(CMD_KEY_PRIORITY, &[self.key_priority])));
        }
        if self.through_m2u != old.through_m2u
            || self.through_mst != old.through_mst
            || self.through_u2m != old.through_u2m
        {
            out.push((
                "midi through".into(),
                frame(CMD_MIDI_THROUGH, &[self.through_m2u, self.through_mst, self.through_u2m]),
            ));
        }
        out
    }

    /// Decode a 0x76 globals dump. Real Edge dump observed on hardware:
    /// 5 bytes [?, ?, pitch_bend, key_priority, ?] — indices 2/3 per the
    /// SynthTribe config map. Through flags (indices 5/6/7) are only present
    /// in longer dumps; absent them they stay at defaults (write-only path).
    pub fn from_globals_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 4 {
            return None;
        }
        let mut s = Self {
            pitch_bend: b[2],
            key_priority: b[3],
            through_m2u: if b.len() > 5 { b[5] } else { 0 },
            through_mst: if b.len() > 6 { b[6] } else { 0 },
            through_u2m: if b.len() > 7 { b[7] } else { 0 },
            midi_channel: 1,
        };
        s.validate();
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_layout_protocol3() {
        let f = frame(CMD_PITCH_BEND, &[12, 0]);
        assert_eq!(f, vec![0xF0, 0x00, 0x20, 0x32, 0x00, 0x01, 0x22, 0x00, 0x11, 0x0C, 0x00, 0xF7]);
        let (cmd, data) = parse(&f).unwrap();
        assert_eq!(cmd, CMD_PITCH_BEND);
        assert_eq!(data, vec![12, 0]);
    }

    #[test]
    fn pitch_bend_write_has_trailing_zero() {
        let mut s = EdgeSettings::default();
        s.pitch_bend = 7;
        let cmds = s.diff_commands(&EdgeSettings::default());
        assert_eq!(cmds[0].1, frame(CMD_PITCH_BEND, &[7, 0]));
    }

    #[test]
    fn real_5byte_globals_dump() {
        // captured from a real Edge: F0 00 20 32 00 01 22 00 76 01 01 02 02 01 F7
        let dump = [0x01, 0x01, 0x02, 0x02, 0x01];
        let s = EdgeSettings::from_globals_bytes(&dump).unwrap();
        assert_eq!(s.pitch_bend, 2);
        assert_eq!(s.key_priority, 2); // Last
    }

    #[test]
    fn through_is_one_composite_frame() {
        let mut s = EdgeSettings::default();
        s.through_u2m = 1;
        let cmds = s.diff_commands(&EdgeSettings::default());
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].1, frame(CMD_MIDI_THROUGH, &[0, 0, 1]));
    }

    #[test]
    fn globals_index_map() {
        let dump = [0, 0, 7, 2, 0, 1, 0, 1];
        let s = EdgeSettings::from_globals_bytes(&dump).unwrap();
        assert_eq!(s.pitch_bend, 7);
        assert_eq!(s.key_priority, 2);
        assert_eq!(s.through_m2u, 1);
        assert_eq!(s.through_mst, 0);
        assert_eq!(s.through_u2m, 1);
    }
}
