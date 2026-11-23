# craved — Behringer Crave companion for Linux

A Dockerized companion daemon + web UI for the Behringer Crave analog
semi-modular synth. Fills the gap left by Behringer's Windows/macOS-only
SynthTribe app, and goes further: pattern librarian, visual sequencer,
device settings editor, and a modulation/playback workstation.

**Status: hardware-verified, multi-device.** Crave pattern read/write confirmed
on a real unit (nibble-packed wire format; see *Protocol* below). Multiple
devices can be connected and played **at the same time** — each gets its own MIDI connection,
settings, player and modulation engine; a device strip in the UI selects the
editor context and offers per-device transport.

## Quick start (Docker)

```bash
docker compose up -d --build
# UI + API: http://localhost:8090
```

- Plug the Crave in via USB **before or after** starting the container —
  hit ⟳ in the device bar to rescan ALSA ports. It shows up as
  `BEHRINGER CRAVE` (class-compliant USB MIDI, no drivers).
- No hardware? Connect to **Crave Simulator (built-in)** — a full in-process
  simulation of the documented SysEx command set, so every feature can be
  exercised without a device.
- Patterns and settings persist in `./data` (mounted at `/data`).

### How MIDI works in the container

The ALSA sequencer is kernel-global. `docker-compose.yml` passes
`/dev/snd` into the container, which makes all host MIDI clients (USB
devices, PipeWire bridges, other apps) visible and connectable from inside.
No `--privileged`, no host networking needed.

## Features

| Area | What it does |
|---|---|
| Device manager | Enumerates ALSA MIDI ports, auto-flags Crave, **multiple simultaneous connections**, per-device transport chips, built-in simulator |
| Sequencer | 32-step grid (4×8), per-step note/rest/accent/glide/velocity/gate/ratchet, length 1–32, tempo, swing and global gate |
| Playback | Monophonic note playback over MIDI with swing + gate scheduling + ratchet retriggers, live playhead in the UI. **Clock mode** (⏱ on the device chip): ▶/■ send MIDI start/stop + 24 PPQN clock at the header BPM to drive the device's OWN sequencer (set its clock source to USB/MIDI) |
| Song mode | Chain library patterns with per-entry repeats (1–999) and transpose (±24 st), loop, live entry highlight. App-side playback (the Crave has no song memory) — stop the device sequencer while a song plays |
| Modulation engine | Two app-side LFO slots (sine/triangle/square/saw/random S&H), free Hz or tempo-synced (1/16…2 bars), depth + center, targets CC1/2/4/7, pitch bend, channel pressure. "→ ASSIGN" switches the Crave's ASSIGN output to the slot's source (0x1F) so it appears at the ASSIGN jack for CV patching |
| Librarian | Save/load/import/export patterns as JSON; raw device SysEx preserved with dumps; songs stored separately |
| Device settings | Crave: clock source/type/edge, ASSIGN mode (16 options), accent threshold, pitch bend range, seq auto-play, MIDI clock out | Dump/restore | `0x77` get / `0x78` set sequencer data, bank 1–8 × pattern 1–8 |
| Live keys | 13-key keyboard (mouse + computer keys a–k), octave shift, velocity |
| MIDI monitor | Every IN/OUT message incl. SysEx, over WebSocket |
| Raw SysEx | Send arbitrary frames; identity request + globals request buttons |

## Architecture

```
┌─────────────────────────── Docker container ───────────────────────────┐
│ craved (Rust)                                                          │
│  ├─ axum HTTP + WebSocket ── serves web/ (vanilla JS UI)               │
│  ├─ midi.rs   midir/ALSA: enumerate, connect, send/receive             │
│  ├─ crave.rs  Crave SysEx protocol, settings, pattern encode/decode      │
│  ├─ sim.rs    built-in Crave simulator (same protocol)                 │
│  ├─ player.rs tempo/swing/gate/ratchet playback + song chaining        │
│  ├─ song.rs   song model + JSON store (/data/songs)                    │
│  ├─ modulation.rs LFO engine → MIDI CC/bend/pressure + ASSIGN routing  │
│  └─ library.rs JSON persistence (/data volume)                         │
└───────────────/dev/snd────────────────────────────────────────────────┘
        ↕ ALSA sequencer (kernel-global)
   Host USB MIDI: BEHRINGER CRAVE / interfaces
```

## Protocol honesty

Documented in the Crave manual (`F0 00 20 32 00 01 05 <cmd> <data> F7`):

| Cmd | Function |
|---|---|
| `0x11` | pitch bend range (2 bytes) |
| `0x17` | MIDI clock out |
| `0x19` / `0x1A` / `0x1B` | clock edge / type / source (INT, MIDI, USB, EXT, AUTO) |
| `0x1C` | accent threshold (0–0x7E) |
| `0x1D` | sequencer auto-play |
| `0x1F` | ASSIGN output mode (16 modes incl. MIDI CC1/2/4/7, velocity, seq clock/ramp/random…) |
| `0x75` / `0x76` | get / set globals (9 bytes) |
| `0x77` / `0x78` | get / set sequencer data (bank, pattern, …) |
| `0x7D` | factory reset (not exposed in UI; raw SysEx only) |

