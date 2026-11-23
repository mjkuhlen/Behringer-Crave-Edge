//! Behringer Crave SysEx protocol.
//!
//! Documented commands come from the CRAVE manual, "SYSTEM EXCLUSIVE COMMANDS":
//! frame = F0 00 20 32 00 01 05 <command> <data...> F7
//!   (00 20 32 = Behringer GmbH manufacturer ID, 00 01 05 = Crave device header)
//!
//! VERIFIED-FROM-MANUAL commands: 0x11 0x17 0x19 0x1A 0x1B 0x1C 0x1D 0x1F
//!                                0x75 0x76 0x77 0x78 0x7D
//!
//! The 0x77/0x78 sequencer wire format is VERIFIED ON HARDWARE (nibble-packed
//! mono body; see the wire-layout docs below) and cross-checked with SynthHUB
//! (MIT). The 9-byte globals layout of 0x76/0x75 remains experimental.
//! Raw SysEx is always preserved verbatim for byte-exact backup/restore.

use serde::{Deserialize, Serialize};

pub const SYSEX_END: u8 = 0xF7;
/// F0 00 20 32 00 01 05
pub const HEADER: [u8; 7] = [0xF0, 0x00, 0x20, 0x32, 0x00, 0x01, 0x05];

pub const CMD_PITCH_BEND_RANGE: u8 = 0x11;
pub const CMD_MIDI_CLOCK_OUT: u8 = 0x17;
pub const CMD_CLOCK_EDGE: u8 = 0x19;
pub const CMD_CLOCK_TYPE: u8 = 0x1A;
pub const CMD_CLOCK_SOURCE: u8 = 0x1B;
pub const CMD_ACCENT_THRESHOLD: u8 = 0x1C;
pub const CMD_SEQ_AUTOPLAY: u8 = 0x1D;
pub const CMD_ASSIGN_MODE: u8 = 0x1F;
pub const CMD_GET_GLOBALS: u8 = 0x75;
pub const CMD_SET_GLOBALS: u8 = 0x76;
pub const CMD_GET_SEQ_DATA: u8 = 0x77;
pub const CMD_SET_SEQ_DATA: u8 = 0x78;
pub const CMD_FACTORY_RESET: u8 = 0x7D;

pub const CLOCK_SOURCES: [&str; 5] = ["INT", "MIDI", "USB", "EXT", "AUTO"];
pub const CLOCK_TYPES: [&str; 5] = ["1PPS", "2PPQ", "24PPQN", "48PPQN", "CV"];
pub const CLOCK_EDGES: [&str; 2] = ["Rising", "Falling"];
/// ASSIGN output modes, per manual "Assignable Output Modes" (1..16 -> index 0..15)
pub const ASSIGN_MODES: [&str; 16] = [
    "Sequencer Accent",
    "Sequencer Clock",
    "Sequencer Clock/2",
    "Sequencer Clock/4",
    "Sequencer Step Ramp",
    "Sequencer Step Saw",
    "Sequencer Step Triangle",
    "Sequencer Step Random",
    "Sequencer Step 1 Trigger",
    "MIDI Velocity",
    "MIDI Channel Pressure",
    "MIDI Pitch Bend",
    "MIDI CC1 (Mod Wheel)",
    "MIDI CC2 (Breath)",
    "MIDI CC4 (Foot)",
    "MIDI CC7 (Volume)",
];

/// Build a complete Crave SysEx frame.
pub fn frame(command: u8, data: &[u8]) -> Vec<u8> {
    let mut v = HEADER.to_vec();
    v.push(command);
    v.extend_from_slice(data);
    v.push(SYSEX_END);
    v
}

