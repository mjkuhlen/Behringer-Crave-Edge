/* craved web UI — vanilla JS, talks to the daemon REST API + WebSocket */
"use strict";

const $ = (id) => document.getElementById(id);
const NOTE_NAMES = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
const BLACK_OFFSETS = new Set([1,3,6,8,10]);
const KEYMAP = { a:0, w:1, s:2, e:3, d:4, f:5, t:6, g:7, y:8, h:9, u:10, j:11, k:12 };

const CRAVE_OUTPUTS = [
  "MULTIPLE 1", "MULTIPLE 2", "OSC PULSE", "OSC SAW", "ENV", "NOISE",
  "VCA/LINE", "PHONES", "LFO TRI", "LFO SQU", "VC MIX", "ASSIGN",
  "KB CV", "GATE", "VCF",
];
const CRAVE_INPUTS = [
  "OSC CV", "OSC FM", "OSC MOD", "VCF CUTOFF", "VCF RES", "MIX 1",
  "MIX 2", "VC MIX", "MULTIPLE", "MIX CV", "EXT AUDIO", "TEMPO",
  "PLAY/STOP", "RESET", "HOLD", "ENV GATE", "VCA CV", "LFO RATE",
];

function patchSelect(options, value, placeholder, onChange) {
  const select = document.createElement("select");
  const empty = document.createElement("option"); empty.value = ""; empty.textContent = placeholder; select.appendChild(empty);
  for (const option of options) {
    const o = document.createElement("option"); o.value = option; o.textContent = option; select.appendChild(o);
  }
  if (value && !options.includes(value)) {
    const o = document.createElement("option"); o.value = value; o.textContent = value + " (saved)"; select.appendChild(o);
  }
  select.value = value || "";
  select.addEventListener("change", () => onChange(select.value));
  return select;
}

const S = {
  devices: [], connected: null, isSim: false, kind: "crave",
  deviceMap: {},          // id -> {id,name,kind,playing,mod_running,clock_running,settings}
  clockMode: {},          // id -> true: chip transport drives the device sequencer via MIDI clock
  settings: null, options: null,
  pattern: newPattern(), selStep: 0, playing: false, playhead: -1,
  octave: 2, liveVel: 100, pressed: new Set(),
  patch: { schema_version: 1, id: "", name: "New Patch", device_kind: "crave", notes: "", cables: [], controls: [], pattern_ids: [], song_ids: [], checklist: [], is_recipe: false, created: "", updated: "" },
  patches: [],
};

function newPattern() {
  return {
    schema_version: 1, id: "", name: "Untitled", length: 16, tempo: 120, swing: 0, gate_length: 50, ratchet: 1,
    steps: Array.from({length: 32}, () => ({note: 36, rest: true, accent: false, glide: false, gate: 50, velocity: 100, ratchet: 1})),
    created: "", updated: "",
  };
}
const noteName = (n) => NOTE_NAMES[n % 12] + (Math.floor(n / 12) - 1);
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

async function api(path, method = "GET", body = null) {
  const opt = { method, headers: {} };
  if (body !== null) { opt.headers["Content-Type"] = "application/json"; opt.body = JSON.stringify(body); }
  const r = await fetch(path, opt);
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || `${r.status} ${r.statusText}`);
  return j;
}
function status(msg, isErr = false) {
  const el = $("footer-status");
  el.textContent = msg;
  el.style.color = isErr ? "var(--red)" : "var(--muted)";
  if (isErr) console.error(msg);
}

/* In-page modal — replaces native prompt/confirm/alert, which browsers
   silently block in iframes/embedded previews and via dialog settings. */
function showModal({ title, body, confirmWord = null, okLabel = "OK", danger = false }) {
  return new Promise((resolve) => {
    const ov = $("modal-overlay");
    $("modal-title").textContent = title;
    $("modal-body").textContent = body;
    const row = $("modal-input-row");
    const input = $("modal-input");
    const ok = $("modal-ok");
    ok.textContent = okLabel;
    ok.classList.toggle("danger", danger);
    if (confirmWord) {
      row.hidden = false;
      $("modal-word").textContent = confirmWord;
      input.value = "";
      ok.disabled = true;
      input.oninput = () => { ok.disabled = input.value.trim() !== confirmWord; };
    } else {
      row.hidden = true;
      ok.disabled = false;
      input.oninput = null;
    }
    const done = (val) => {
      ov.hidden = true;
      ok.onclick = null;
      $("modal-cancel").onclick = null;
      document.removeEventListener("keydown", onKey);
      resolve(val);
    };
    const onKey = (e) => {
      if (e.key === "Escape") done(false);
      else if (e.key === "Enter" && !ok.disabled) done(true);
    };
    ok.onclick = () => done(true);
    $("modal-cancel").onclick = () => done(false);
    document.addEventListener("keydown", onKey);
    ov.hidden = false;
    if (confirmWord) input.focus(); else ok.focus();
  });
}

