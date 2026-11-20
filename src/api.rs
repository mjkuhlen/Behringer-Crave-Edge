//! HTTP + WebSocket API and static web UI serving.
//!
//! Multi-device: every connected device gets a DeviceCtx (own MIDI connection,
//! settings, player, modulation engine). The "selected" device is the editor
//! context for the legacy /api/* routes; /api/d/:id/* routes address any
//! connected device directly, so several devices can play at the same time.

use crate::crave::{self, CraveSettings, Pattern};
use crate::clock::ClockTransport;
use crate::edge::{self, EdgeSettings};
use crate::library::{self, Library};
use crate::midi::{Event, MidiManager};
use crate::modulation::{self, ModConfig, ModEngine};
use crate::player::{Player, SongEntry};
use crate::song::{Song, SongStore};
use anyhow::{anyhow, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Debug, Clone)]
pub enum SettingsModel {
    Crave(CraveSettings),
    Edge(EdgeSettings),
}

impl SettingsModel {
    fn to_json(&self) -> Value {
        match self {
            SettingsModel::Crave(s) => json!(s),
            SettingsModel::Edge(s) => json!(s),
        }
    }
    fn midi_channel(&self) -> u8 {
        match self {
            SettingsModel::Crave(s) => s.midi_channel,
            SettingsModel::Edge(s) => s.midi_channel,
        }
    }
}

/// Everything scoped to one connected device.
pub struct DeviceCtx {
    pub id: String,
    pub kind: String, // crave | edge | sim | other
    pub name: String,
    pub midi: Arc<MidiManager>,
    pub settings: Arc<Mutex<SettingsModel>>,
    pub player: Arc<Player>,
    pub mod_engine: Arc<ModEngine>,
    pub clock: Arc<ClockTransport>,
}

impl DeviceCtx {
    fn is_crave_like(&self) -> bool {
        self.kind == "crave" || self.kind == "sim"
    }
    fn settings_file(&self) -> String {
        format!("settings-{}.json", self.kind)
    }
    fn save_settings(&self, data_dir: &std::path::Path) -> Result<()> {
        let v = self.settings.lock().unwrap().to_json();
        library::save_value(data_dir, &self.settings_file(), &v)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub contexts: Arc<Mutex<HashMap<String, Arc<DeviceCtx>>>>,
    pub selected: Arc<Mutex<Option<String>>>,
    /// unconnected manager used only to enumerate ALSA ports
    pub enumerator: Arc<MidiManager>,
    pub library: Arc<Library>,
    pub songs: Arc<SongStore>,
    pub events: broadcast::Sender<Event>,
    pub data_dir: PathBuf,
}

impl AppState {
    /// The selected device (falls back to the only/first connected one).
    fn cur(&self) -> Result<Arc<DeviceCtx>> {
        let sel = self.selected.lock().unwrap().clone();
        let ctxs = self.contexts.lock().unwrap();
        sel.and_then(|id| ctxs.get(&id).cloned())
            .or_else(|| ctxs.values().next().cloned())
            .ok_or_else(|| anyhow!("no device connected"))
    }

    fn ctx(&self, id: &str) -> Result<Arc<DeviceCtx>> {
        self.contexts
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("device {} is not connected", id))
    }
}

struct ApiError(anyhow::Error);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

/// Static UI files must revalidate so a stale app.js can never leave buttons
/// without handlers after an update.
async fn no_cache_static(req: Request<axum::body::Body>, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api");
    let mut res = next.run(req).await;
    if !is_api {
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    res
}

pub fn router(state: AppState, web_dir: &std::path::Path) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/devices", get(devices))
        .route("/api/connect", post(connect))
        .route("/api/disconnect", post(disconnect))
        .route("/api/select", post(select_device))
        .route("/api/state", get(get_state))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/patterns", get(list_patterns).post(create_pattern))
        .route("/api/patterns/delete-all", post(delete_all_patterns))
        .route("/api/patterns/:id", get(get_pattern).put(update_pattern).delete(delete_pattern))
        .route("/api/patterns/:id/to-device", post(pattern_to_device))
        .route("/api/dump", post(dump))
        .route("/api/backup-all", post(backup_all))
        .route("/api/restore-all", post(restore_all))
        .route("/api/clear-all", post(clear_all))
        .route("/api/sysex", post(raw_sysex))
        .route("/api/play", post(play))
        .route("/api/play-song", post(play_song))
        .route("/api/stop", post(stop))
        .route("/api/note", post(note))
        .route("/api/songs", get(list_songs).post(create_song))
        .route("/api/songs/:id", get(get_song).put(update_song).delete(delete_song))
        .route("/api/modulation", get(get_modulation).put(put_modulation))
        .route("/api/modulation/start", post(mod_start))
        .route("/api/modulation/stop", post(mod_stop))
        .route("/api/modulation/route-assign", post(mod_route_assign))
        // direct per-device control (simultaneous playback on several devices)
        .route("/api/d/:id/state", get(d_state))
        .route("/api/d/:id/play", post(d_play))
        .route("/api/d/:id/play-song", post(d_play_song))
        .route("/api/d/:id/stop", post(d_stop))
        .route("/api/d/:id/note", post(d_note))
        .route("/api/d/:id/modulation/start", post(d_mod_start))
        .route("/api/d/:id/modulation/stop", post(d_mod_stop))
        .route("/api/clock/start", post(clock_start))
        .route("/api/clock/stop", post(clock_stop))
        .route("/api/d/:id/clock/start", post(d_clock_start))
        .route("/api/d/:id/clock/stop", post(d_clock_stop))
        .route("/api/ws", get(ws_handler))
        .fallback_service(
            ServeDir::new(web_dir).not_found_service(ServeFile::new(web_dir.join("index.html"))),
        )
        .layer(CorsLayer::permissive())
        .layer(middleware::from_fn(no_cache_static))
        .with_state(state)
}