/// If `msg` is a Behringer Crave SysEx frame, return (command, payload).
pub fn parse(msg: &[u8]) -> Option<(u8, Vec<u8>)> {
    if msg.len() >= 9 && msg.starts_with(&HEADER) && msg.last() == Some(&SYSEX_END) {
        Some((msg[7], msg[8..msg.len() - 1].to_vec()))
    } else {
        None
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if cleaned.len() % 2 != 0 {
        return Err("hex string has odd number of digits".into());
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// Settings (documented single-parameter commands)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CraveSettings {
    pub clock_source: u8,      // 0..4  INT/MIDI/USB/EXT/AUTO
    pub clock_type: u8,        // 0..4  1PPS/2PPQ/24PPQN/48PPQN/CV
    pub clock_edge: u8,        // 0..1  rising/falling
    pub accent_threshold: u8,  // 0..0x7E
    pub seq_autoplay: u8,      // 0/1
    pub assign_mode: u8,       // 0..15
    pub pitch_bend_down: u8,   // semitones
    pub pitch_bend_up: u8,     // semitones
    pub midi_clock_out: u8,    // 0/1 (MIDI OUT/THRU clock forwarding)
    /// App-side MIDI channel for note messages (device channel is set by DIP
    /// switches on the rear panel; this must match it).
    pub midi_channel: u8, // 1..16
}

impl Default for CraveSettings {
    fn default() -> Self {
        Self {
            clock_source: 0,
            clock_type: 2, // 24 PPQN
            clock_edge: 0,
            accent_threshold: 100,
            seq_autoplay: 0,
            assign_mode: 12, // MIDI CC1 -> ASSIGN jack
            pitch_bend_down: 2,
            pitch_bend_up: 2,
            midi_clock_out: 0,
            midi_channel: 1,
        }
    }
}

impl CraveSettings {
    pub fn validate(&mut self) {
        self.clock_source = self.clock_source.min(4);
        self.clock_type = self.clock_type.min(4);
        self.clock_edge = self.clock_edge.min(1);
        self.accent_threshold = self.accent_threshold.min(0x7E);
        self.seq_autoplay = self.seq_autoplay.min(1);
        self.assign_mode = self.assign_mode.min(15);
        self.pitch_bend_down = self.pitch_bend_down.min(24);
        self.pitch_bend_up = self.pitch_bend_up.min(24);
        self.midi_clock_out = self.midi_clock_out.min(1);
        self.midi_channel = self.midi_channel.clamp(1, 16);
    }

    /// SysEx frames needed to move from `old` to `self`.
    /// midi_channel is app-side only (hardware channel = DIP switches).
    pub fn diff_commands(&self, old: &CraveSettings) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        if self.pitch_bend_down != old.pitch_bend_down || self.pitch_bend_up != old.pitch_bend_up {
            out.push((
                "pitch bend range".into(),
                frame(CMD_PITCH_BEND_RANGE, &[self.pitch_bend_down, self.pitch_bend_up]),
            ));
        }
        if self.midi_clock_out != old.midi_clock_out {
            out.push(("midi clock out".into(), frame(CMD_MIDI_CLOCK_OUT, &[self.midi_clock_out])));
        }
        if self.clock_edge != old.clock_edge {
            out.push(("clock edge".into(), frame(CMD_CLOCK_EDGE, &[self.clock_edge])));
        }
        if self.clock_type != old.clock_type {
            out.push(("clock type".into(), frame(CMD_CLOCK_TYPE, &[self.clock_type])));
        }
        if self.clock_source != old.clock_source {
            out.push(("clock source".into(), frame(CMD_CLOCK_SOURCE, &[self.clock_source])));
        }
        if self.accent_threshold != old.accent_threshold {
            out.push(("accent threshold".into(), frame(CMD_ACCENT_THRESHOLD, &[self.accent_threshold])));
        }
        if self.seq_autoplay != old.seq_autoplay {
            out.push(("seq autoplay".into(), frame(CMD_SEQ_AUTOPLAY, &[self.seq_autoplay])));
        }
        if self.assign_mode != old.assign_mode {
            out.push(("assign mode".into(), frame(CMD_ASSIGN_MODE, &[self.assign_mode])));
        }
        out
    }

    /// EXPERIMENTAL 9-byte globals layout for 0x76 (see module docs).
    pub fn to_globals_bytes(&self) -> [u8; 9] {
        [
            self.clock_source,
            self.clock_type,
            self.clock_edge,
            self.accent_threshold,
            self.seq_autoplay,
            self.assign_mode,
            self.pitch_bend_down,
            self.pitch_bend_up,
            self.midi_clock_out,
        ]
    }

    pub fn from_globals_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 9 {
            return None;
        }
        let mut s = Self {
            clock_source: b[0],
            clock_type: b[1],
            clock_edge: b[2],
            accent_threshold: b[3],
            seq_autoplay: b[4],
            assign_mode: b[5],
            pitch_bend_down: b[6],
            pitch_bend_up: b[7],
            midi_clock_out: b[8],
            midi_channel: 1,
        };
        s.validate();
        Some(s)
    }
}

