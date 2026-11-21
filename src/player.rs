//! Pattern playback: sends notes over MIDI with tempo/swing/gate/accent and
//! live ratchet (retriggers per step). Monophonic like the Crave voice: a new
//! note kills the previous one. Song mode chains patterns with per-entry
//! repeats and transpose; the device itself has no song memory, so songs are
//! played app-side through the Crave's MIDI input.

use crate::crave::Pattern;
use crate::midi::{Event, MidiManager};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// One song entry: a pattern played `repeats` times (0 = forever), transposed.
#[derive(Debug, Clone)]
pub struct SongEntry {
    pub pattern: Pattern,
    pub repeats: u16,
    pub transpose: i8,
}

pub struct Player {
    midi: Arc<MidiManager>,
    events: broadcast::Sender<Event>,
    generation: Arc<AtomicU64>,
}

impl Player {
    pub fn new(midi: Arc<MidiManager>, events: broadcast::Sender<Event>) -> Self {
        Self { midi, events, generation: Arc::new(AtomicU64::new(0)) }
    }

    pub fn is_playing(&self) -> bool {
        // a live task holds the current generation; stop() bumps it
        self.generation.load(Ordering::SeqCst) % 2 == 1
    }

    pub fn stop(&self) {
        // bump to an even generation -> any running odd-gen task exits
        let g = self.generation.load(Ordering::SeqCst);
        if g % 2 == 1 {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn play(&self, pattern: Pattern, channel: u8) {
        self.play_entries(
            vec![SongEntry { pattern, repeats: 0, transpose: 0 }],
            channel,
            false,
        );
    }

    pub fn play_song(&self, entries: Vec<SongEntry>, channel: u8, loop_song: bool) {
        self.play_entries(entries, channel, loop_song);
    }

    fn play_entries(&self, mut entries: Vec<SongEntry>, channel: u8, loop_song: bool) {
        for e in &mut entries {
            e.pattern.normalize();
            e.repeats = e.repeats.min(999);
        }
        entries.retain(|e| e.pattern.length > 0);
        if entries.is_empty() {
            return;
        }
        self.stop();
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1; // odd = running
        let generation = self.generation.clone();
        let midi = self.midi.clone();
        let events = self.events.clone();
        let ch = (channel.clamp(1, 16) - 1) & 0x0F;

        tokio::spawn(async move {
            let mut entry_idx = 0usize;
            let mut repeats_done = 0u16;
            let mut i = 0usize;
            let mut next = tokio::time::Instant::now();
            // scheduled raw MIDI messages (note-offs, ratchet retriggers)
            let mut pending: Vec<(tokio::time::Instant, Vec<u8>)> = Vec::new();
            let mut last_note: Option<u8> = None;
            let mut failed = false;

            macro_rules! cur {
                () => {
                    entries[entry_idx]
                };
            }

            let _ = events.send(Event::SongPosition {
                device: midi.id(),
                entry: entry_idx,
                repeat: repeats_done,
                playing: true,
            });

            loop {
                // wake for the next step or the earliest scheduled note-off/ratchet
                let wake = pending.iter().map(|(t, _)| *t).min().map_or(next, |t| t.min(next));
                tokio::time::sleep_until(wake).await;
                if generation.load(Ordering::SeqCst) != gen {
                    break;
                }
                let now = tokio::time::Instant::now();
                // fire due scheduled messages
                pending.retain(|(t, msg)| {
                    if *t <= now {
                        let _ = midi.send(msg);
                        false
                    } else {
                        true
                    }
                });
                if now < next {
                    continue;
                }
                let _ = events.send(Event::Playhead { device: midi.id(), step: i, playing: true });

                let pat = &cur!().pattern;
                let tempo = pat.tempo as f64;
                let tick = Duration::from_secs_f64(60.0 / tempo / 4.0);
                let swing_frac = pat.swing as f64 / 200.0;
                let gate_scale = pat.gate_length as f64 / 100.0;
                let step = &pat.steps[i];
                let note = (step.note as i16 + cur!().transpose as i16).clamp(0, 127) as u8;

                if !step.rest {
                    let vel = if step.accent { 127 } else { step.velocity };
                    let ratchet = pat.ratchet.max(1) as usize;
                    if ratchet == 1 {
                        if midi.send(&[0x90 | ch, note, vel]).is_err() {
                            failed = true;
                            break;
                        }
                        // cap below the step so the off never lands on the next note-on
                        let gate = (tick.as_secs_f64() * (step.gate as f64 / 100.0) * gate_scale)
                            .min(tick.as_secs_f64() * 0.9);
                        pending.push((
                            now + Duration::from_secs_f64(gate.max(0.005)),
                            vec![0x80 | ch, note, 0],
                        ));
                    } else {
                        // ratchet: N retriggers inside the step, each restarting
                        // the envelope (note-off + note-on), like SHIFT+GLIDE
                        let slice = tick.as_secs_f64() / ratchet as f64;
                        for k in 0..ratchet {
                            let at = now + Duration::from_secs_f64(slice * k as f64);
                            let mut msgs = Vec::new();
                            if k > 0 {
                                msgs.push(vec![0x80 | ch, note, 0]);
                            }
                            msgs.push(vec![0x90 | ch, note, vel]);
                            for m in msgs {
                                if k == 0 {
                                    if midi.send(&m).is_err() {
                                        failed = true;
                                        break;
                                    }
                                } else {
                                    pending.push((at, m));
                                }
                            }
                            if failed {
                                break;
                            }
                            pending.push((
                                at + Duration::from_secs_f64(slice * 0.8),
                                vec![0x80 | ch, note, 0],
                            ));
                        }
                        if failed {
                            break;
                        }
                    }
                    last_note = Some(note);
                } else if let Some(n) = last_note.take() {
                    let _ = midi.send(&[0x80 | ch, n, 0]);
                }

                let s = if i % 2 == 0 { swing_frac } else { -swing_frac };
                next += Duration::from_secs_f64(tick.as_secs_f64() * (1.0 + s));
                i += 1;
                if i >= pat.length as usize {
                    i = 0;
                    repeats_done += 1;
                    let want = cur!().repeats;
                    if want > 0 && repeats_done >= want {
                        // advance to the next song entry
                        entry_idx += 1;
                        repeats_done = 0;
                        if entry_idx >= entries.len() {
                            if loop_song {
                                entry_idx = 0;
                            } else {
                                break; // song finished
                            }
                        }
                        let _ = events.send(Event::SongPosition {
                            device: midi.id(),
                            entry: entry_idx,
                            repeat: 0,
                            playing: true,
                        });
                    }
                }
            }

            if let Some(n) = last_note {
                let _ = midi.send(&[0x80 | ch, n, 0]);
            }
            for (_, msg) in pending {
                let _ = midi.send(&msg);
            }
            let _ = events.send(Event::Playhead { device: midi.id(), step: 0, playing: false });
            let _ = events.send(Event::SongPosition { device: midi.id(), entry: 0, repeat: 0, playing: false });
            if failed {
                let _ = events.send(Event::Log {
                    device: midi.id(),
                    message: "playback stopped: MIDI send failed (device disconnected?)".into(),
                });
            }
            // mark stopped (even generation) if we still own it
            let _ = generation.compare_exchange(gen, gen + 1, Ordering::SeqCst, Ordering::SeqCst);
        });
    }
}