async fn health(State(s): State<AppState>) -> Json<Value> {
    let n = s.contexts.lock().unwrap().len();
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "connected": s.cur().ok().map(|d| d.id.clone()),
        "devices_connected": n,
        "playing": s.cur().map(|d| d.player.is_playing()).unwrap_or(false),
    }))
}

async fn devices(State(s): State<AppState>) -> Json<Value> {
    let mut list = s.enumerator.list_devices();
    let ctxs = s.contexts.lock().unwrap();
    for d in &mut list {
        d.connected = ctxs.contains_key(&d.id);
    }
    // contexts whose ALSA client vanished (unplugged) still show as connected
    for (id, ctx) in ctxs.iter() {
        if !list.iter().any(|d| &d.id == id) {
            list.push(crate::midi::DeviceInfo {
                id: id.clone(),
                name: ctx.name.clone(),
                kind: ctx.kind.clone(),
                in_port: None,
                out_port: None,
                connected: true,
            });
        }
    }
    Json(json!({ "devices": list, "selected": *s.selected.lock().unwrap() }))
}

fn load_settings_model(data_dir: &std::path::Path, kind: &str) -> SettingsModel {
    if kind == "edge" {
        let s = library::load_value(data_dir, "settings-edge.json")
            .and_then(|v| serde_json::from_value::<EdgeSettings>(v).ok())
            .unwrap_or_default();
        return SettingsModel::Edge(s);
    }
    // crave / sim / other: CraveSettings; migrate the legacy settings.json
    let file = format!("settings-{}.json", kind);
    let v = library::load_value(data_dir, &file)
        .or_else(|| library::load_value(data_dir, "settings.json"));
    let s = v
        .and_then(|v| serde_json::from_value::<CraveSettings>(v).ok())
        .unwrap_or_default();
    SettingsModel::Crave(s)
}

#[derive(Deserialize)]
struct ConnectReq {
    device_id: String,
}

async fn connect(State(s): State<AppState>, Json(req): Json<ConnectReq>) -> ApiResult<Value> {
    if s.contexts.lock().unwrap().contains_key(&req.device_id) {
        *s.selected.lock().unwrap() = Some(req.device_id.clone());
        let d = s.ctx(&req.device_id)?;
        return Ok(Json(json!({
            "device": {"id": d.id, "name": d.name, "kind": d.kind},
            "settings": d.settings.lock().unwrap().to_json(),
            "already_connected": true,
        })));
    }
    let dev = s
        .enumerator
        .list_devices()
        .into_iter()
        .find(|d| d.id == req.device_id)
        .ok_or_else(|| anyhow!("device {} not found", req.device_id))?;
    let midi = Arc::new(MidiManager::new(s.events.clone()).with_id(&dev.id));
    midi.connect(&req.device_id)?;

    let settings = Arc::new(Mutex::new(load_settings_model(&s.data_dir, &dev.kind)));
    let player = Arc::new(Player::new(midi.clone(), s.events.clone()));
    let mod_engine = Arc::new(ModEngine::new(
        midi.clone(),
        s.events.clone(),
        modulation::load_config(&s.data_dir),
    ));
    let ctx = Arc::new(DeviceCtx {
        id: dev.id.clone(),
        kind: dev.kind.clone(),
        name: dev.name.clone(),
        midi: midi.clone(),
        settings: settings.clone(),
        player,
        mod_engine,
        clock: Arc::new(ClockTransport::new(midi.clone(), s.events.clone())),
    });

    // Probe globals (0x75): read-only. Crave/sim use the Crave frame; Edge the
    // protocol-3 frame. Responses are captured raw for verification.
    let mut globals_raw: Option<String> = None;
    if dev.kind == "edge" {
        midi.clear_responses();
        let _ = midi.send(&edge::frame(edge::CMD_GET_GLOBALS, &[]));
        tokio::time::sleep(Duration::from_millis(900)).await;
        for r in midi.take_responses() {
            if let Some((cmd, data)) = edge::parse(&r) {
                if cmd == edge::CMD_ENTIRE_PARAMS || cmd == edge::CMD_GET_GLOBALS {
                    globals_raw = Some(crave::hex(&r));
                    if let Some(mut gs) = EdgeSettings::from_globals_bytes(&data) {
                        {
                            let mut cur = settings.lock().unwrap();
                            gs.midi_channel = cur.midi_channel();
                            *cur = SettingsModel::Edge(gs);
                        }
                        let _ = ctx.save_settings(&s.data_dir);
                    }
                }
            }
        }
    } else if dev.kind == "crave" || dev.kind == "sim" {
        midi.clear_responses();
        let _ = midi.send(&crave::frame(crave::CMD_GET_GLOBALS, &[]));
        tokio::time::sleep(Duration::from_millis(if midi.is_sim() { 50 } else { 900 })).await;
        for r in midi.take_responses() {
            if let Some((cmd, data)) = crave::parse(&r) {
                if cmd == crave::CMD_GET_GLOBALS {
                    globals_raw = Some(crave::hex(&r));
                    if let Some(mut gs) = CraveSettings::from_globals_bytes(&data) {
                        {
                            let mut cur = settings.lock().unwrap();
                            gs.midi_channel = cur.midi_channel();
                            *cur = SettingsModel::Crave(gs);
                        }
                        let _ = ctx.save_settings(&s.data_dir);
                    }
                }
            }
        }
    }

    s.contexts.lock().unwrap().insert(dev.id.clone(), ctx.clone());
    *s.selected.lock().unwrap() = Some(dev.id.clone());
    Ok(Json(json!({
        "device": dev,
        "settings": settings.lock().unwrap().to_json(),
        "globals_raw": globals_raw,
    })))
}