/* ---------------- devices (multi-device) ---------------- */
async function refreshDevices() {
  const j = await api("/api/devices");
  S.devices = j.devices;
  const sel = $("device-select");
  const cur = sel.value;
  sel.innerHTML = "";
  for (const d of S.devices) {
    const o = document.createElement("option");
    o.value = d.id;
    o.textContent = `${d.name}${d.kind === "crave" ? " ★" : ""}${d.connected ? " ●" : ""}`;
    sel.appendChild(o);
  }
  if (S.connected && S.devices.some(d => d.id === S.connected)) sel.value = S.connected;
  else if (cur && S.devices.some(d => d.id === cur)) sel.value = cur;
  else {
    const fav = S.devices.find(d => d.kind === "crave");
    if (fav) sel.value = fav.id;
  }
  updateConnUI();
  renderDeviceStrip();
}
function updateConnUI() {
  const st = $("conn-status");
  const btn = $("btn-connect");
  const selId = $("device-select").value;
  const d = S.deviceMap[S.connected];
  if (d) {
    st.textContent = (d.kind === "sim" ? "SIM: " : "") + d.name;
    st.className = "status " + (d.kind === "sim" ? "sim" : "on");
  } else {
    st.textContent = "disconnected";
    st.className = "status off";
  }
  const selConnected = S.devices.find(x => x.id === selId)?.connected;
  btn.textContent = selConnected ? "Select" : "Connect";
}
async function connectToggle() {
  const id = $("device-select").value;
  try {
    const j = await api("/api/connect", "POST", { device_id: id });
    await syncFromState();
    status(`connected: ${j.device.name}` + (j.globals_raw ? " (globals captured)" : ""));
  } catch (e) { status(e.message, true); }
}
async function syncFromState() {
  const st = await api("/api/state");
  S.deviceMap = {};
  for (const d of st.devices || []) S.deviceMap[d.id] = d;
  S.connected = st.device_id || null;
  S.isSim = !!st.is_sim;
  const sel = S.deviceMap[S.connected];
  S.kind = sel ? sel.kind : "crave";
  S.playing = !!st.playing;
  await refreshDevices();
  if (S.connected) await loadSettings().catch(() => {});
}
function renderDeviceStrip() {
  const strip = $("device-strip");
  strip.innerHTML = "";
  const devs = Object.values(S.deviceMap);
  if (!devs.length) { strip.hidden = true; return; }
  strip.hidden = false;
  for (const d of devs) {
    const chip = document.createElement("div");
    chip.className = "dev-chip" + (d.id === S.connected ? " selected" : "") + (d.playing || d.clock_running ? " playing" : "");
    chip.dataset.dev = d.id;
    const label = document.createElement("span");
    label.className = "dev-chip-name";
    label.textContent = `${d.name} (${d.kind})`;
    label.title = "Click to select as editor context";
    label.addEventListener("click", () => selectDevice(d.id));
    const clock = document.createElement("button");
    clock.textContent = "⏱";
    clock.title = "Clock mode: ▶/■ drive the device's OWN sequencer with MIDI clock (set its clock source to USB/MIDI). Off: ▶ plays the app pattern as MIDI notes.";
    clock.classList.toggle("active", !!S.clockMode[d.id]);
    clock.addEventListener("click", (e) => {
      e.stopPropagation();
      S.clockMode[d.id] = !S.clockMode[d.id];
      renderDeviceStrip();
      status(S.clockMode[d.id]
        ? `${d.name}: clock mode — ▶ sends MIDI start + clock at the header BPM`
        : `${d.name}: note mode — ▶ plays the app pattern`);
    });
    const play = document.createElement("button");
    play.textContent = "▶"; play.title = "Play (clock mode: device sequencer; note mode: app pattern)";
    play.addEventListener("click", (e) => { e.stopPropagation(); playOnDevice(d.id); });
    const stop = document.createElement("button");
    stop.textContent = "■"; stop.title = "Stop this device";
    stop.addEventListener("click", (e) => { e.stopPropagation(); stopDevice(d.id); });
    const disc = document.createElement("button");
    disc.textContent = "✕"; disc.title = "Disconnect this device";
    disc.addEventListener("click", (e) => { e.stopPropagation(); disconnectDevice(d.id); });
    chip.append(label, clock, play, stop, disc);
    strip.appendChild(chip);
  }
}
async function selectDevice(id) {
  try {
    await api("/api/select", "POST", { device_id: id });
    await syncFromState();
    status("editor context: " + (S.deviceMap[id]?.name || id));
  } catch (e) { status(e.message, true); }
}
async function disconnectDevice(id) {
  try {
    await api("/api/disconnect", "POST", { device_id: id });
    await syncFromState();
    status("disconnected " + id);
  } catch (e) { status(e.message, true); }
}
async function playOnDevice(id) {
  try {
    if (S.clockMode[id]) {
      const bpm = clamp(parseInt($("tempo").value) || 120, 20, 300);
      await api(`/api/d/${id}/clock/start`, "POST", { bpm });
      status(`MIDI clock → ${S.deviceMap[id]?.name || id} at ${bpm} BPM (device sequencer)`);
    } else {
      await api(`/api/d/${id}/play`, "POST", { pattern: currentPattern() });
      status(`playing current pattern on ${S.deviceMap[id]?.name || id}`);
    }
  } catch (e) { status(e.message, true); }
}
async function stopDevice(id) {
  try {
    if (S.clockMode[id]) await api(`/api/d/${id}/clock/stop`, "POST", {});
    else await api(`/api/d/${id}/stop`, "POST", {});
  } catch (e) { status(e.message, true); }
}

/* ---------------- tabs ---------------- */
document.querySelectorAll(".tab").forEach(t => t.addEventListener("click", () => {
  document.querySelectorAll(".tab").forEach(x => x.classList.toggle("active", x === t));
  document.querySelectorAll(".tab-panel").forEach(p => p.classList.toggle("active", p.id === "tab-" + t.dataset.tab));
  if (t.dataset.tab === "library") refreshLibrary();
  if (t.dataset.tab === "song") { refreshSongs().catch(e => status(e.message, true)); renderSong(); }
  if (t.dataset.tab === "patch") refreshPatches().catch(e => status(e.message, true));
  if (t.dataset.tab === "modulation") loadModulation().catch(e => status(e.message, true));
}));
function showTab(name) { document.querySelector(`.tab[data-tab="${name}"]`).click(); }

/* ---------------- sequencer grid ---------------- */
function renderGrid() {
  const g = $("step-grid");
  g.innerHTML = "";
  for (let i = 0; i < 32; i++) {
    const s = S.pattern.steps[i];
    const cell = document.createElement("div");
    cell.className = "step" + (s.rest ? " rest" : "") + (i >= S.pattern.length ? " inactive" : "")
      + (i === S.selStep ? " selected" : "") + (i === S.playhead && S.playing ? " playhead" : "");
    const ratchet = s.ratchet || 1;
    const flags = (s.rest ? '<span class="r">R</span>' : "") + (s.accent ? '<span class="a">A</span>' : "") + (s.glide ? '<span class="g">G</span>' : "") + (ratchet > 1 ? `<span class="ratchet">×${ratchet}</span>` : "");
    cell.innerHTML = `<div class="num">${i + 1}</div><div class="note">${s.rest ? "—" : noteName(s.note)}</div><div class="flags">${flags}</div>`;
    cell.addEventListener("click", () => selectStep(i));
    g.appendChild(cell);
  }
}
function selectStep(i) {
  S.selStep = i;
  const s = S.pattern.steps[i];
  $("sel-step-num").textContent = i + 1;
  $("step-note").value = s.note;
  $("step-note-name").textContent = noteName(s.note);
  $("step-rest").checked = s.rest;
  $("step-accent").checked = s.accent;
  $("step-glide").checked = s.glide;
  $("step-velocity").value = s.velocity;
  $("step-vel-val").textContent = s.velocity;
  $("step-gate").value = s.gate;
  $("step-gate-val").textContent = s.gate + "%";
  $("step-ratchet").value = s.ratchet || S.pattern.ratchet || 1;
  renderGrid();
  renderMiniKeys();
}
function editStep(fn) { fn(S.pattern.steps[S.selStep]); renderGrid(); }

function renderMiniKeys() {
  const mk = $("mini-keys");
  mk.innerHTML = "";
  const base = S.octave * 12 + 12;
  const cur = S.pattern.steps[S.selStep].note;
  for (let i = 0; i < 13; i++) {
    const b = document.createElement("button");
    b.textContent = NOTE_NAMES[i % 12];
    if (BLACK_OFFSETS.has(i)) b.classList.add("black");
    if (cur === base + i) b.classList.add("hit");
    b.addEventListener("click", () => {
      editStep(s => { s.note = base + i; s.rest = false; });
      $("step-note").value = base + i;
      $("step-note-name").textContent = noteName(base + i);
      $("step-rest").checked = false;
      renderMiniKeys();
    });
    mk.appendChild(b);
  }
}

