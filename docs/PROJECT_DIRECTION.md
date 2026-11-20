# Project review and suggested direction

## Executive summary

This repository already has a substantial foundation: it connects multiple
MIDI devices, edits and verifies Crave pattern dumps, stores songs, sends
app-side MIDI playback and modulation, and includes a Crave simulator. The Edge
is recognized and can receive app playback and MIDI clock, but its own
sequencer is not editable through the app.

The clearest direction is to become a **hands-on companion for building,
remembering, and performing with a Crave and an Edge together**. The instruments
are semi-modular: useful work happens both in software and through patch cables,
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
- **The Edge is deliberately partial:** its settings and MIDI clock are
  supported, but pattern dump/write is Crave-only; the current UI explains
  Edge-specific setup limitations ([edge.rs](../src/edge.rs),
  [index.html](../web/index.html)).
- **A strong test seam, but Crave-centric:** the built-in simulator exercises
  many workflows and the smoke test covers API behavior, while there is no
  corresponding Edge simulator or automated browser test in the current
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

Start with a small set of verified Crave and Edge examples rather than a
generic visual patch editor. Make it easy to capture analog state that cannot
be saved as SysEx. This addresses an important gap between the quickstarts'
hardware-first guidance and the app's current software-first tabs.

**Success checks:** a new user can connect a simulator or device, follow one
verified patch recipe, understand which steps require cables/panel changes,
and save enough information to recreate the patch later.

### 2. Give the Edge a first-class performance workflow

Keep the existing Crave dump/write path intact, but distinguish it clearly
from software-authored patterns that are played over MIDI. The current
Crave-shaped step model and monophonic player are not a good long-term fit for
an Edge-focused rhythm workflow.

Add a device-neutral **performance clip** model with explicit MIDI note or
trigger lanes, per-lane note/velocity/gate, length, and timing. Let users
sequence or trigger the Edge from the app while preserving its panel sequencer
as a separate, clock-driven option. Keep device-specific capabilities and
warnings visible: do not present unsupported Edge SysEx editing as available.

**Success checks:** independently authored clips can drive the Edge over MIDI;
clock mode remains a separate option for playing the Edge's panel sequence;
Crave device dumps still round-trip byte-exactly where supported.

### 3. Combine the two devices into a performable scene

Add a **scene** that references existing library objects rather than copying
them: Crave pattern or song, Edge clip or clock mode, per-device channel,
tempo/transport behavior, and modulation configuration. Include a clear
preflight view for required connections, clock source, MIDI channel, and
physical patching. A scene should make the existing multi-device capability
feel intentional and repeatable.

**Success checks:** one action loads a scene's software state, reports any
missing hardware setup, and starts/stops the intended devices together without
silently changing unrelated device settings.

### 4. Improve confidence before expanding hardware claims

Extend the simulator and smoke coverage to the Edge settings, per-device
selection, clock, and playback paths. Add hardware-validation notes and
fixtures for any newly supported protocol behavior. Keep probing undocumented
Edge sequencer SysEx as an optional research track; only productize it after
repeatable read/write behavior is demonstrated on real units and safely
verified.

For timing-sensitive playback, add measurable tests around clock cadence,
stop behavior, note-off cleanup, and switching/stopping while events are
scheduled. These checks are especially valuable before adding more devices or
more concurrent lanes.

**Success checks:** simulator-based regressions cover both device kinds, and
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
2. Edge-aware performance clips and Edge simulator coverage.
3. Combined scenes and preflight checks.
4. Optional deeper protocol research, broader device support, and remote access
   only when user demand and validation justify them.

This order delivers useful workflow improvements without making the roadmap
depend on reverse-engineering the Edge sequencer.

## Reference material

- [Behringer CRAVE Quick Start Guide (PDF)](https://cdn-media.empowertribe.com/c909b084422e4efd95cf769b47573d5c/QSG_BE_0718-AAJ_CRAVE_WW.pdf)
- [Behringer EDGE Quick Start Guide (PDF)](https://cdn-media.empowertribe.com/eb9d38bc2f1645859332a6c89247fd61/QSG_BE_EDGE_A5_WW.pdf)
- [Behringer EDGE - Answering Your Questions](https://www.youtube.com/watch?v=NoCz0yFXQto)
- [Behringer CRAVE: Review, tutorial and patch ideas (loopop)](https://youtu.be/GU0PQTswBt8)
