//! Built-in Crave simulator: responds to the documented SysEx command set so
//! the whole app (librarian, settings, dump/restore, playback) can be exercised
//! without hardware. Implements the same EXPERIMENTAL wire layouts as crave.rs.
//! This is explicitly a simulator — it is not a claim about real device behavior.

use crate::crave::{self, CraveSettings, Pattern};
use std::collections::HashMap;

pub struct SimCrave {
    pub settings: CraveSettings,
    /// (bank, pattern) -> Pattern
    pub patterns: HashMap<(u8, u8), Pattern>,
}

impl SimCrave {
    pub fn new() -> Self {
        let mut sim = Self { settings: CraveSettings::default(), patterns: HashMap::new() };
        sim.seed();
        sim
    }

    fn seed(&mut self) {
        // B1 P1: classic 16-step acid line
        let mut p = Pattern::new_empty("Sim: Acid 16");
        p.length = 16;
        p.tempo = 128;
        p.swing = 8;
        p.gate_length = 45;
        let notes = [36, 36, 48, 36, 39, 36, 43, 36, 36, 48, 36, 51, 48, 43, 39, 34];
        for (i, n) in notes.iter().enumerate() {
            p.steps[i] = crave::Step {
                note: *n,
                rest: false,
                accent: i % 4 == 0,
                glide: i == 2 || i == 9,
                gate: 45,
                velocity: 100,
                flags_low: 0,
                ratchet: 1,
            };
        }
        self.patterns.insert((0, 0), p);

        // B1 P2: sparse halftime
        let mut p2 = Pattern::new_empty("Sim: Halftime");
        p2.length = 16;
        p2.tempo = 90;
        p2.gate_length = 70;
        for i in [0usize, 6, 8, 14] {
            p2.steps[i] = crave::Step {
                note: [36, 41, 36, 43][[0, 6, 8, 14].iter().position(|&x| x == i).unwrap()],
                rest: false,
                accent: i == 0,
                glide: false,
                gate: 70,
                velocity: 110,
                flags_low: 0,
                ratchet: 1,
            };
        }
        self.patterns.insert((0, 1), p2);
    }

    /// Handle an incoming MIDI message; return an optional SysEx response.
    pub fn handle(&mut self, msg: &[u8]) -> Option<Vec<u8>> {
        // Universal identity request -> identity reply (simulated)
        if msg == [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7] {
            return Some(vec![
                0xF0, 0x7E, 0x7F, 0x06, 0x02, 0x00, 0x20, 0x32, 0x05, 0x00, 0x00, 0x00, 0x01,
                0xF7,
            ]);
        }
        let (cmd, data) = crave::parse(msg)?;
        match cmd {
            crave::CMD_PITCH_BEND_RANGE if data.len() >= 2 => {
                self.settings.pitch_bend_down = data[0];
                self.settings.pitch_bend_up = data[1];
                None
            }
            crave::CMD_MIDI_CLOCK_OUT if !data.is_empty() => {
                self.settings.midi_clock_out = data[0];
                None
            }
            crave::CMD_CLOCK_EDGE if !data.is_empty() => {
                self.settings.clock_edge = data[0];
                None
            }
            crave::CMD_CLOCK_TYPE if !data.is_empty() => {
                self.settings.clock_type = data[0];
                None
            }
            crave::CMD_CLOCK_SOURCE if !data.is_empty() => {
                self.settings.clock_source = data[0];
                None
            }
            crave::CMD_ACCENT_THRESHOLD if !data.is_empty() => {
                self.settings.accent_threshold = data[0];
                None
            }
            crave::CMD_SEQ_AUTOPLAY if !data.is_empty() => {
                self.settings.seq_autoplay = data[0];
                None
            }
            crave::CMD_ASSIGN_MODE if !data.is_empty() => {
                self.settings.assign_mode = data[0];
                None
            }
            crave::CMD_GET_GLOBALS => {
                Some(crave::frame(crave::CMD_GET_GLOBALS, &self.settings.to_globals_bytes()))
            }
            crave::CMD_SET_GLOBALS => {
                if let Some(s) = CraveSettings::from_globals_bytes(&data) {
                    let ch = self.settings.midi_channel;
                    self.settings = s;
                    self.settings.midi_channel = ch;
                }
                None
            }
            crave::CMD_GET_SEQ_DATA if data.len() >= 2 => {
                let key = (data[0], data[1]);
                let p = self
                    .patterns
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| Pattern::new_empty("Empty"));
                // real hardware answers a 0x77 get with a 0x78-command frame
                Some(crave::frame(
                    crave::CMD_SET_SEQ_DATA,
                    &crave::encode_pattern_data(key.0, key.1, &p),
                ))
            }
            crave::CMD_SET_SEQ_DATA => {
                if let Some((bank, pat, mut p)) = crave::decode_pattern_data(&data) {
                    p.id = format!("sim-b{}-p{}", bank, pat);
                    self.patterns.insert((bank, pat), p);
                }
                None
            }
            crave::CMD_FACTORY_RESET => {
                self.settings = CraveSettings::default();
                self.patterns.clear();
                self.seed();
                None
            }
            _ => None,
        }
    }
}
