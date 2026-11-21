//! MIDI clock transport: drives a device's INTERNAL sequencer when its clock
//! source is set to USB/MIDI (Crave: clock source
//! setting). Sends MIDI Start (0xFA), 24 PPQN clock (0xF8) at the given BPM,
//! and MIDI Stop (0xFC). This is distinct from app-side note playback — the
//! device plays its own panel-programmed pattern under the app's tempo.

use crate::midi::{Event, MidiManager};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub struct ClockTransport {
    midi: Arc<MidiManager>,
    events: broadcast::Sender<Event>,
    generation: Arc<AtomicU64>,
}

impl ClockTransport {
    pub fn new(midi: Arc<MidiManager>, events: broadcast::Sender<Event>) -> Self {
        Self { midi, events, generation: Arc::new(AtomicU64::new(0)) }
    }

    pub fn is_running(&self) -> bool {
        self.generation.load(Ordering::SeqCst) % 2 == 1
    }

    pub fn stop(&self) {
        let g = self.generation.load(Ordering::SeqCst);
        if g % 2 == 1 {
            self.generation.fetch_add(1, Ordering::SeqCst);
            let _ = self.midi.send(&[0xFC]); // MIDI Stop
            let _ = self.events.send(Event::Log {
                device: self.midi.id(),
                message: "MIDI clock stopped (0xFC)".into(),
            });
        }
    }

    pub fn start(&self, bpm: u16) {
        self.stop();
        let bpm = bpm.clamp(20, 300) as f64;
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();
        let midi = self.midi.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let _ = midi.send(&[0xFA]); // MIDI Start
            let _ = events.send(Event::Log {
                device: midi.id(),
                message: format!("MIDI clock started (0xFA + 24 PPQN at {} BPM)", bpm as u16),
            });
            let period = Duration::from_secs_f64(60.0 / bpm / 24.0);
            let mut ticker = tokio::time::interval(period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if generation.load(Ordering::SeqCst) != gen {
                    break;
                }
                if midi.send(&[0xF8]).is_err() {
                    let _ = events.send(Event::Log {
                        device: midi.id(),
                        message: "MIDI clock stopped: send failed (device disconnected?)".into(),
                    });
                    break;
                }
            }
            let _ = generation.compare_exchange(gen, gen + 1, Ordering::SeqCst, Ordering::SeqCst);
        });
    }
}
