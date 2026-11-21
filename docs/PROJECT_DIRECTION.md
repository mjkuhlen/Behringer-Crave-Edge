# Project review and suggested direction

## Executive summary

This repository already has a substantial foundation: it connects multiple
MIDI devices, edits and verifies Crave pattern dumps, stores songs, sends
app-side MIDI playback and modulation, and includes a Crave simulator.

The clearest direction is to become a **hands-on companion for building,
remembering, and performing with a Crave**. The instrument
is semi-modular: useful work happens both in software and through patch cables,
panel controls, and clock configuration. A patch notebook and a device-aware
performance workflow would complement the existing hardware librarian instead
of competing with Behringer's SynthTribe editor.

## Current strengths and boundaries

- **A credible Crave librarian:** pattern data is modeled and round-trip
  verification is built into device writes. The protocol notes distinguish
  hardware-verified pattern behavior from the still-unverified Crave globals
  layout ([README](../README.md), [crave.rs](../src/crave.rs)).
- **A useful shared performance layer:** songs, note playback, MIDI clock,
  live keys, and modulation already work across connected-device contexts
  ([player.rs](../src/player.rs), [clock.rs](../src/clock.rs),
  [modulation.rs](../src/modulation.rs)).
- **A strong test seam:** the built-in simulator exercises
  many workflows and the smoke test covers API behavior, while there is no
  automated browser test in the current
  inventory ([sim.rs](../src/sim.rs), [smoke_test.py](../scripts/smoke_test.py)).
- **The app is a trusted-network tool today:** the server binds to all
  interfaces and the API enables permissive CORS ([main.rs](../src/main.rs),
  [api.rs](../src/api.rs)). Treat that as a deployment constraint before
  recommending exposure beyond a user's own machine or trusted LAN.

## Recommended roadmap

### 1. Make first use and patch recall excellent

Build a guided "make a sound" path using the official quickstarts as the
hardware reference. Pair each software action with the relevant physical
control, switch, jack, or cable. Add a **patch notebook** that stores:

- Device(s), patch name, notes, and optional photos.
- Cable endpoints and a simple patchbay diagram.
- Important knob positions or ranges, clock source, MIDI channel, and any
  software pattern/song references.
- A concise setup/checklist and a "restore this session" view.

Start with a small set of verified Crave examples rather than a
generic visual patch editor. Make it easy to capture analog state that cannot
be saved as SysEx. This addresses an important gap between the quickstarts'
hardware-first guidance and the app's current software-first tabs.

**Success checks:** a new user can connect a simulator or device, follow one
verified patch recipe, understand which steps require cables/panel changes,
and save enough information to recreate the patch later.

### 2. Give software-authored patterns a first-class performance workflow

Keep the existing Crave dump/write path intact, but distinguish it clearly
from software-authored patterns that are played over MIDI (ratchet, swing and
global gate are app-playback only; the Crave stores none of them).

Add a **performance clip** model that can hold what the device cannot store,
so a clip can be played from the app or reduced to a device pattern with a
clear warning about what is lost. Keep device capabilities and warnings
visible.

**Success checks:** clips can be authored, played over MIDI, and reduced to a
device pattern; clock mode remains a separate option for playing the Crave's
own sequence; Crave device dumps still round-trip byte-exactly.

### 3. Combine patterns, songs, and modulation into a performable scene

Add a **scene** that references existing library objects rather than copying
them: Crave pattern or song, channel, tempo/transport behavior, and
modulation configuration. Include a clear
preflight view for required connections, clock source, MIDI channel, and
physical patching. A scene should make playback setup intentional and
repeatable.

**Success checks:** one action loads a scene's software state, reports any
missing hardware setup, and starts/stops playback without
silently changing unrelated device settings.

### 4. Improve confidence before expanding hardware claims

Extend the smoke coverage to per-device
selection, clock, and playback paths. Add hardware-validation notes and
fixtures for any newly supported protocol behavior. Verify the still-unverified
Crave globals layout on hardware before relying on it.

For timing-sensitive playback, add measurable tests around clock cadence,
stop behavior, note-off cleanup, and switching/stopping while events are
scheduled.

**Success checks:** simulator-based regressions cover playback and clock, and
each hardware capability is labeled as documented, verified, or experimental.

### 5. Establish a safe deployment story

Before adding remote-control or multi-user features, define the trust boundary.
Consider binding to localhost by default, making network exposure an explicit
opt-in, and restricting CORS to configured origins. If remote access becomes a
supported use case, add an authentication mechanism appropriate to a
single-user home studio. Document the current behavior and avoid exposing raw
SysEx or destructive library/device operations to an untrusted network.

## Suggested order

1. Patch notebook and guided first-use recipes.
2. Performance clips and playback test coverage.
3. Combined scenes and preflight checks.
4. Optional deeper protocol research, broader device support, and remote access
   only when user demand and validation justify them.

This order delivers useful workflow improvements without depending on
unverified protocol work.

## Reference material

- [Behringer CRAVE Quick Start Guide (PDF)](https://cdn-media.empowertribe.com/c909b084422e4efd95cf769b47573d5c/QSG_BE_0718-AAJ_CRAVE_WW.pdf)
- [Behringer CRAVE: Review, tutorial and patch ideas (loopop)](https://youtu.be/GU0PQTswBt8)