/* ---------------- pattern controls ---------------- */
function syncPatternControls() {
  $("pattern-name").value = S.pattern.name;
  $("pattern-length").value = S.pattern.length;
  $("tempo").value = S.pattern.tempo;
  $("swing").value = S.pattern.swing;
  $("swing-val").textContent = S.pattern.swing + "%";
  $("gate-length").value = S.pattern.gate_length;
  $("gate-val").textContent = S.pattern.gate_length + "%";
  $("ratchet").value = S.pattern.ratchet || 1;
}
function currentPattern() {
  const p = JSON.parse(JSON.stringify(S.pattern));
  p.name = $("pattern-name").value || "Untitled";
  p.tempo = clamp(parseInt($("tempo").value) || 120, 20, 300);
  p.length = parseInt($("pattern-length").value) || 16;
  p.swing = parseInt($("swing").value) || 0;
  p.gate_length = parseInt($("gate-length").value) || 50;
  p.ratchet = parseInt($("ratchet").value) || 1;
  return p;
}
async function savePattern() {
  try {
    const p = currentPattern();
    const j = p.id ? await api("/api/patterns/" + p.id, "PUT", p) : await api("/api/patterns", "POST", p);
    S.pattern = j;
    syncPatternControls();
    $("save-status").textContent = "saved ✓ " + new Date().toLocaleTimeString();
    status("pattern saved: " + j.id);
    return j;
  } catch (e) { status(e.message, true); return null; }
}

/* ---------------- transport ---------------- */
async function play() {
  try {
    if (S.clockMode[S.connected]) {
      const bpm = clamp(parseInt($("tempo").value) || 120, 20, 300);
      await api("/api/clock/start", "POST", { bpm });
      status(`MIDI clock at ${bpm} BPM → device sequencer`);
    } else {
      await api("/api/play", "POST", { pattern: currentPattern(), channel: S.settings ? S.settings.midi_channel : 1 });
    }
    S.playing = true;
    $("btn-play").classList.add("active");
  } catch (e) { status(e.message, true); }
}
async function stop() {
  try {
    if (S.clockMode[S.connected]) await api("/api/clock/stop", "POST", {});
    else await api("/api/stop", "POST", {});
  } catch (e) { status(e.message, true); }
  S.playing = false; S.playhead = -1;
  $("btn-play").classList.remove("active");
  renderGrid();
}

/* ---------------- device slot dump/write ---------------- */
async function dumpFromDevice() {
  const bank = parseInt($("dump-bank").value), pattern = parseInt($("dump-pattern").value);
  try {
    status(`dumping bank ${bank + 1} pattern ${pattern + 1} from device…`);
    const j = await api("/api/dump", "POST", { bank, pattern });
    if (j.decoded) {
      S.pattern = j.decoded;
      S.pattern.id = ""; // save as new library entry
      syncPatternControls(); selectStep(0);
      const act = S.pattern.steps.filter(s => !s.rest).length;
      status(`read B${bank + 1} P${pattern + 1} from DEVICE: ${S.pattern.length} steps, ${act} active (raw SysEx in monitor)`);
    } else {
      status("dump: no decodable response — raw SysEx in monitor", true);
      await showModal({ title: "Dump failed", body: "No decodable pattern response.\nRaw responses (" + j.responses.length + ") are in the MIDI Monitor.", okLabel: "Close" });
    }
  } catch (e) { status(e.message, true); }
}
async function writeToDevice() {
  const bank = parseInt($("dump-bank").value), pattern = parseInt($("dump-pattern").value);
  // show what is currently in the target slot before overwriting it
  let targetInfo = "(could not read target slot)";
  try {
    const cur = await api("/api/dump", "POST", { bank, pattern });
    if (cur.decoded) {
      const act = cur.decoded.steps.filter(s => !s.rest);
      const first = act.slice(0, 6).map(s => noteName(s.note)).join(" ");
      targetInfo = `${cur.decoded.length} steps, ${act.length} active — starts: ${first || "(all rests)"}`;
    }
  } catch (e) { /* proceed with generic warning */ }
  const step0Rest = currentPattern().steps[0].rest;
  const ok = await showModal({
    title: `Write to device B${bank + 1} P${pattern + 1}`,
    body: `Write the editor pattern to DEVICE bank ${bank + 1} pattern ${pattern + 1}?\n\nTarget currently holds: ${targetInfo}\n\nThis OVERWRITES that device slot. Use "Backup all 64 slots" in the Library tab first if unsure. The write is verified by readback afterwards.` +
      (step0Rest ? "\n\nNOTE: step 1 is a rest in your pattern — the Crave cannot store that and will force it active (C3)." : ""),
    okLabel: "Write to device",
    danger: true,
  });
  if (!ok) { status("write cancelled"); return; }
  try {
    const saved = await savePattern();
    if (!saved) return;
    status(`writing B${bank + 1} P${pattern + 1} and verifying…`);
    const j = await api(`/api/patterns/${saved.id}/to-device`, "POST", { bank, pattern, confirm: true });
    if (j.verified) {
      status(`write to B${bank + 1} P${pattern + 1} VERIFIED — device readback matches byte-for-byte`);
    } else if (j.device_normalized) {
      status(`write to B${bank + 1} P${pattern + 1} verified — device forced step 1 active (Crave can't store a rest there)`);
    } else {
      status(`write to B${bank + 1} P${pattern + 1} NOT verified — readback differs!`, true);
      await showModal({ title: "Write NOT verified", body: "The device readback does NOT match what was sent.\nDo not trust this write. Raw readback is in the MIDI Monitor.", okLabel: "Close", danger: true });
    }
  } catch (e) { status(e.message, true); }
}