**Pattern wire format — VERIFIED ON HARDWARE** (2026-11-19: writes commit
immediately, readback byte-exact, playback confirmed by ear; cross-checked
against [SynthHUB](https://github.com/utajum/SynthHUB)'s MIT-licensed,
HW-verified type-0 mono codec; unit-tested against captured dumps in
`tests/fixtures/`):

- The device answers a `0x77` get with a **`0x78`-command frame**.
- Payload (264 bytes): `[bank, pattern, nibble-packed body]`. **Nibble
  packing** is the crux: every body byte travels as two bytes, hi nibble
  first (`0x3C` → `03 0C`). Sending raw bytes gets nibble-unpacked by the
  firmware into garbage — the failure mode that makes every pattern
  collapse to a 1-step blip.
- Body (131 bytes): `[bank_echo, len_hi, len_lo, 32×(note, trig, vel, flags)]`
  — `length = len_hi*8 + len_lo + 1` (1–32). Tempo/swing/gate are **global
  knobs**, not stored per pattern.
- `note` = plain MIDI note number 0–127 (identity — no octave offset).
- `trig` = gate time in 1/48-step units: `0x30` = full step, `0x54`–`0x7E` =
  held into the next step (slide/legato), `≥0x7F` = virgin filler (plays as
  full gate), `0` = default gate.
- `vel` = MIDI velocity 1–127 (`0xFF` virgin → plays as 64). **Accent is
  velocity ≥ 112** — there is no separate accent bit.
- `flags`: bit 7 = REST; bits 0–6 have no audible effect (preserved).
- The Crave stores no ratchet/probability in its pattern dumps. Ratchet is app/PWA playback state, available per step in this app; it is retriggered by MIDI note-on/off playback and is not written to or returned by the Crave pattern format.
- **There is no save/commit handshake** — a `0x78` write commits straight to
  pattern memory, playing or stopped, loaded or not.
- Steps beyond the active length keep stale data (not rests).
- `clear-all` writes the factory-blank shape: one C3 note on step 1 at full
  gate/vel 64, length 1, steps 2–32 rests.

Device writes are confirm-gated and verified by readback (byte equality, or
canonical re-encode equality when the firmware normalizes unmodeled bytes);
`POST /api/backup-all` snapshots all 64 slots, and `restore-all` prefers
byte-exact raw replay of the captured frames.

The globals layout of `0x75`/`0x76` remains unverified — the test unit did not
answer `0x75`; individual documented setting commands (`0x11`…`0x1F`) are the
supported path.

**Multi-device API**

`POST /api/connect` adds a device (does not replace existing connections);
`POST /api/select {device_id}` switches the editor context;
`POST /api/disconnect {device_id?}` drops one (or the selected) device.
Legacy `/api/*` routes operate on the selected device; `/api/d/:id/*`
(`state`, `play`, `play-song`, `stop`, `note`, `modulation/start|stop`)
address any connected device directly for simultaneous playback.
Settings persist per device kind (`settings-crave.json`).

## API sketch

```
GET  /api/health | /api/devices | /api/state | /api/settings
POST /api/connect {device_id} | /api/disconnect
PUT  /api/settings {…}                  → sends only changed params as SysEx
GET/POST /api/patterns                  → library
GET/PUT/DELETE /api/patterns/:id
POST /api/patterns/:id/to-device {bank, pattern, confirm:true}
                                        → writes, then reads back and verifies byte equality
POST /api/dump {bank, pattern}          → raw + decoded
POST /api/backup-all                    → read-only dump of all 64 slots into the library
POST /api/restore-all {confirm:true}    → write all 64 library backups back to their device
                                          slots, each verified by readback
POST /api/clear-all {confirm:true, backup_first?:true}
                                        → reset all 64 slots to the device default (one C3
                                          note on step 1), each write verified by readback;
                                          backup-first is optional (default on) and aborts
                                          the clear if any read fails
POST /api/sysex {hex}
POST /api/play {pattern, channel?}      → app-side playback (ratchet/swing/gate)
POST /api/play-song {song_id|song, channel?} → chain playback
GET/POST /api/songs, GET/PUT/DELETE /api/songs/:id
GET/PUT /api/modulation                 → LFO slot config (persisted)
POST /api/modulation/start | /stop
POST /api/modulation/route-assign {slot} → set ASSIGN mode to slot's source
POST /api/clock/start {bpm?} | /api/clock/stop   → MIDI clock transport (selected device)
POST /api/d/:id/clock/start {bpm?} | /api/d/:id/clock/stop → per-device clock
POST /api/stop | /api/note {note, velocity, on}
GET  /api/ws                            → events: midi_in/out, playhead, song_position, connected, log
```

## Development

```bash
# rebuild after code changes
docker compose up -d --build
# logs
docker compose logs -f craved

# fast compile-check loop (this host has no buildx; classic builder):
docker build -f Dockerfile.dev -t craved-dev .
docker run --rm -v "$PWD:/build" -w /build \
  -v craved-cargo:/usr/local/cargo/registry craved-dev cargo check

# end-to-end smoke test against the simulator (23 checks)
python3 scripts/smoke_test.py

# run without Docker (needs rust + libasound2-dev)
cargo run -- --port 8090 --data-dir ./data --web-dir ./web
```

## Roadmap

1. **Hardware verification** of pattern SysEx layout (dump known patterns, diff).
2. **Modulation engine**: software LFOs/envelopes/step modulators → CC1 →
   ASSIGN jack → VCF CUTOFF / OSC FM / LFO RATE (ASSIGN routing set via `0x1F`).
3. **Patch documentation**: interactive patchbay diagram (18 in / 15 out),
   knob-position recipes, photos — the analog state the Crave can't save.
4. MIDI clock master/slave tied to PipeWire/JACK transport; poly-chain
   voice allocation across multiple Craves.
