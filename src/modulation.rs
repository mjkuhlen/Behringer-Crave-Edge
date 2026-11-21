//! Modulation engine: app-side LFOs routed to MIDI CC / pitch bend / channel
//! pressure. The Crave's analog pots have no SysEx control, so software
//! modulation reaches the patch bay through the ASSIGN jack: ASSIGN mode
//! (0x1F) selects which MIDI source appears there (CC1/2/4/7, velocity,
//! pitch bend, pressure…). Patch ASSIGN → VCF cutoff, VCO pitch, VCA, etc.
//! Only ONE source feeds ASSIGN at a time; the route-assign API switches it.

use crate::midi::{Event, MidiManager};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WaveShape {
    Sine,
    Triangle,
    Square,
    Saw,
    Random,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModTarget {
    Cc1,
    Cc2,
    Cc4,
    Cc7,
    Pitchbend,
    Aftertouch,
}

impl ModTarget {
    /// ASSIGN mode index (0x1F value) that routes this source to the jack.
    pub fn assign_mode(self) -> Option<u8> {
        match self {
            ModTarget::Cc1 => Some(12),
            ModTarget::Cc2 => Some(13),
            ModTarget::Cc4 => Some(14),
            ModTarget::Cc7 => Some(15),
            ModTarget::Pitchbend => Some(11),
            ModTarget::Aftertouch => Some(10),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModSlot {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "def_shape")]
    pub shape: WaveShape,
    /// free-running rate in Hz (used when sync_beats is null)
    #[serde(default = "def_rate")]
    pub rate_hz: f64,
    /// tempo-synced period in beats (0.25..8); null = free Hz
    #[serde(default)]
    pub sync_beats: Option<f64>,
    /// modulation amount 0..127 (CC units; bend scales ×64)
    #[serde(default = "def_depth")]
    pub depth: u8,
    /// center value 0..127
    #[serde(default = "def_center")]
    pub center: u8,
    #[serde(default = "def_target")]
    pub target: ModTarget,
}

fn def_shape() -> WaveShape {
    WaveShape::Sine
}
fn def_rate() -> f64 {
    0.5
}
fn def_depth() -> u8 {
    32
}
fn def_center() -> u8 {
    64
}
fn def_target() -> ModTarget {
    ModTarget::Cc1
}

impl Default for ModSlot {
    fn default() -> Self {
        Self {
            enabled: false,
            shape: def_shape(),
            rate_hz: def_rate(),
            sync_beats: None,
            depth: def_depth(),
            center: def_center(),
            target: def_target(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModConfig {
    /// tempo for synced slots (BPM)
    #[serde(default = "def_tempo")]
    pub tempo: u16,
    /// MIDI channel 1..16 — must match the Crave's rear DIP switches
    #[serde(default = "one")]
    pub channel: u8,
    #[serde(default = "def_slots")]
    pub slots: Vec<ModSlot>,
}

fn def_tempo() -> u16 {
    120
}
fn one() -> u8 {
    1
}
fn def_slots() -> Vec<ModSlot> {
    vec![ModSlot::default(), ModSlot::default()]
}

impl Default for ModConfig {
    fn default() -> Self {
        Self { tempo: def_tempo(), channel: 1, slots: def_slots() }
    }
}

impl ModConfig {
    pub fn normalize(&mut self) {
        self.tempo = self.tempo.clamp(20, 300);
        self.channel = self.channel.clamp(1, 16);
        self.slots.truncate(4);
        while self.slots.len() < 2 {
            self.slots.push(ModSlot::default());
        }
        for s in &mut self.slots {
            s.rate_hz = s.rate_hz.clamp(0.05, 20.0);
            if let Some(b) = s.sync_beats {
                s.sync_beats = Some(b.clamp(0.25, 8.0));
            }
            s.depth = s.depth.min(127);
            s.center = s.center.min(127);
        }
    }
}

pub fn load_config(data_dir: &std::path::Path) -> ModConfig {
    std::fs::read_to_string(data_dir.join("modulation.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_config(data_dir: &std::path::Path, c: &ModConfig) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(data_dir.join("modulation.json"), serde_json::to_string_pretty(c)?)?;
    Ok(())
}

pub struct ModEngine {
    midi: Arc<MidiManager>,
    events: broadcast::Sender<Event>,
    pub config: Arc<Mutex<ModConfig>>,
    generation: Arc<AtomicU64>,
}

impl ModEngine {
    pub fn new(
        midi: Arc<MidiManager>,
        events: broadcast::Sender<Event>,
        config: ModConfig,
    ) -> Self {
        Self {
            midi,
            events,
            config: Arc::new(Mutex::new(config)),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.generation.load(Ordering::SeqCst) % 2 == 1
    }

    pub fn stop(&self) {
        let g = self.generation.load(Ordering::SeqCst);
        if g % 2 == 1 {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn start(&self) {
        self.stop();
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();
        let midi = self.midi.clone();
        let events = self.events.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            let tick = Duration::from_millis(20); // 50 Hz update rate
            let mut phases: Vec<f64> = vec![0.0; 4];
            let mut rng: u64 = 0x9E3779B97F4A7C15;
            let mut last_vals: Vec<i32> = vec![-1; 4];
            let mut next = tokio::time::Instant::now();
            loop {
                tokio::time::sleep_until(next).await;
                if generation.load(Ordering::SeqCst) != gen {
                    break;
                }
                next += tick;
                let cfg = config.lock().unwrap().clone();
                let ch = (cfg.channel.clamp(1, 16) - 1) & 0x0F;
                let dt = tick.as_secs_f64();
                for (idx, slot) in cfg.slots.iter().enumerate() {
                    if !slot.enabled {
                        continue;
                    }
                    let period = match slot.sync_beats {
                        Some(beats) => (60.0 / cfg.tempo as f64) * beats,
                        None => 1.0 / slot.rate_hz.max(0.05),
                    };
                    phases[idx] += dt / period.max(0.01);
                    let new_cycle = phases[idx] >= 1.0;
                    if new_cycle {
                        phases[idx] -= phases[idx].floor();
                    }
                    let ph = phases[idx];
                    let wave: f64 = match slot.shape {
                        WaveShape::Sine => (ph * std::f64::consts::TAU).sin(),
                        WaveShape::Triangle => {
                            if ph < 0.5 { 4.0 * ph - 1.0 } else { 3.0 - 4.0 * ph }
                        }
                        WaveShape::Square => {
                            if ph < 0.5 { 1.0 } else { -1.0 }
                        }
                        WaveShape::Saw => 2.0 * ph - 1.0,
                        WaveShape::Random => {
                            // sample & hold: new random value each cycle
                            if new_cycle || last_vals[idx] < 0 {
                                rng ^= rng << 13;
                                rng ^= rng >> 7;
                                rng ^= rng << 17;
                                last_vals[idx] = (rng % 255) as i32 - 127;
                            }
                            last_vals[idx] as f64 / 127.0
                        }
                    };
                    let msg: Vec<u8> = match slot.target {
                        ModTarget::Pitchbend => {
                            let v = (8192.0 + wave * slot.depth as f64 * 64.0)
                                .clamp(0.0, 16383.0) as u16;
                            vec![0xE0 | ch, (v & 0x7F) as u8, ((v >> 7) & 0x7F) as u8]
                        }
                        ModTarget::Aftertouch => {
                            let v = (slot.center as f64 + wave * slot.depth as f64)
                                .clamp(0.0, 127.0) as u8;
                            vec![0xD0 | ch, v]
                        }
                        cc => {
                            let n = match cc {
                                ModTarget::Cc1 => 1,
                                ModTarget::Cc2 => 2,
                                ModTarget::Cc4 => 4,
                                ModTarget::Cc7 => 7,
                                _ => unreachable!(),
                            };
                            let v = (slot.center as f64 + wave * slot.depth as f64)
                                .clamp(0.0, 127.0) as u8;
                            vec![0xB0 | ch, n, v]
                        }
                    };
                    let _ = midi.send(&msg);
                }
            }
            let _ = events.send(Event::Log { device: midi.id(), message: "modulation engine stopped".into() });
            let _ = generation.compare_exchange(gen, gen + 1, Ordering::SeqCst, Ordering::SeqCst);
        });
    }
}