// ---------------------------------------------------------------------------
// Pattern model + experimental wire encoding
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Step {
    /// MIDI note number 0..127
    pub note: u8,
    pub rest: bool,
    pub accent: bool,
    pub glide: bool,
    /// gate length for this step, percent of step duration (0..100)
    pub gate: u8,
    /// MIDI velocity 1..127 — stored per step on the device; accent = vel >= 112
    pub velocity: u8,
    /// device flags bits 0..6 (no audible effect; preserved across roundtrips)
    #[serde(default)]
    pub flags_low: u8,
    /// App/PWA-only retrigger count for this step. 1 is a normal hit.
    #[serde(default = "default_step_ratchet")]
    pub ratchet: u8,
}

fn default_step_ratchet() -> u8 {
    0
}

impl Default for Step {
    fn default() -> Self {
        Self { note: 36, rest: true, accent: false, glide: false, gate: 50, velocity: 100, flags_low: 0, ratchet: 1 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    /// active step count 1..=32
    pub length: u8,
    pub tempo: u16,
    /// 0..75 (%)
    pub swing: u8,
    /// global gate scale 10..100 (%)
    pub gate_length: u8,
    /// live-performance ratchet count 1..4 (retriggers per step, all steps).
    /// On the device: SHIFT + GLIDE during playback — NOT stored in pattern
    /// memory (manual, "LIVE PERFORMANCE"); here it is an app-playback setting
    /// saved with the pattern JSON.
    #[serde(default = "default_ratchet")]
    pub ratchet: u8,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
    /// raw SysEx captured from the device for this pattern, if any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_sysex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_bank: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_pattern: Option<u8>,
}

fn default_ratchet() -> u8 {
    1
}

impl Pattern {
    pub fn new_empty(name: &str) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            schema_version: 1,
            id: String::new(),
            name: name.to_string(),
            length: 16,
            tempo: 120,
            swing: 0,
            gate_length: 50,
            ratchet: 1,
            steps: (0..32).map(|_| Step::default()).collect(),
            created: now.clone(),
            updated: now,
            raw_sysex: None,
            device_bank: None,
            device_pattern: None,
        }
    }

    pub fn normalize(&mut self) {
        self.length = self.length.clamp(1, 32);
        self.tempo = self.tempo.clamp(20, 300);
        self.swing = self.swing.min(75);
        self.gate_length = self.gate_length.clamp(10, 100);
        self.ratchet = self.ratchet.clamp(1, 4);
        while self.steps.len() < 32 {
            self.steps.push(Step::default());
        }
        self.steps.truncate(32);
        for s in &mut self.steps {
            if s.ratchet == 0 { s.ratchet = self.ratchet; }
            s.note = s.note.min(127);
            s.gate = s.gate.min(100);
            s.velocity = s.velocity.clamp(1, 127);
            s.ratchet = s.ratchet.clamp(1, 4);
        }
    }
}

/// Wire layout for 0x77 get / 0x78 set — VERIFIED on real hardware
/// (2026-11-19: writes commit immediately, readback byte-exact, playback
/// confirmed by ear) and cross-checked against SynthHUB's HW-verified
/// type-0 mono codec (https://github.com/utajum/SynthHUB, MIT license).
///
/// 0x77 get [bank, pattern] -> device answers with a 0x78-command frame:
///   payload = [bank, pattern, nibble-packed body]
/// Nibble packing: each body byte travels as two bytes (hi nibble, lo nibble).
/// Body (131 bytes for a full 32-step pattern):
///   [0]     bank echo (not used by the editor)
///   [1..2]  length: body[1]*8 + body[2] + 1  (1..=32 on the Crave)
///   [3+4i]  step i: [note, trig, vel, flags]
///     note  = MIDI note number 0..127 (identity — no octave offset)
///     trig  = gate time in 1/48-step units: 0x30 = full step,
///             0x54..0x7e = held into the next step (slide/legato),
///             >= 0x7f = virgin filler (plays as a full gate), 0 = default
///     vel   = MIDI velocity 1..127 (0xff virgin -> plays as 64);
///             accent = velocity >= 112, there is no separate accent bit
///     flags = bit7 REST; bits 0..6 have no audible effect (preserved)
/// The Crave stores no per-step ratchet/probability in the pattern format.
/// Ratchet is app/PWA playback state. This app stores it per Step.ratchet
/// (1..4), and retriggers MIDI notes when app playback reaches that step.
/// There is NO save/commit handshake — a 0x78 write commits straight to
/// pattern memory. Byte-exact backup/restore replays the captured raw frame.
pub const STEP_COUNT: usize = 32;
pub const BODY_LEN: usize = 3 + STEP_COUNT * 4; // 131
const TRIG_FULL: u8 = 0x30;
const TRIG_SLIDE_MIN: u8 = 0x54;
const TRIG_SLIDE: u8 = 0x60;
const TRIG_VIRGIN: u8 = 0x7F;
const REST_BIT: u8 = 0x80;
const ACCENT_VEL: u8 = 112;