/* ---------------- settings ---------------- */
function fillSelect(sel, options, value) {
  sel.innerHTML = "";
  options.forEach((label, i) => {
    const o = document.createElement("option");
    o.value = i; o.textContent = `${i}: ${label}`;
    sel.appendChild(o);
  });
  sel.value = value;
}
function fillSettings() {
  if (!S.settings || !S.options) return;
  const s = S.settings;
  fillSelect($("set-clock-source"), S.options.clock_source, s.clock_source);
  fillSelect($("set-clock-type"), S.options.clock_type, s.clock_type);
  fillSelect($("set-clock-edge"), S.options.clock_edge, s.clock_edge);
  fillSelect($("set-assign-mode"), S.options.assign_mode, s.assign_mode);
  $("set-accent").value = s.accent_threshold;
  $("set-accent-val").textContent = s.accent_threshold;
  $("set-bend-down").value = s.pitch_bend_down;
  $("set-bend-up").value = s.pitch_bend_up;
  $("set-autoplay").checked = !!s.seq_autoplay;
  $("set-clock-out").checked = !!s.midi_clock_out;
  $("set-channel").value = s.midi_channel;
}
async function loadSettings() {
  const j = await api("/api/settings");
  S.kind = j.kind || S.kind;
  S.settings = j.settings; S.options = j.options;
  fillSettings();
}
async function applySettings() {
  const s = {
    clock_source: +$("set-clock-source").value,
    clock_type: +$("set-clock-type").value,
    clock_edge: +$("set-clock-edge").value,
    assign_mode: +$("set-assign-mode").value,
    accent_threshold: +$("set-accent").value,
    pitch_bend_down: +$("set-bend-down").value,
    pitch_bend_up: +$("set-bend-up").value,
    seq_autoplay: $("set-autoplay").checked ? 1 : 0,
    midi_clock_out: $("set-clock-out").checked ? 1 : 0,
    midi_channel: clamp(+$("set-channel").value || 1, 1, 16),
  };
  try {
    const j = await api("/api/settings", "PUT", s);
    S.settings = j.settings;
    $("settings-status").textContent = j.applied.length
      ? `sent ${j.applied.length} SysEx command(s): ` + j.applied.map(a => a.name).join(", ")
      : j.note;
    status("settings applied");
  } catch (e) { status(e.message, true); }
}
async function sendSysex(hex) {
  try { const j = await api("/api/sysex", "POST", { hex }); status("sent: " + j.sent); }
  catch (e) { status(e.message, true); }
}