#[derive(Deserialize)]
struct DisconnectReq {
    device_id: Option<String>,
}

async fn disconnect(State(s): State<AppState>, Json(req): Json<DisconnectReq>) -> ApiResult<Value> {
    let target = match req.device_id {
        Some(id) => id,
        None => s.selected.lock().unwrap().clone().ok_or_else(|| anyhow!("no device connected"))?,
    };
    let ctx = s.contexts.lock().unwrap().remove(&target);
    if let Some(d) = ctx {
        d.player.stop();
        d.mod_engine.stop();
        d.clock.stop();
        d.midi.disconnect();
    }
    let mut sel = s.selected.lock().unwrap();
    if sel.as_deref() == Some(target.as_str()) {
        *sel = s.contexts.lock().unwrap().keys().next().cloned();
    }
    Ok(Json(json!({ "ok": true, "disconnected": target, "selected": *sel })))
}

#[derive(Deserialize)]
struct SelectReq {
    device_id: String,
}

async fn select_device(State(s): State<AppState>, Json(req): Json<SelectReq>) -> ApiResult<Value> {
    let d = s.ctx(&req.device_id)?;
    *s.selected.lock().unwrap() = Some(d.id.clone());
    Ok(Json(json!({ "selected": d.id, "kind": d.kind })))
}

fn ctx_state(d: &DeviceCtx) -> Value {
    json!({
        "id": d.id,
        "name": d.name,
        "kind": d.kind,
        "playing": d.player.is_playing(),
        "mod_running": d.mod_engine.is_running(),
        "clock_running": d.clock.is_running(),
        "settings": d.settings.lock().unwrap().to_json(),
    })
}

async fn get_state(State(s): State<AppState>) -> Json<Value> {
    let ctxs = s.contexts.lock().unwrap();
    let devices: Vec<Value> = ctxs.values().map(|d| ctx_state(d)).collect();
    let selected = s.selected.lock().unwrap().clone();
    let cur = selected
        .as_deref()
        .and_then(|id| ctxs.get(id))
        .or_else(|| ctxs.values().next());
    match cur {
        Some(d) => Json(json!({
            "selected": selected,
            "devices": devices,
            // legacy fields mirror the selected device
            "device_id": d.id,
            "device_name": d.name,
            "is_sim": d.kind == "sim",
            "settings": d.settings.lock().unwrap().to_json(),
            "playing": d.player.is_playing(),
            "mod_running": d.mod_engine.is_running(),
        })),
        None => Json(json!({
            "selected": Value::Null,
            "devices": devices,
            "device_id": Value::Null,
            "device_name": Value::Null,
            "is_sim": false,
            "settings": Value::Null,
            "playing": false,
            "mod_running": false,
        })),
    }
}

async fn d_state(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = s.ctx(&id)?;
    Ok(Json(ctx_state(&d)))
}

async fn get_settings(State(s): State<AppState>) -> ApiResult<Value> {
    let d = s.cur()?;
    let settings = d.settings.lock().unwrap().clone();
    let options = match &settings {
        SettingsModel::Crave(_) => json!({
            "clock_source": crave::CLOCK_SOURCES,
            "clock_type": crave::CLOCK_TYPES,
            "clock_edge": crave::CLOCK_EDGES,
            "assign_mode": crave::ASSIGN_MODES,
        }),
        SettingsModel::Edge(_) => json!({ "key_priority": edge::KEY_PRIORITIES }),
    };
    Ok(Json(json!({
        "kind": d.kind,
        "settings": settings.to_json(),
        "options": options,
    })))
}