/// 1 byte -> (hi nibble, lo nibble), the 0x77/0x78 sequencer encoding.
pub fn pack_nibbles(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push((b >> 4) & 0x0f);
        out.push(b & 0x0f);
    }
    out
}

/// Inverse of pack_nibbles: (hi << 4) | lo per payload pair.
pub fn unpack_nibbles(packed: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(packed.len() / 2);
    for pair in packed.chunks_exact(2) {
        out.push(((pair[0] & 0x0f) << 4) | (pair[1] & 0x0f));
    }
    out
}

pub fn encode_pattern_data(bank: u8, pattern: u8, p: &Pattern) -> Vec<u8> {
    let n = (p.length as usize).clamp(1, STEP_COUNT);
    let mut body = vec![0u8; BODY_LEN];
    body[0] = bank;
    let value = (n - 1) as u8; // body[1]*8 + body[2] == length-1
    body[1] = (value >> 3) & 0x7f;
    body[2] = value & 7;
    for i in 0..STEP_COUNT {
        let s = p.steps.get(i).cloned().unwrap_or_default();
        let off = 3 + 4 * i;
        let trig = if s.glide {
            TRIG_SLIDE
        } else if s.gate == 0 {
            0
        } else {
            ((s.gate as u16 * TRIG_FULL as u16 + 50) / 100).clamp(6, 0x48) as u8
        };
        let vel = if s.accent { s.velocity.max(ACCENT_VEL) } else { s.velocity.clamp(1, 127) };
        body[off] = s.note & 0x7f;
        body[off + 1] = trig;
        body[off + 2] = vel;
        body[off + 3] = (s.flags_low & 0x7f) | if s.rest { REST_BIT } else { 0 };
    }
    let mut payload = vec![bank, pattern];
    payload.extend(pack_nibbles(&body));
    payload
}