/* ---------------- library ---------------- */
async function refreshLibrary() {
  try {
    const j = await api("/api/patterns");
    const tb = $("lib-body");
    tb.innerHTML = "";
    if (!j.patterns.length) {
      tb.innerHTML = '<tr><td colspan="6" class="hint">no patterns saved yet — save one from the Sequencer tab</td></tr>';
      return;
    }
    for (const p of j.patterns) {
      const tr = document.createElement("tr");
      const slot = p.device_bank != null ? `B${p.device_bank + 1} P${p.device_pattern + 1}` : "—";
      tr.innerHTML = `<td>${esc(p.name)}</td><td>${p.length}</td><td>${p.tempo}</td>
        <td class="hint">${new Date(p.updated).toLocaleString()}</td><td>${slot}</td><td></td>`;
      const td = tr.lastElementChild;
      const mk = (label, fn, cls) => { const b = document.createElement("button"); b.textContent = label; if (cls) b.className = cls; b.addEventListener("click", fn); td.appendChild(b); };
      mk("Load", () => loadLibraryPattern(p.id));
      mk("Export", () => exportPattern(p));
      mk("Delete", async () => {
        const ok = await showModal({ title: "Delete pattern", body: `Delete "${p.name}" from the library?`, okLabel: "Delete", danger: true });
        if (ok) { await api("/api/patterns/" + p.id, "DELETE"); refreshLibrary(); }
      }, "danger");
      tb.appendChild(tr);
    }
  } catch (e) { status(e.message, true); }
}
const esc = (s) => String(s).replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
async function loadLibraryPattern(id) {
  try {
    S.pattern = await api("/api/patterns/" + id);
    syncPatternControls(); selectStep(0); showTab("sequencer");
    status("loaded: " + S.pattern.name);
  } catch (e) { status(e.message, true); }
}
function exportPattern(p) {
  const blob = new Blob([JSON.stringify(p, null, 2)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = p.id + ".json";
  a.click();
  URL.revokeObjectURL(a.href);
}
async function importPattern(file) {
  try {
    const p = JSON.parse(await file.text());
    p.id = "";
    const j = await api("/api/patterns", "POST", p);
    status("imported: " + j.name);
    refreshLibrary();
  } catch (e) { status("import failed: " + e.message, true); }
}

/* ---------------- live keys ---------------- */
function renderLiveKeys() {
  const lk = $("live-keys");
  lk.innerHTML = "";
  const base = S.octave * 12 + 12;
  $("oct-label").textContent = "C" + S.octave;
  for (let i = 0; i < 13; i++) {
    const b = document.createElement("button");
    b.textContent = NOTE_NAMES[i % 12] + (i === 12 ? S.octave + 1 : S.octave);
    if (BLACK_OFFSETS.has(i)) b.classList.add("black");
    const n = base + i;
    b.dataset.note = n;
    b.addEventListener("mousedown", (e) => { e.preventDefault(); liveOn(n, b); });
    b.addEventListener("mouseup", () => liveOff(n, b));
    b.addEventListener("mouseleave", () => { if (S.pressed.has(n)) liveOff(n, b); });
    lk.appendChild(b);
  }
}
async function liveOn(n, btn) {
  if (S.pressed.has(n)) return;
  S.pressed.add(n);
  if (btn) btn.classList.add("down");
  try { await api("/api/note", "POST", { note: n, velocity: S.liveVel, on: true }); }
  catch (e) { status(e.message, true); }
}
async function liveOff(n, btn) {
  if (!S.pressed.has(n)) return;
  S.pressed.delete(n);
  if (btn) btn.classList.remove("down");
  try { await api("/api/note", "POST", { note: n, velocity: 0, on: false }); }
  catch (e) { /* device may be gone */ }
}
document.addEventListener("keydown", (e) => {
  if (e.repeat || e.target.matches("input,select,textarea")) return;
  const off = KEYMAP[e.key.toLowerCase()];
  if (off === undefined) return;
  const n = S.octave * 12 + 12 + off;
  const btn = document.querySelector(`#live-keys button[data-note="${n}"]`);
  liveOn(n, btn);
});
document.addEventListener("keyup", (e) => {
  const off = KEYMAP[e.key.toLowerCase()];
  if (off === undefined) return;
  const n = S.octave * 12 + 12 + off;
  const btn = document.querySelector(`#live-keys button[data-note="${n}"]`);
  liveOff(n, btn);
});

/* ---------------- song mode ---------------- */
S.song = { schema_version: 1, id: "", name: "New Song", entries: [], loop_song: true, created: "", updated: "" };
S.songs = []; S.songEntry = -1; S.libPatterns = [];

async function refreshSongs() {
  const [sj, pj] = await Promise.all([api("/api/songs"), api("/api/patterns")]);
  S.songs = sj.songs; S.libPatterns = pj.patterns;
  const sel = $("song-load"); const cur = sel.value;
  sel.innerHTML = '<option value="">— load song —</option>';
  for (const s of S.songs) {
    const o = document.createElement("option"); o.value = s.id; o.textContent = s.name; sel.appendChild(o);
  }
  if (cur && S.songs.some(s => s.id === cur)) sel.value = cur;
}
function currentSong() {
  const s = JSON.parse(JSON.stringify(S.song));
  s.name = $("song-name").value || "New Song";
  s.loop_song = $("song-loop").checked;
  return s;
}
function renderSong() {
  $("song-name").value = S.song.name;
  $("song-loop").checked = S.song.loop_song !== false;
  const tb = $("song-body"); tb.innerHTML = "";
  const mkbtn = (txt, fn, title) => {
    const b = document.createElement("button"); b.textContent = txt; b.title = title || "";
    b.addEventListener("click", fn); return b;
  };
  S.song.entries.forEach((e, idx) => {
    const tr = document.createElement("tr");
    if (idx === S.songEntry) tr.className = "song-active";
    const tdN = document.createElement("td"); tdN.textContent = idx + 1;
    const tdP = document.createElement("td");
    const sel = document.createElement("select");
    if (!S.libPatterns.some(p => p.id === e.pattern_id)) {
      const o = document.createElement("option"); o.value = e.pattern_id;
      o.textContent = "(missing) " + e.pattern_id; sel.appendChild(o);
    }
    for (const p of S.libPatterns) {
      const o = document.createElement("option"); o.value = p.id;
      o.textContent = `${p.name} (${p.length} st)`; sel.appendChild(o);
    }
    sel.value = e.pattern_id;
    sel.addEventListener("change", () => { e.pattern_id = sel.value; });
    tdP.appendChild(sel);
    const tdR = document.createElement("td");
    const rep = document.createElement("input");
    rep.type = "number"; rep.min = 1; rep.max = 999; rep.value = e.repeats || 1;
    rep.addEventListener("change", () => { e.repeats = clamp(parseInt(rep.value) || 1, 1, 999); });
    tdR.appendChild(rep);
    const tdT = document.createElement("td");
    const trn = document.createElement("input");
    trn.type = "number"; trn.min = -24; trn.max = 24; trn.value = e.transpose || 0;
    trn.addEventListener("change", () => { e.transpose = clamp(parseInt(trn.value) || 0, -24, 24); });
    tdT.appendChild(trn);
    const tdC = document.createElement("td");
    tdC.appendChild(mkbtn("↑", () => {
      if (idx > 0) { [S.song.entries[idx - 1], S.song.entries[idx]] = [S.song.entries[idx], S.song.entries[idx - 1]]; renderSong(); }
    }, "move up"));
    tdC.appendChild(mkbtn("↓", () => {
      if (idx < S.song.entries.length - 1) { [S.song.entries[idx + 1], S.song.entries[idx]] = [S.song.entries[idx], S.song.entries[idx + 1]]; renderSong(); }
    }, "move down"));
    tdC.appendChild(mkbtn("✕", () => { S.song.entries.splice(idx, 1); renderSong(); }, "remove entry"));
    tr.append(tdN, tdP, tdR, tdT, tdC);
    tb.appendChild(tr);
  });
  if (!S.song.entries.length) {
    const tr = document.createElement("tr"); const td = document.createElement("td");
    td.colSpan = 5; td.className = "hint"; td.textContent = "no entries — add patterns from the library";
    tr.appendChild(td); tb.appendChild(tr);
  }
}
async function saveSong() {
  try {
    const s = currentSong();
    const j = s.id ? await api("/api/songs/" + s.id, "PUT", s) : await api("/api/songs", "POST", s);
    S.song = j;
    $("song-status").textContent = "saved ✓";
    status("song saved: " + j.id);
    await refreshSongs();
    $("song-load").value = j.id;
  } catch (e) { status(e.message, true); }
}
async function playSong() {
  if (!S.connected) { status("connect to the device first", true); return; }
  const s = currentSong();
  if (!s.entries.length) { status("song has no entries", true); return; }
  try {
    await api("/api/play-song", "POST", { song: s, channel: S.settings ? S.settings.midi_channel : 1 });
    status(`song playing: ${s.entries.length} entries${s.loop_song ? ", looping" : ""} — stop the device sequencer!`);
  } catch (e) { status(e.message, true); }
}

/* ---------------- patch notebook ---------------- */
S.patch = { schema_version: 1, id: "", name: "New Patch", device_kind: "crave", notes: "", cables: [], controls: [], pattern_ids: [], song_ids: [], checklist: [], is_recipe: false, created: "", updated: "" };
S.patches = [];

function currentPatch() {
  const p = JSON.parse(JSON.stringify(S.patch));
  p.name = $("patch-name").value || "New Patch";
  p.notes = $("patch-notes").value;
  return p;
}
async function refreshPatches() {
  const j = await api("/api/patches");
  S.patches = j.patches;
  const sel = $("patch-load"); const cur = sel.value;
  sel.innerHTML = '<option value="">— load patch / recipe —</option>';
  for (const p of S.patches) {
    const o = document.createElement("option"); o.value = p.id; o.textContent = (p.is_recipe ? "Recipe: " : "") + p.name; sel.appendChild(o);
  }
  if (cur && S.patches.some(p => p.id === cur)) sel.value = cur;
}
function renderPatch() {
  $("patch-name").value = S.patch.name;
  $("patch-notes").value = S.patch.notes || "";
  const cableWrap = $("patch-cables"); cableWrap.innerHTML = "";
  S.patch.cables.forEach((c, i) => {
    const row = document.createElement("div"); row.className = "patch-row cable-row";
    const from = patchSelect(CRAVE_OUTPUTS, c.from || "", "— Crave output —", v => c.from = v);
    const to = patchSelect(CRAVE_INPUTS, c.to || "", "— Crave input —", v => c.to = v);
    const notes = document.createElement("input"); notes.placeholder = "Cable notes"; notes.value = c.notes || "";
    notes.oninput = () => c.notes = notes.value;
    const rm = document.createElement("button"); rm.textContent = "✕"; rm.onclick = () => { S.patch.cables.splice(i, 1); renderPatch(); };
    row.append(from, document.createTextNode("→"), to, notes, rm); cableWrap.appendChild(row);
  });
  const controlWrap = $("patch-controls"); controlWrap.innerHTML = "";
  S.patch.controls.forEach((c, i) => {
    const row = document.createElement("div"); row.className = "patch-row control-row";
    const control = document.createElement("input"); control.placeholder = "Control / switch"; control.value = c.control || "";
    const value = document.createElement("input"); value.placeholder = "Position / range"; value.value = c.value || "";
    control.oninput = () => c.control = control.value; value.oninput = () => c.value = value.value;
    const rm = document.createElement("button"); rm.textContent = "✕"; rm.onclick = () => { S.patch.controls.splice(i, 1); renderPatch(); };
    row.append(control, value, rm); controlWrap.appendChild(row);
  });
  const checklist = $("patch-checklist"); checklist.innerHTML = "";
  S.patch.checklist.forEach((c, i) => {
    const row = document.createElement("label"); row.className = "patch-check";
    const box = document.createElement("input"); box.type = "checkbox"; box.checked = !!c.done;
    box.onchange = () => c.done = box.checked;
    const text = document.createElement("input"); text.value = c.text || ""; text.oninput = () => c.text = text.value;
    const rm = document.createElement("button"); rm.textContent = "✕"; rm.onclick = () => { S.patch.checklist.splice(i, 1); renderPatch(); };
    row.append(box, text, rm); checklist.appendChild(row);
  });
}
async function savePatch() {
  const p = currentPatch();
  const j = p.id ? await api("/api/patches/" + p.id, "PUT", p) : await api("/api/patches", "POST", p);
  S.patch = j; $("patch-status").textContent = "saved ✓";
  await refreshPatches(); $("patch-load").value = j.id; status("patch saved: " + j.id);
}

/* ---------------- modulation engine ---------------- */
S.mod = null;
const MOD_SHAPES = ["sine", "triangle", "square", "saw", "random"];
const MOD_TARGETS = [
  ["cc1", "CC1 (mod wheel)"], ["cc2", "CC2 (breath)"], ["cc4", "CC4 (foot)"],
  ["cc7", "CC7 (volume)"], ["pitchbend", "Pitch bend"], ["aftertouch", "Channel pressure"],
];
const SYNC_OPTS = [[null, "free Hz"], [0.25, "1/16"], [0.5, "1/8"], [1, "1/4"], [2, "1/2"], [4, "1 bar"], [8, "2 bars"]];

async function loadModulation() {
  const j = await api("/api/modulation");
  S.mod = j.config;
  renderMod(j.running);
}
async function applyMod() {
  try {
    S.mod.tempo = clamp(parseInt($("mod-tempo").value) || 120, 20, 300);
    const j = await api("/api/modulation", "PUT", S.mod);
    S.mod = j.config;
    $("mod-status").textContent = j.running ? "running" : "stopped";
  } catch (e) { status(e.message, true); }
}
function renderMod(running) {
  $("mod-tempo").value = S.mod.tempo;
  $("mod-status").textContent = running ? "running" : "stopped";
  $("btn-mod-start").classList.toggle("active", running);
  const wrap = $("mod-slots"); wrap.innerHTML = "";
  S.mod.slots.forEach((slot, idx) => {
    const card = document.createElement("div");
    card.className = "mod-slot" + (slot.enabled ? " enabled" : "");
    const head = document.createElement("div"); head.className = "mod-slot-head";
    const en = document.createElement("label"); en.className = "toggle";
    const enb = document.createElement("input"); enb.type = "checkbox"; enb.checked = slot.enabled;
    enb.addEventListener("change", () => { slot.enabled = enb.checked; card.classList.toggle("enabled", slot.enabled); applyMod(); });
    en.append(enb, document.createTextNode(` LFO ${idx + 1}`));
    const route = document.createElement("button");
    route.textContent = "→ ASSIGN"; route.title = "Switch the Crave ASSIGN output to this slot's source";
    route.addEventListener("click", async () => {
      try {
        const j = await api("/api/modulation/route-assign", "POST", { slot: idx });
        status(`ASSIGN output → ${j.mode_name} (patch ASSIGN to a CV destination)`);
        if (S.settings) S.settings.assign_mode = j.assign_mode;
      } catch (e) { status(e.message, true); }
    });
    head.append(en, route);
    const row = document.createElement("div"); row.className = "mod-slot-row";
    const mksel = (opts, val, fn) => {
      const s = document.createElement("select");
      for (const [v, lbl] of opts) { const o = document.createElement("option"); o.value = v === null ? "" : v; o.textContent = lbl; s.appendChild(o); }
      s.value = val === null || val === undefined ? "" : val;
      s.addEventListener("change", () => fn(s.value));
      return s;
    };
    const shape = mksel(MOD_SHAPES.map(x => [x, x]), slot.shape, v => { slot.shape = v; applyMod(); });
    const sync = mksel(SYNC_OPTS, slot.sync_beats, v => {
      slot.sync_beats = v === "" ? null : parseFloat(v);
      rate.disabled = slot.sync_beats !== null;
      applyMod();
    });
    const rate = document.createElement("input");
    rate.type = "number"; rate.min = 0.05; rate.max = 20; rate.step = 0.05; rate.value = slot.rate_hz;
    rate.disabled = slot.sync_beats !== null; rate.title = "free rate, Hz";
    rate.addEventListener("change", () => { slot.rate_hz = clamp(parseFloat(rate.value) || 0.5, 0.05, 20); applyMod(); });
    const depth = document.createElement("input");
    depth.type = "range"; depth.min = 0; depth.max = 127; depth.value = slot.depth;
    const depthVal = document.createElement("span"); depthVal.textContent = slot.depth;
    depth.addEventListener("input", () => { slot.depth = +depth.value; depthVal.textContent = depth.value; });
    depth.addEventListener("change", applyMod);
    const center = document.createElement("input");
    center.type = "range"; center.min = 0; center.max = 127; center.value = slot.center;
    const centerVal = document.createElement("span"); centerVal.textContent = slot.center;
    center.addEventListener("input", () => { slot.center = +center.value; centerVal.textContent = center.value; });
    center.addEventListener("change", applyMod);
    const target = mksel(MOD_TARGETS, slot.target, v => { slot.target = v; applyMod(); });
    const lbl = (txt, el) => { const l = document.createElement("label"); l.append(document.createTextNode(txt + " "), el); return l; };
    row.append(lbl("Shape", shape), lbl("Sync", sync), lbl("Hz", rate),
      lbl("Depth", depth), depthVal, lbl("Center", center), centerVal, lbl("Target", target));
    card.append(head, row);
    wrap.appendChild(card);
  });
}

/* ---------------- monitor + websocket ---------------- */
function monLine(text, cls) {
  const log = $("monitor-log");
  const div = document.createElement("div");
  div.className = cls || "";
  div.textContent = `[${new Date().toLocaleTimeString()}] ${text}`;
  log.appendChild(div);
  while (log.childElementCount > 800) log.removeChild(log.firstChild);
  if ($("monitor-autoscroll").checked) log.scrollTop = log.scrollHeight;
}
function updateChipPlaying(id, playing) {
  const chip = document.querySelector(`.dev-chip[data-dev="${id}"]`);
  if (chip) chip.classList.toggle("playing", playing);
}
function connectWS() {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const ws = new WebSocket(`${proto}://${location.host}/api/ws`);
  ws.onmessage = (m) => {
    let e; try { e = JSON.parse(m.data); } catch { return; }
    switch (e.type) {
      case "midi_in": monLine(`IN  [${S.deviceMap[e.device]?.name || e.device || "?"}] ` + e.hex, e.hex.startsWith("F0") ? "in sys" : "in"); break;
      case "midi_out": monLine(`OUT [${S.deviceMap[e.device]?.name || e.device || "?"}] ` + e.hex, e.hex.startsWith("F0") ? "out sys" : "out"); break;
      case "playhead": {
        const d = S.deviceMap[e.device];
        if (d) { d.playing = e.playing; updateChipPlaying(e.device, e.playing); }
        if (e.device === S.connected || !e.device) {
          S.playing = e.playing;
          S.playhead = e.playing ? e.step : -1;
          $("btn-play").classList.toggle("active", e.playing);
          renderGrid();
        }
        break;
      }
      case "song_position":
        if (e.device === S.connected || !e.device) {
          S.songEntry = e.playing ? e.entry : -1;
          $("btn-song-play").classList.toggle("active", e.playing);
          renderSong();
        }
        break;
      case "connected": syncFromState().catch(() => {}); monLine("connected: " + e.name, "sys"); break;
      case "disconnected": syncFromState().catch(() => {}); monLine("disconnected: " + (e.device || ""), "sys"); break;
      case "log": monLine(`[${S.deviceMap[e.device]?.name || e.device || "?"}] ` + e.message, "sys"); break;
      case "hello": break;
    }
  };
  ws.onclose = () => { status("websocket lost — reconnecting…", true); setTimeout(connectWS, 2000); };
}

/* ---------------- init ---------------- */
function init() {
  // selects
  const lenSel = $("pattern-length");
  for (let i = 1; i <= 32; i++) { const o = document.createElement("option"); o.value = i; o.textContent = i; lenSel.appendChild(o); }
  for (const id of ["dump-bank", "dump-pattern"]) {
    const sel = $(id);
    for (let i = 0; i < 8; i++) { const o = document.createElement("option"); o.value = i; o.textContent = i + 1; sel.appendChild(o); }
  }
  // header
  $("btn-connect").addEventListener("click", connectToggle);
  $("btn-refresh-devices").addEventListener("click", refreshDevices);
  $("device-select").addEventListener("change", updateConnUI);
  $("btn-play").addEventListener("click", play);
  $("btn-stop").addEventListener("click", stop);
  // pattern bar
  $("btn-new").addEventListener("click", () => { S.pattern = newPattern(); S.selStep = 0; syncPatternControls(); selectStep(0); $("save-status").textContent = ""; });
  $("btn-save").addEventListener("click", savePattern);
  $("pattern-length").addEventListener("change", () => { S.pattern.length = +$("pattern-length").value; renderGrid(); });
  $("swing").addEventListener("input", () => { S.pattern.swing = +$("swing").value; $("swing-val").textContent = S.pattern.swing + "%"; });
  $("gate-length").addEventListener("input", () => { S.pattern.gate_length = +$("gate-length").value; $("gate-val").textContent = S.pattern.gate_length + "%"; });
  $("tempo").addEventListener("change", () => { S.pattern.tempo = clamp(+$("tempo").value || 120, 20, 300); });
  // step editor
  $("step-note").addEventListener("change", () => {
    const n = clamp(+$("step-note").value || 0, 0, 127);
    editStep(s => s.note = n);
    $("step-note-name").textContent = noteName(n);
    renderMiniKeys();
  });
  $("step-rest").addEventListener("change", () => editStep(s => s.rest = $("step-rest").checked));
  $("step-accent").addEventListener("change", () => editStep(s => s.accent = $("step-accent").checked));
  $("step-glide").addEventListener("change", () => editStep(s => s.glide = $("step-glide").checked));
  $("step-velocity").addEventListener("input", () => { const v = +$("step-velocity").value; $("step-vel-val").textContent = v; editStep(s => s.velocity = v); });
  $("step-gate").addEventListener("input", () => { const v = +$("step-gate").value; $("step-gate-val").textContent = v + "%"; editStep(s => s.gate = v); });
  $("step-ratchet").addEventListener("change", () => editStep(s => s.ratchet = parseInt($("step-ratchet").value) || 1));
  // device slot
  $("btn-dump").addEventListener("click", dumpFromDevice);
  $("btn-write-device").addEventListener("click", writeToDevice);
  // settings
  $("btn-apply-settings").addEventListener("click", applySettings);
  $("set-accent").addEventListener("input", () => $("set-accent-val").textContent = $("set-accent").value);
  $("btn-send-sysex").addEventListener("click", () => sendSysex($("sysex-input").value));
  $("btn-get-globals").addEventListener("click", () => sendSysex("F0 00 20 32 00 01 05 75 F7"));
  $("btn-identity").addEventListener("click", () => sendSysex("F0 7E 7F 06 01 F7"));
  $("btn-clear-all").addEventListener("click", async () => {
    if (!S.connected) { status("connect to the device first", true); return; }
    if (S.isSim) { status("clear-all is for hardware; the simulator resets on reconnect", true); return; }
    const backupFirst = $("clear-backup-first").checked;
    const ok = await showModal({
      title: "Clear all device patterns",
      body: (backupFirst
        ? "This will:\n1. Back up all 64 device slots to the library (aborts if any read fails)\n2. Reset every slot to the device default: one C3 note on step 1, the rest silent\n3. Verify each write by readback afterwards"
        : "This will:\n1. Reset every slot to the device default: one C3 note on step 1, the rest silent\n2. Verify each write by readback\n\nNO BACKUP will be made — patterns are permanently lost unless you already have one.") +
        "\n\nUndo afterwards: Library tab → Restore all to device.",
      confirmWord: "CLEAR",
      okLabel: "Clear device",
      danger: true,
    });
    if (!ok) { status("clear-all cancelled"); return; }
    $("clear-status").textContent = (backupFirst ? "backing up + " : "") + "clearing 64 slots…";
    status("clear-all running — progress in MIDI Monitor");
    try {
      const j = await api("/api/clear-all", "POST", { confirm: true, backup_first: backupFirst });
      const msg = `clear-all: ${j.cleared}/64 cleared` +
        (backupFirst ? `, ${j.backed_up}/64 backed up` : " (no backup)") +
        (j.failed.length ? `, ${j.failed.length} FAILED` : " ✓");
      $("clear-status").textContent = msg;
      status(msg, j.failed.length > 0);
    } catch (e) {
      $("clear-status").textContent = e.message;
      status(e.message, true);
    }
  });
  // library maintenance actions intentionally live elsewhere in the UI
  // live
  $("oct-down").addEventListener("click", () => { S.octave = clamp(S.octave - 1, 0, 7); renderLiveKeys(); renderMiniKeys(); });
  $("oct-up").addEventListener("click", () => { S.octave = clamp(S.octave + 1, 0, 7); renderLiveKeys(); renderMiniKeys(); });
  $("live-vel").addEventListener("input", () => { S.liveVel = +$("live-vel").value; $("live-vel-val").textContent = S.liveVel; });
  // monitor
  $("btn-monitor-clear").addEventListener("click", () => $("monitor-log").innerHTML = "");
  // ratchet (live performance param — app playback; device: SHIFT+GLIDE)
  $("ratchet").addEventListener("change", () => { S.pattern.ratchet = parseInt($("ratchet").value) || 1; });
  // song mode
  $("btn-song-add").addEventListener("click", async () => {
    try {
      if (!S.libPatterns.length) await refreshSongs();
      const first = S.libPatterns[0];
      S.song.entries.push({ pattern_id: first ? first.id : "", repeats: 2, transpose: 0 });
      renderSong();
      if (!first) status("library is empty — save a pattern first", true);
    } catch (e) { status(e.message, true); }
  });
  $("btn-song-save").addEventListener("click", saveSong);
  $("btn-song-play").addEventListener("click", playSong);
  $("song-load").addEventListener("change", async () => {
    const id = $("song-load").value;
    if (!id) return;
    try {
      S.song = await api("/api/songs/" + id);
      S.songEntry = -1;
      renderSong();
      status("song loaded: " + S.song.name);
    } catch (e) { status(e.message, true); }
  });
  $("btn-song-delete").addEventListener("click", async () => {
    if (!S.song.id) { status("song is not saved yet", true); return; }
    const ok = await showModal({ title: "Delete song", body: `Delete song "${S.song.name}"?`, okLabel: "Delete", danger: true });
    if (!ok) return;
    try {
      await api("/api/songs/" + S.song.id, "DELETE");
      S.song = { schema_version: 1, id: "", name: "New Song", entries: [], loop_song: true, created: "", updated: "" };
      renderSong();
      await refreshSongs();
      status("song deleted");
    } catch (e) { status(e.message, true); }
  });
  // patch notebook
  $("btn-patch-save").addEventListener("click", () => savePatch().catch(e => status(e.message, true)));
  $("btn-patch-new").addEventListener("click", () => {
    S.patch = { schema_version: 1, id: "", name: "New Patch", device_kind: "crave", notes: "", cables: [], controls: [], pattern_ids: [], song_ids: [], checklist: [], is_recipe: false, created: "", updated: "" }; renderPatch(); $("patch-load").value = "";
  });
  $("btn-patch-delete").addEventListener("click", async () => {
    if (!S.patch.id) { status("patch is not saved yet", true); return; }
    const ok = await showModal({ title: "Delete patch", body: `Delete patch "${S.patch.name}"?`, okLabel: "Delete", danger: true }); if (!ok) return;
    await api("/api/patches/" + S.patch.id, "DELETE");
    $("btn-patch-new").click(); await refreshPatches(); status("patch deleted");
  });
  $("patch-load").addEventListener("change", async () => {
    const id = $("patch-load").value; if (!id) return;
    S.patch = await api("/api/patches/" + id); renderPatch(); $("patch-status").textContent = S.patch.is_recipe ? "guided recipe" : "saved patch";
  });
  $("btn-patch-add-cable").addEventListener("click", () => { S.patch.cables.push({ from: "", to: "", color: "", notes: "" }); renderPatch(); });
  $("btn-patch-add-control").addEventListener("click", () => { S.patch.controls.push({ control: "", value: "" }); renderPatch(); });
  $("btn-patch-add-check").addEventListener("click", () => { S.patch.checklist.push({ text: "", done: false }); renderPatch(); });
  // modulation
  $("btn-mod-start").addEventListener("click", async () => {
    if (!S.connected) { status("connect to the device first", true); return; }
    try {
      await applyMod();
      await api("/api/modulation/start", "POST", {});
      renderMod(true);
      status("modulation running — hit → ASSIGN on a slot and patch the ASSIGN jack");
    } catch (e) { status(e.message, true); }
  });
  $("btn-mod-stop").addEventListener("click", async () => {
    try { await api("/api/modulation/stop", "POST", {}); renderMod(false); status("modulation stopped"); }
    catch (e) { status(e.message, true); }
  });
  $("mod-tempo").addEventListener("change", applyMod);

  // library
  $("btn-lib-refresh").addEventListener("click", refreshLibrary);
  $("btn-lib-backup").addEventListener("click", async () => {
    if (!S.connected) { status("connect to the device first", true); return; }
    if (S.isSim) { status("backup works on hardware; simulator has no real slots", true); return; }
    const ok = await showModal({ title: "Backup all device slots", body: "Read all 64 device slots into the library (read-only, ~30s)?", okLabel: "Start backup" });
    if (!ok) return;
    status("backing up all 64 device slots…");
    try { const j = await api("/api/backup-all", "POST", {}); status(`device backup: ${j.saved}/64 slots saved` + (j.failed.length ? `, ${j.failed.length} FAILED` : "")); refreshLibrary(); }
    catch (e) { status(e.message, true); }
  });
  $("btn-lib-restore").addEventListener("click", async () => {
    if (!S.connected) { status("connect to the device first", true); return; }
    if (S.isSim) { status("restore-all is for hardware", true); return; }
    const ok = await showModal({ title: "Restore all backups to device", body: "Write all 64 library device-backups back to their device slots?\n\nThis OVERWRITES the current device contents. Each write is verified by readback.", confirmWord: "RESTORE", okLabel: "Restore device", danger: true });
    if (!ok) return;
    status("restore-all running — progress in MIDI Monitor");
    try { const j = await api("/api/restore-all", "POST", { confirm: true }); status(`restore-all: ${j.restored}/64 restored` + (j.failed.length ? `, ${j.failed.length} FAILED` : " ✓")); }
    catch (e) { status(e.message, true); }
  });
  $("btn-lib-import").addEventListener("click", () => $("lib-import-file").click());
  $("lib-import-file").addEventListener("change", (e) => { if (e.target.files[0]) importPattern(e.target.files[0]); e.target.value = ""; });
  $("btn-lib-delete-all").addEventListener("click", async () => {
    let n = 0;
    try { n = (await api("/api/patterns")).patterns.length; } catch (e) { status(e.message, true); return; }
    if (n === 0) { status("library is already empty"); return; }
    const ok = await showModal({ title: "Delete entire library", body: `Delete ALL ${n} patterns from the library? This cannot be undone.`, confirmWord: "DELETE", okLabel: "Delete all", danger: true });
    if (!ok) return;
    try { const j = await api("/api/patterns/delete-all", "POST", { confirm: true }); status(`library: ${j.deleted} patterns deleted`); refreshLibrary(); }
    catch (e) { status(e.message, true); }
  });

  syncPatternControls();
  selectStep(0);
  renderLiveKeys();
  refreshDevices().catch(e => status(e.message, true));
  connectWS();
  api("/api/health").then(h => {
    $("footer-status").textContent = `craved v${h.version}`;
  }).catch(() => {});
  syncFromState().catch(() => {});
}
document.addEventListener("DOMContentLoaded", init);