async fn put_settings(State(s): State<AppState>, Json(new): Json<Value>) -> ApiResult<Value> {
    let d = s.cur()?;
    let connected = d.midi.connected_id().is_some();
    // parse + validate + diff under the lock, then RELEASE it before awaiting
    // sends (a std MutexGuard held across .await makes the future non-Send)
    let (cmds, new_model) = {
        let guard = d.settings.lock().unwrap();
        match (&*guard, d.kind.as_str()) {
            (SettingsModel::Edge(old), "edge") => {
                let mut new: EdgeSettings =
                    serde_json::from_value(new).map_err(|e| anyhow!("invalid edge settings: {}", e))?;
                new.validate();
                let cmds = new.diff_commands(old);
                (cmds, SettingsModel::Edge(new))
            }
            (SettingsModel::Crave(old), _) => {
                let mut new: CraveSettings =
                    serde_json::from_value(new).map_err(|e| anyhow!("invalid crave settings: {}", e))?;
                new.validate();
                let cmds = new.diff_commands(old);
                (cmds, SettingsModel::Crave(new))
            }
            _ => return Err(anyhow!("settings model does not match device kind").into()),
        }
    };
    let mut applied = Vec::new();
    if connected {
        for (name, bytes) in &cmds {
            d.midi.send(bytes)?;
            applied.push(json!({ "name": name, "hex": crave::hex(bytes) }));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let out_settings = {
        let mut guard = d.settings.lock().unwrap();
        *guard = new_model;
        guard.to_json()
    };
    d.save_settings(&s.data_dir)?;
    Ok(Json(json!({
        "settings": out_settings,
        "applied": applied,
        "device_connected": connected,
        "note": if connected { "changes sent to device" } else { "saved app-side only (no device connected)" },
    })))
}

// ---------------------------------------------------------------------------
// Pattern library (shared across devices; patterns are Crave-format for now)
// ---------------------------------------------------------------------------

async fn list_patterns(State(s): State<AppState>) -> Json<Value> {
    Json(json!({ "patterns": s.library.list() }))
}

async fn create_pattern(State(s): State<AppState>, Json(mut p): Json<Pattern>) -> ApiResult<Pattern> {
    p.normalize();
    if p.id.is_empty() {
        p.id = s.library.new_id(&p.name);
    }
    p.updated = chrono::Utc::now().to_rfc3339();
    s.library.save(&p)?;
    Ok(Json(p))
}

async fn get_pattern(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Pattern> {
    let p = s.library.load(&id).ok_or_else(|| anyhow!("pattern {} not found", id))?;
    Ok(Json(p))
}

async fn update_pattern(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(mut p): Json<Pattern>,
) -> ApiResult<Pattern> {
    p.normalize();
    p.id = id;
    p.updated = chrono::Utc::now().to_rfc3339();
    s.library.save(&p)?;
    Ok(Json(p))
}

async fn delete_pattern(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let deleted = s.library.delete(&id)?;
    Ok(Json(json!({ "deleted": deleted })))
}

#[derive(Deserialize)]
struct DeleteAllReq {
    confirm: bool,
}

async fn delete_all_patterns(State(s): State<AppState>, Json(req): Json<DeleteAllReq>) -> ApiResult<Value> {
    if !req.confirm {
        return Err(anyhow!("deleting the whole library requires confirm=true").into());
    }
    let n = s.library.delete_all()?;
    Ok(Json(json!({ "deleted": n })))
}

// ---------------------------------------------------------------------------
// Crave pattern device I/O (dump / write / backup / restore / clear)
// ---------------------------------------------------------------------------

fn require_crave(d: &DeviceCtx) -> Result<()> {
    if d.is_crave_like() {
        Ok(())
    } else {
        Err(anyhow!("{} does not support pattern SysEx (Crave only for now)", d.name))
    }
}

#[derive(Deserialize)]
struct RestoreAllReq {
    confirm: bool,
}

/// Write every library device-backup (crave-b{B}-p{P}) back to its slot,
/// verified by readback. The inverse of backup-all.
async fn restore_all(State(s): State<AppState>, Json(req): Json<RestoreAllReq>) -> ApiResult<Value> {
    if !req.confirm {
        return Err(anyhow!("restoring all device slots requires confirm=true").into());
    }
    let d = s.cur()?;
    require_crave(&d)?;
    let mut restored = 0u32;
    let mut failed = Vec::new();
    let mut missing = Vec::new();
    for bank in 0..8u8 {
        for pattern in 0..8u8 {
            let prefix = if d.midi.is_sim() { "sim-" } else { "" };
            let id = format!("{}crave-b{}-p{}", prefix, bank + 1, pattern + 1);
            let p = match s.library.load(&id) {
                Some(p) => p,
                None => {
                    missing.push(id);
                    continue;
                }
            };
            // Prefer byte-exact raw replay of the captured dump; fall back to
            // re-encoding the decoded pattern when no raw capture exists.
            let raw_frame = p
                .raw_sysex
                .as_deref()
                .and_then(|h| crave::parse_hex(h).ok())
                .filter(|raw| {
                    crave::parse(raw)
                        .map(|(c, dd)| {
                            c == crave::CMD_SET_SEQ_DATA && dd.len() >= 2 && dd[0] == bank && dd[1] == pattern
                        })
                        .unwrap_or(false)
                });
            let (payload, frame) = match raw_frame {
                Some(raw) => (crave::parse(&raw).unwrap().1, raw),
                None => {
                    let pl = crave::encode_pattern_data(bank, pattern, &p);
                    let fr = crave::frame(crave::CMD_SET_SEQ_DATA, &pl);
                    (pl, fr)
                }
            };
            if let Err(e) = d.midi.send(&frame) {
                failed.push(json!({"bank": bank, "pattern": pattern, "error": e.to_string()}));
                continue;
            }
            tokio::time::sleep(Duration::from_millis(if d.midi.is_sim() { 20 } else { 300 })).await;
            match dump_slot(&d, bank, pattern, 600).await {
                Ok(rb) => {
                    let r = rb.payload.as_deref();
                    if r == Some(payload.as_slice())
                        || r.map(|rb| canonically_equal(&payload, rb, bank, pattern)).unwrap_or(false)
                    {
                        restored += 1;
                    } else {
                        failed.push(json!({"bank": bank, "pattern": pattern, "error": "readback mismatch"}));
                    }
                }
                Err(e) => failed.push(json!({"bank": bank, "pattern": pattern, "error": e.to_string()})),
            }
            let done = bank as u32 * 8 + pattern as u32 + 1;
            if done % 8 == 0 {
                let _ = s.events.send(Event::Log { device: d.midi.id(), message: format!("restore-all: {}/64 slots", done) });
            }
        }
    }
    Ok(Json(json!({ "restored": restored, "failed": failed, "missing": missing })))
}

#[derive(Deserialize)]
struct DumpReq {
    bank: u8,
    pattern: u8,
}

struct SlotDump {
    responses: Vec<String>,
    pattern: Option<Pattern>,
    payload: Option<Vec<u8>>,
}

/// True if a readback payload is semantically identical to the sent payload:
/// decoding it and re-encoding reproduces the sent bytes exactly (the encoder
/// is canonical, so this tolerates firmware normalization of unmodeled bytes).
fn canonically_equal(sent: &[u8], readback: &[u8], bank: u8, pattern: u8) -> bool {
    match crave::decode_pattern_data(readback) {
        Some((_, _, p)) => crave::encode_pattern_data(bank, pattern, &p) == sent,
        None => false,
    }
}

/// Send a 0x77 get and collect the device's answer.
async fn dump_slot(d: &DeviceCtx, bank: u8, pattern: u8, wait_ms: u64) -> Result<SlotDump> {
    let request = crave::frame(crave::CMD_GET_SEQ_DATA, &[bank, pattern]);
    d.midi.clear_responses();
    d.midi.send(&request)?;
    // simulator answers synchronously; hardware needs settling time
    let wait = if d.midi.is_sim() { 20 } else { wait_ms };
    tokio::time::sleep(Duration::from_millis(wait)).await;
    let responses = d.midi.take_responses();
    let mut out = SlotDump { responses: Vec::new(), pattern: None, payload: None };
    for r in &responses {
        out.responses.push(crave::hex(r));
        if let Some((cmd, data)) = crave::parse(r) {
            // real hardware answers a 0x77 get with a 0x78-command frame
            if cmd == crave::CMD_GET_SEQ_DATA || cmd == crave::CMD_SET_SEQ_DATA {
                if out.payload.is_none() {
                    out.payload = Some(data.clone());
                }
                if let Some((_, _, mut p)) = crave::decode_pattern_data(&data) {
                    p.raw_sysex = Some(crave::hex(r));
                    out.pattern = Some(p);
                }
            }
        }
    }
    Ok(out)
}

async fn dump(State(s): State<AppState>, Json(req): Json<DumpReq>) -> ApiResult<Value> {
    let d = s.cur()?;
    require_crave(&d)?;
    if req.bank > 7 || req.pattern > 7 {
        return Err(anyhow!("bank/pattern must be 0..7").into());
    }
    let dd = dump_slot(&d, req.bank, req.pattern, 1500).await?;
    Ok(Json(json!({
        "request_hex": crave::hex(&crave::frame(crave::CMD_GET_SEQ_DATA, &[req.bank, req.pattern])),
        "responses": dd.responses,
        "decoded": dd.pattern,
        "wire_format_experimental": false,
    })))
}

/// Read-only backup of all 64 device slots into the library (stable ids).
async fn backup_all_slots(s: &AppState, d: &DeviceCtx) -> Result<(u32, Vec<Value>)> {
    let mut saved = 0u32;
    let mut failed = Vec::new();
    for bank in 0..8u8 {
        for pattern in 0..8u8 {
            let mut got: Option<Pattern> = None;
            for attempt in 0..2 {
                match dump_slot(d, bank, pattern, 400).await {
                    Ok(dd) if dd.pattern.is_some() => {
                        got = dd.pattern;
                        break;
                    }
                    _ => {
                        if attempt == 0 {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
            }
            match got {
                Some(mut p) => {
                    // namespace simulator backups so they never clobber real
                    // device backups in the shared library
                    let prefix = if d.midi.is_sim() { "sim-" } else { "" };
                    p.id = format!("{}crave-b{}-p{}", prefix, bank + 1, pattern + 1);
                    p.name = format!("{}B{} P{} (device backup)", if d.midi.is_sim() { "Sim " } else { "Crave " }, bank + 1, pattern + 1);
                    if let Some(old) = s.library.load(&p.id) {
                        p.created = old.created;
                    }
                    p.device_bank = Some(bank);
                    p.device_pattern = Some(pattern);
                    p.updated = chrono::Utc::now().to_rfc3339();
                    match s.library.save(&p) {
                        Ok(_) => saved += 1,
                        Err(e) => failed.push(json!({"bank": bank, "pattern": pattern, "error": e.to_string()})),
                    }
                }
                None => failed.push(json!({"bank": bank, "pattern": pattern, "error": "no response"})),
            }
        }
    }
    Ok((saved, failed))
}

async fn backup_all(State(s): State<AppState>) -> ApiResult<Value> {
    let d = s.cur()?;
    require_crave(&d)?;
    let (saved, failed) = backup_all_slots(&s, &d).await?;
    Ok(Json(json!({ "saved": saved, "failed": failed })))
}

#[derive(Deserialize)]
struct ClearAllReq {
    confirm: bool,
    #[serde(default = "default_true")]
    backup_first: bool,
}
fn default_true() -> bool {
    true
}

/// Wipe all 64 device slots to the factory-blank pattern.
/// Safety: backs up every slot first and ABORTS before writing anything if
/// any backup read fails; each write is verified by byte-exact readback.
async fn clear_all(State(s): State<AppState>, Json(req): Json<ClearAllReq>) -> ApiResult<Value> {
    if !req.confirm {
        return Err(anyhow!("clearing all device patterns requires confirm=true").into());
    }
    let d = s.cur()?;
    require_crave(&d)?;
    let mut backed_up = 0u32;
    if req.backup_first {
        let _ = s.events.send(Event::Log { device: d.midi.id(), message: "clear-all: backing up all 64 slots…".into() });
        let (saved, failed) = backup_all_slots(&s, &d).await?;
        if !failed.is_empty() {
            return Err(anyhow!(
                "backup incomplete ({}/64, {} failed) — aborting clear, nothing was written",
                saved, failed.len()
            ).into());
        }
        backed_up = saved;
    }

    // "Cleared" = the device's own factory-blank pattern: one C3 note on
    // step 1 at full gate/vel 64, length 1, steps 2-32 rests (matches the
    // blank slots captured from a factory-state survey).
    let mut empty = Pattern::new_empty("Device default");
    empty.length = 1;
    for st in &mut empty.steps {
        st.rest = true;
        st.note = 48;
        st.gate = 100; // trig 0x30 = full step
        st.velocity = 64;
        st.accent = false;
        st.glide = false;
    }
    empty.steps[0].rest = false;

    let _ = s.events.send(Event::Log { device: d.midi.id(), message: "clear-all: writing empty patterns…".into() });
    let mut cleared = 0u32;
    let mut failed = Vec::new();
    for bank in 0..8u8 {
        for pattern in 0..8u8 {
            // encode per slot: the body carries a bank echo inside the
            // nibble-packed data, so patching the outer header is not enough
            let payload = crave::encode_pattern_data(bank, pattern, &empty);
            let frame = crave::frame(crave::CMD_SET_SEQ_DATA, &payload);
            if let Err(e) = d.midi.send(&frame) {
                failed.push(json!({"bank": bank, "pattern": pattern, "error": e.to_string()}));
                continue;
            }
            tokio::time::sleep(Duration::from_millis(if d.midi.is_sim() { 20 } else { 300 })).await;
            match dump_slot(&d, bank, pattern, 600).await {
                Ok(rb) if rb.payload.as_deref() == Some(payload.as_slice()) => cleared += 1,
                Ok(_) => failed.push(json!({"bank": bank, "pattern": pattern, "error": "readback mismatch"})),
                Err(e) => failed.push(json!({"bank": bank, "pattern": pattern, "error": e.to_string()})),
            }
            let done = bank as u32 * 8 + pattern as u32 + 1;
            if done % 8 == 0 {
                let _ = s.events.send(Event::Log { device: d.midi.id(), message: format!("clear-all: {}/64 slots", done) });
            }
        }
    }
    Ok(Json(json!({ "backed_up": backed_up, "cleared": cleared, "failed": failed })))
}

#[derive(Deserialize)]
struct ToDeviceReq {
    bank: u8,
    pattern: u8,
    confirm: bool,
}

async fn pattern_to_device(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ToDeviceReq>,
) -> ApiResult<Value> {
    if !req.confirm {
        return Err(anyhow!("writing to the device overwrites its pattern slot; set confirm=true").into());
    }
    let d = s.cur()?;
    require_crave(&d)?;
    if req.bank > 7 || req.pattern > 7 {
        return Err(anyhow!("bank/pattern must be 0..7").into());
    }
    let p = s.library.load(&id).ok_or_else(|| anyhow!("pattern {} not found", id))?;
    let payload = crave::encode_pattern_data(req.bank, req.pattern, &p);
    let bytes = crave::frame(crave::CMD_SET_SEQ_DATA, &payload);
    d.midi.send(&bytes)?;
    // verify: read the slot back; accept byte equality or canonical equality
    tokio::time::sleep(Duration::from_millis(if d.midi.is_sim() { 20 } else { 400 })).await;
    let readback = dump_slot(&d, req.bank, req.pattern, 800).await?;
    let rb = readback.payload.as_deref();
    let verified = rb == Some(payload.as_slice());
    let device_normalized = !verified
        && rb.map(|r| canonically_equal(&payload, r, req.bank, req.pattern)).unwrap_or(false);
    // record where this pattern went
    let mut upd = p.clone();
    upd.device_bank = Some(req.bank);
    upd.device_pattern = Some(req.pattern);
    upd.updated = chrono::Utc::now().to_rfc3339();
    let _ = s.library.save(&upd);
    Ok(Json(json!({
        "sent_hex": crave::hex(&bytes),
        "verified": verified,
        "device_normalized": device_normalized,
        "readback": readback.responses,
        "warning": if verified { Value::Null }
            else if device_normalized { json!("device readback differs byte-wise but is semantically identical (firmware normalization)") }
            else { json!("device readback differs from what was sent — do not trust this write") },
    })))
}

#[derive(Deserialize)]
struct SysexReq {
    hex: String,
}

async fn raw_sysex(State(s): State<AppState>, Json(req): Json<SysexReq>) -> ApiResult<Value> {
    let d = s.cur()?;
    let bytes = crave::parse_hex(&req.hex).map_err(|e| anyhow!(e))?;
    if bytes.is_empty() {
        return Err(anyhow!("empty message").into());
    }
    d.midi.send(&bytes)?;
    Ok(Json(json!({ "sent": crave::hex(&bytes) })))
}

// ---------------------------------------------------------------------------
// Playback (selected device + direct per-device routes)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PlayReq {
    pattern: Pattern,
    channel: Option<u8>,
}

fn start_play(d: &DeviceCtx, req: PlayReq) {
    // note playback and the device's own clock-driven sequencer fight each
    // other (on the Edge a running sequencer masks MIDI notes) — stop the
    // clock transport before playing notes on the same device
    d.clock.stop();
    let ch = req.channel.unwrap_or_else(|| d.settings.lock().unwrap().midi_channel());
    d.player.play(req.pattern, ch);
}

async fn play(State(s): State<AppState>, Json(req): Json<PlayReq>) -> ApiResult<Value> {
    let d = s.cur()?;
    start_play(&d, req);
    Ok(Json(json!({ "playing": true, "device": d.id })))
}

async fn d_play(State(s): State<AppState>, Path(id): Path<String>, Json(req): Json<PlayReq>) -> ApiResult<Value> {
    let d = s.ctx(&id)?;
    start_play(&d, req);
    Ok(Json(json!({ "playing": true, "device": d.id })))
}

async fn stop(State(s): State<AppState>) -> ApiResult<Value> {
    let d = s.cur()?;
    d.player.stop();
    Ok(Json(json!({ "playing": false, "device": d.id })))
}

async fn d_stop(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = s.ctx(&id)?;
    d.player.stop();
    Ok(Json(json!({ "playing": false, "device": d.id })))
}

#[derive(Deserialize)]
struct NoteReq {
    note: u8,
    velocity: Option<u8>,
    on: bool,
}

fn send_note(d: &DeviceCtx, req: &NoteReq) -> Result<String> {
    let ch = (d.settings.lock().unwrap().midi_channel().clamp(1, 16) - 1) & 0x0F;
    let msg = if req.on {
        [0x90 | ch, req.note.min(127), req.velocity.unwrap_or(100).clamp(1, 127)]
    } else {
        [0x80 | ch, req.note.min(127), 0]
    };
    d.midi.send(&msg)?;
    Ok(crave::hex(&msg))
}

async fn note(State(s): State<AppState>, Json(req): Json<NoteReq>) -> ApiResult<Value> {
    let d = s.cur()?;
    let sent = send_note(&d, &req)?;
    Ok(Json(json!({ "sent": sent })))
}

async fn d_note(State(s): State<AppState>, Path(id): Path<String>, Json(req): Json<NoteReq>) -> ApiResult<Value> {
    let d = s.ctx(&id)?;
    let sent = send_note(&d, &req)?;
    Ok(Json(json!({ "sent": sent })))
}

// ---------------------------------------------------------------------------
// Song mode
// ---------------------------------------------------------------------------

async fn list_songs(State(s): State<AppState>) -> Json<Value> {
    Json(json!({ "songs": s.songs.list() }))
}

async fn create_song(State(s): State<AppState>, Json(mut song): Json<Song>) -> ApiResult<Value> {
    song.normalize();
    if song.id.is_empty() {
        song.id = s.songs.new_id(&song.name);
    }
    let now = chrono::Utc::now().to_rfc3339();
    if song.created.is_empty() {
        song.created = now.clone();
    }
    song.updated = now;
    s.songs.save(&song)?;
    Ok(Json(json!(song)))
}

async fn get_song(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let song = s.songs.load(&id).ok_or_else(|| anyhow!("song {} not found", id))?;
    Ok(Json(json!(song)))
}

async fn update_song(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(mut song): Json<Song>,
) -> ApiResult<Value> {
    if s.songs.load(&id).is_none() {
        return Err(anyhow!("song {} not found", id).into());
    }
    song.normalize();
    song.id = id;
    song.updated = chrono::Utc::now().to_rfc3339();
    s.songs.save(&song)?;
    Ok(Json(json!(song)))
}

async fn delete_song(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let deleted = s.songs.delete(&id)?;
    Ok(Json(json!({ "deleted": deleted })))
}

#[derive(Deserialize)]
struct PlaySongReq {
    /// play a saved song by id…
    song_id: Option<String>,
    /// …or an inline song body (unsaved edits)
    song: Option<Song>,
    channel: Option<u8>,
}

fn start_song(s: &AppState, d: &DeviceCtx, req: PlaySongReq) -> Result<Value> {
    d.clock.stop(); // notes replace the device's clock-driven sequencer
    let song = match (req.song_id, req.song) {
        (Some(id), _) => s.songs.load(&id).ok_or_else(|| anyhow!("song {} not found", id))?,
        (None, Some(song)) => song,
        _ => return Err(anyhow!("provide song_id or song")),
    };
    if song.entries.is_empty() {
        return Err(anyhow!("song has no entries"));
    }
    let mut entries = Vec::new();
    for e in &song.entries {
        let p = s
            .library
            .load(&e.pattern_id)
            .ok_or_else(|| anyhow!("pattern {} not found (song entry)", e.pattern_id))?;
        entries.push(SongEntry { pattern: p, repeats: e.repeats, transpose: e.transpose });
    }
    let ch = req.channel.unwrap_or_else(|| d.settings.lock().unwrap().midi_channel());
    d.player.play_song(entries, ch, song.loop_song);
    Ok(json!({ "playing": true, "device": d.id, "entries": song.entries.len(), "loop": song.loop_song }))
}

async fn play_song(State(s): State<AppState>, Json(req): Json<PlaySongReq>) -> ApiResult<Value> {
    let d = s.cur()?;
    let out = start_song(&s, &d, req)?;
    Ok(Json(out))
}

async fn d_play_song(State(s): State<AppState>, Path(id): Path<String>, Json(req): Json<PlaySongReq>) -> ApiResult<Value> {
    let d = s.ctx(&id)?;
    let out = start_song(&s, &d, req)?;
    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// Modulation engine (per device)
// ---------------------------------------------------------------------------

async fn get_modulation(State(s): State<AppState>) -> ApiResult<Value> {
    let d = s.cur()?;
    let cfg = d.mod_engine.config.lock().unwrap().clone();
    Ok(Json(json!({ "config": cfg, "running": d.mod_engine.is_running(), "device": d.id })))
}

async fn put_modulation(State(s): State<AppState>, Json(mut cfg): Json<ModConfig>) -> ApiResult<Value> {
    let d = s.cur()?;
    cfg.normalize();
    modulation::save_config(&s.data_dir, &cfg)?;
    *d.mod_engine.config.lock().unwrap() = cfg.clone();
    Ok(Json(json!({ "config": cfg, "running": d.mod_engine.is_running() })))
}

fn start_mod(d: &DeviceCtx) {
    {
        let mut cfg = d.mod_engine.config.lock().unwrap();
        cfg.channel = d.settings.lock().unwrap().midi_channel();
    }
    d.mod_engine.start();
}

async fn mod_start(State(s): State<AppState>) -> ApiResult<Value> {
    let d = s.cur()?;
    start_mod(&d);
    Ok(Json(json!({ "running": true, "device": d.id })))
}

async fn d_mod_start(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = s.ctx(&id)?;
    start_mod(&d);
    Ok(Json(json!({ "running": true, "device": d.id })))
}

async fn mod_stop(State(s): State<AppState>) -> ApiResult<Value> {
    let d = s.cur()?;
    d.mod_engine.stop();
    Ok(Json(json!({ "running": false })))
}

async fn d_mod_stop(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = s.ctx(&id)?;
    d.mod_engine.stop();
    Ok(Json(json!({ "running": false })))
}

#[derive(Deserialize)]
struct RouteAssignReq {
    slot: usize,
}

/// Switch the Crave's ASSIGN output to carry this slot's MIDI source, so the
/// modulation appears at the ASSIGN jack for patching. Only one source feeds
/// ASSIGN at a time. Crave only — the Edge has no ASSIGN output.
async fn mod_route_assign(State(s): State<AppState>, Json(req): Json<RouteAssignReq>) -> ApiResult<Value> {
    let d = s.cur()?;
    require_crave(&d)?;
    let cfg = d.mod_engine.config.lock().unwrap().clone();
    let slot = cfg.slots.get(req.slot).ok_or_else(|| anyhow!("slot {} does not exist", req.slot))?;
    let mode = slot.target.assign_mode().ok_or_else(|| anyhow!("target has no ASSIGN mode"))?;
    let frame = crave::frame(crave::CMD_ASSIGN_MODE, &[mode]);
    d.midi.send(&frame)?;
    {
        let mut st = d.settings.lock().unwrap();
        if let SettingsModel::Crave(cs) = &mut *st {
            cs.assign_mode = mode;
        }
    }
    let _ = d.save_settings(&s.data_dir);
    Ok(Json(json!({
        "assign_mode": mode,
        "mode_name": crave::ASSIGN_MODES[mode as usize],
        "sent": crave::hex(&frame),
    })))
}

// ---------------------------------------------------------------------------
// MIDI clock transport (drives the device's internal sequencer)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ClockStartReq {
    bpm: Option<u16>,
}

fn start_clock(d: &DeviceCtx, bpm: Option<u16>) {
    d.clock.start(bpm.unwrap_or(120));
}

async fn clock_start(State(s): State<AppState>, Json(req): Json<ClockStartReq>) -> ApiResult<Value> {
    let d = s.cur()?;
    start_clock(&d, req.bpm);
    Ok(Json(json!({ "clock_running": true, "device": d.id })))
}

async fn d_clock_start(State(s): State<AppState>, Path(id): Path<String>, Json(req): Json<ClockStartReq>) -> ApiResult<Value> {
    let d = s.ctx(&id)?;
    start_clock(&d, req.bpm);
    Ok(Json(json!({ "clock_running": true, "device": d.id })))
}

async fn clock_stop(State(s): State<AppState>) -> ApiResult<Value> {
    let d = s.cur()?;
    d.clock.stop();
    Ok(Json(json!({ "clock_running": false })))
}

async fn d_clock_stop(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = s.ctx(&id)?;
    d.clock.stop();
    Ok(Json(json!({ "clock_running": false })))
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

async fn ws_handler(ws: WebSocketUpgrade, State(s): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, s))
}

async fn handle_ws(mut socket: WebSocket, s: AppState) {
    let mut rx = s.events.subscribe();
    let hello = json!({
        "type": "hello",
        "selected": *s.selected.lock().unwrap(),
        "devices": s.contexts.lock().unwrap().values().map(|d| ctx_state(d)).collect::<Vec<_>>(),
    });
    if socket.send(Message::Text(hello.to_string())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(e) => {
                    let txt = match serde_json::to_string(&e) { Ok(t) => t, Err(_) => continue };
                    if socket.send(Message::Text(txt)).await.is_err() { break; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(_)) => {} // ignore client messages (ping/pong handled by axum)
                _ => break,
            },
        }
    }
}