pub fn decode_pattern_data(payload: &[u8]) -> Option<(u8, u8, Pattern)> {
    if payload.len() < 2 + 7 * 2 {
        return None;
    }
    let bank = payload[0];
    let pat = payload[1];
    let body = unpack_nibbles(&payload[2..]);
    if body.len() < 7 {
        return None;
    }
    let mut p = Pattern::new_empty(&format!("Device dump B{} P{}", bank + 1, pat + 1));
    let length = (body[1] as usize * 8 + body[2] as usize + 1).clamp(1, STEP_COUNT);
    p.length = length as u8;
    for i in 0..STEP_COUNT {
        let off = 3 + 4 * i;
        if off + 3 >= body.len() {
            break;
        }
        let note = body[off] & 0x7f;
        let trig = body[off + 1];
        let vel_raw = body[off + 2];
        let flags = body[off + 3];
        let (gate, glide) = if trig == 0 {
            (0, false)
        } else if trig >= TRIG_VIRGIN {
            (100, false) // virgin filler -> full gate
        } else if trig >= TRIG_SLIDE_MIN {
            (100, true) // held into the next step
        } else {
            (((trig as u16 * 100 + 24) / TRIG_FULL as u16) as u8, false)
        };
        let velocity = if vel_raw > 0x7f { 64 } else { vel_raw.max(1) };
        p.steps[i] = Step {
            note,
            rest: flags & REST_BIT != 0,
            accent: velocity >= ACCENT_VEL,
            glide,
            gate,
            velocity,
            flags_low: flags & 0x7f,
            ratchet: 1, // default for loaded patterns
        };
    }
    p.normalize();
    p.device_bank = Some(bank);
    p.device_pattern = Some(pat);
    Some((bank, pat, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_payload(hex: &str) -> Vec<u8> {
        let frame = parse_hex(hex.trim()).unwrap();
        let (cmd, payload) = parse(&frame).unwrap();
        assert_eq!(cmd, CMD_SET_SEQ_DATA, "device answers gets with 0x78 frames");
        payload
    }

    #[test]
    fn nibble_pack_roundtrip_all_bytes() {
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(unpack_nibbles(&pack_nibbles(&bytes)), bytes);
    }

    /// Ground truth: real device dump (user's B1P1 arp, captured pre-corruption).
    #[test]
    fn decode_real_dump_b1p1() {
        let payload = fixture_payload(include_str!("../tests/fixtures/crave_b1p1.hex"));
        let (bank, pat, p) = decode_pattern_data(&payload).expect("decode");
        assert_eq!((bank, pat), (0, 0));
        assert_eq!(p.length, 4);
        assert_eq!(p.steps[0].note, 36); // C2 — identity note mapping
        assert!(p.steps[0].glide); // trig 0x70 = slide/hold
        assert_eq!(p.steps[0].velocity, 64);
        assert_eq!(p.steps[1].note, 39); // D#2
        assert_eq!(p.steps[1].gate, 0); // trig 0x00
        assert_eq!(p.steps[2].note, 41); // F2
        assert_eq!(p.steps[3].note, 36); // C2
        assert!(p.steps[3].glide);
        // beyond the active length the device keeps stale step data (not rests)
        assert!(!p.steps[4].rest);
        assert_eq!(p.steps[4].note, 49); // C#3
    }

    /// Ground truth: factory 32-step arp (survey capture, slot B1P3).
    #[test]
    fn decode_real_dump_b1p3_32step_arp() {
        let payload = fixture_payload(include_str!("../tests/fixtures/crave_b1p3.hex"));
        let (bank, pat, p) = decode_pattern_data(&payload).expect("decode");
        assert_eq!((bank, pat), (0, 2));
        assert_eq!(p.length, 32);
        let notes: Vec<u8> = p.steps.iter().map(|s| s.note).collect();
        assert_eq!(&notes[..8], &[48, 51, 53, 58, 55, 48, 51, 53]);
        assert!(p.steps.iter().all(|s| !s.rest));
        assert!(p.steps.iter().all(|s| s.gate == 100)); // trig 0x30 = full step
        assert!(p.steps.iter().all(|s| s.flags_low == 0));
    }

    /// encode -> decode -> encode must be byte-stable (canonical form), and
    /// semantically lossless for everything the device actually stores.
    #[test]
    fn canonical_encode_stability() {
        for hex in [
            include_str!("../tests/fixtures/crave_b1p1.hex"),
            include_str!("../tests/fixtures/crave_b1p3.hex"),
        ] {
            let payload = fixture_payload(hex);
            let (bank, pat, p) = decode_pattern_data(&payload).unwrap();
            let enc = encode_pattern_data(bank, pat, &p);
            let (_, _, p2) = decode_pattern_data(&enc).unwrap();
            assert_eq!(enc, encode_pattern_data(bank, pat, &p2));
            assert_eq!(p.length, p2.length);
            for (a, b) in p.steps.iter().zip(p2.steps.iter()) {
                assert_eq!(a.note, b.note);
                assert_eq!(a.rest, b.rest);
                assert_eq!(a.accent, b.accent);
                assert_eq!(a.glide, b.glide);
                assert_eq!(a.velocity, b.velocity);
                assert_eq!(a.flags_low, b.flags_low);
                assert!(a.gate.abs_diff(b.gate) <= 1);
            }
        }
    }

    #[test]
    fn accent_is_velocity_112() {
        let mut p = Pattern::new_empty("t");
        p.length = 2;
        p.steps[0] = Step { note: 48, rest: false, accent: true, velocity: 100, ..Default::default() };
        p.steps[1] = Step { note: 48, rest: false, accent: false, velocity: 100, ..Default::default() };
        let enc = encode_pattern_data(0, 0, &p);
        let (_, _, d) = decode_pattern_data(&enc).unwrap();
        assert_eq!(d.steps[0].velocity, 112); // accent bumps to >= 112
        assert!(d.steps[0].accent);
        assert_eq!(d.steps[1].velocity, 100);
        assert!(!d.steps[1].accent);
    }

    #[test]
    fn length_encoding_roundtrips_1_to_32() {
        for len in 1u8..=32 {
            let mut p = Pattern::new_empty("t");
            p.length = len;
            let enc = encode_pattern_data(3, 5, &p);
            let (bank, pat, d) = decode_pattern_data(&enc).unwrap();
            assert_eq!((bank, pat), (3, 5));
            assert_eq!(d.length, len, "length {} did not roundtrip", len);
        }
    }

    #[test]
    fn gate_quantization_is_stable() {
        // any gate percent -> trig -> gate -> trig must reach a fixed point
        for gate in 0u8..=100 {
            let mut p = Pattern::new_empty("t");
            p.length = 1;
            p.steps[0] = Step { gate, rest: false, ..Default::default() };
            let enc = encode_pattern_data(0, 0, &p);
            let (_, _, d) = decode_pattern_data(&enc).unwrap();
            let enc2 = encode_pattern_data(0, 0, &d);
            assert_eq!(enc, enc2, "gate {} not canonical", gate);
        }
    }
}
