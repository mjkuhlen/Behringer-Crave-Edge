//! ALSA MIDI device management via midir.
//!
//! In Docker, pass /dev/snd into the container: the ALSA sequencer is
//! kernel-global, so host USB-MIDI clients (e.g. "BEHRINGER CRAVE") are
//! visible and connectable from inside the container.

use crate::crave;
use crate::sim::SimCrave;
use anyhow::{anyhow, Result};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    MidiIn { device: String, hex: String, bytes: Vec<u8> },
    MidiOut { device: String, hex: String, bytes: Vec<u8> },
    Connected { device: String, name: String },
    Disconnected { device: String },
    Playhead { device: String, step: usize, playing: bool },
    SongPosition { device: String, entry: usize, repeat: u16, playing: bool },
    Log { device: String, message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    /// crave | other | sim
    pub kind: String,
    pub in_port: Option<String>,
    pub out_port: Option<String>,
    pub connected: bool,
}

enum Connection {
    Sim {
        sim: SimCrave,
    },
    Hardware {
        device_id: String,
        conn_out: MidiOutputConnection,
        #[allow(dead_code)]
        conn_in: Option<MidiInputConnection<()>>,
    },
}

pub struct MidiManager {
    inner: Mutex<Option<Connection>>,
    events: broadcast::Sender<Event>,
    responses: Arc<Mutex<Vec<Vec<u8>>>>,
    /// device id carried on every event (one manager per connected device)
    id: String,
}

fn kind_from_name(name: &str) -> String {
    // exact token match: "craved" (our own client) must NOT match "CRAVE"
    let u = name.to_uppercase();
    let hit = |tok: &str| {
        u.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|t| t == tok)
    };
    if hit("CRAVE") {
        "crave".into()
    } else {
        "other".into()
    }
}

impl MidiManager {
    pub fn new(events: broadcast::Sender<Event>) -> Self {
        Self {
            inner: Mutex::new(None),
            events,
            responses: Arc::new(Mutex::new(Vec::new())),
            id: "device".into(),
        }
    }

    /// Set the device id carried on events (builder style, before Arc-wrapping).
    pub fn with_id(mut self, id: &str) -> Self {
        self.id = id.to_string();
        self
    }

    pub fn id(&self) -> String {
        self.id.clone()
    }

    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        let mut map: BTreeMap<String, DeviceInfo> = BTreeMap::new();
        let mut add = |id: String, name: String, dir: &str| {
            let client = id.split(':').next().unwrap_or(&id).to_string();
            if client == "0" {
                return; // System Timer/Announce
            }
            if name.to_lowercase().starts_with("craved") {
                return; // our own ALSA clients/ports, not devices
            }
            let e = map.entry(client.clone()).or_insert(DeviceInfo {
                id: client.clone(),
                name: name.clone(),
                kind: kind_from_name(&name),
                in_port: None,
                out_port: None,
                connected: false,
            });
            if e.name.is_empty() && !name.is_empty() {
                e.name = name.clone();
                e.kind = kind_from_name(&name);
            }
            if dir == "in" {
                e.in_port = Some(id);
            } else {
                e.out_port = Some(id);
            }
        };

        if let Ok(mo) = MidiOutput::new("craved-list") {
            for p in mo.ports() {
                let name = mo.port_name(&p).unwrap_or_default().trim().to_string();
                add(p.id(), name, "out");
            }
        }
        if let Ok(mi) = MidiInput::new("craved-list") {
            for p in mi.ports() {
                let name = mi.port_name(&p).unwrap_or_default().trim().to_string();
                add(p.id(), name, "in");
            }
        }

        let connected_id = self.connected_id();
        let mut out: Vec<DeviceInfo> = map
            .into_values()
            .map(|mut d| {
                d.connected = connected_id.as_deref() == Some(d.id.as_str());
                d
            })
            .collect();
        // numeric client order (string sort would put "128" before "14")
        out.sort_by_key(|d| d.id.parse::<u32>().unwrap_or(u32::MAX));
        out.push(DeviceInfo {
            id: "sim-crave".into(),
            name: "Crave Simulator (built-in)".into(),
            kind: "sim".into(),
            in_port: None,
            out_port: None,
            connected: connected_id.as_deref() == Some("sim-crave"),
        });
        out
    }

    pub fn connected_id(&self) -> Option<String> {
        let g = self.inner.lock().ok()?;
        match g.as_ref()? {
            Connection::Sim { .. } => Some("sim-crave".into()),
            Connection::Hardware { device_id, .. } => Some(device_id.clone()),
        }
    }

    pub fn is_sim(&self) -> bool {
        matches!(&*self.inner.lock().unwrap(), Some(Connection::Sim { .. }))
    }

    pub fn connect(&self, device_id: &str) -> Result<DeviceInfo> {
        self.disconnect();
        if device_id == "sim-crave" {
            *self.inner.lock().unwrap() = Some(Connection::Sim { sim: SimCrave::new() });
            let _ = self.events.send(Event::Connected {
                device: "sim-crave".into(),
                name: "Crave Simulator (built-in)".into(),
            });
            return Ok(self
                .list_devices()
                .into_iter()
                .find(|d| d.id == "sim-crave")
                .unwrap());
        }

        let dev = self
            .list_devices()
            .into_iter()
            .find(|d| d.id == device_id)
            .ok_or_else(|| anyhow!("device {} not found", device_id))?;
        let out_id = dev
            .out_port
            .clone()
            .ok_or_else(|| anyhow!("device {} has no MIDI output port", device_id))?;

        let mo = MidiOutput::new("craved").map_err(|e| anyhow!("MIDI out init: {:?}", e))?;
        let ports = mo.ports();
        let port = ports
            .iter()
            .find(|p| p.id() == out_id)
            .ok_or_else(|| anyhow!("output port {} vanished", out_id))?;
        let conn_out = mo
            .connect(port, "craved-out")
            .map_err(|e| anyhow!("MIDI out connect: {:?}", e))?;

        let mut conn_in = None;
        if let Some(in_id) = &dev.in_port {
            let mut mi =
                MidiInput::new("craved-in").map_err(|e| anyhow!("MIDI in init: {:?}", e))?;
            mi.ignore(Ignore::None);
            let in_ports = mi.ports();
            if let Some(p) = in_ports.iter().find(|p| p.id() == *in_id) {
                let events = self.events.clone();
                let responses = self.responses.clone();
                let dev_id = self.id.clone();
                match mi.connect(
                    p,
                    "craved-in",
                    move |_stamp, msg: &[u8], _data: &mut ()| {
                        let _ = events.send(Event::MidiIn {
                            device: dev_id.clone(),
                            hex: crave::hex(msg),
                            bytes: msg.to_vec(),
                        });
                        if msg.first() == Some(&0xF0) {
                            if let Ok(mut r) = responses.lock() {
                                r.push(msg.to_vec());
                            }
                        }
                    },
                    (),
                ) {
                    Ok(c) => conn_in = Some(c),
                    Err(e) => {
                        let _ = self.events.send(Event::Log {
                            device: self.id.clone(),
                            message: format!("MIDI input connect failed (output still works): {:?}", e),
                        });
                    }
                }
            }
        }

        *self.inner.lock().unwrap() = Some(Connection::Hardware {
            device_id: dev.id.clone(),
            conn_out,
            conn_in,
        });
        let _ = self.events.send(Event::Connected { device: dev.id.clone(), name: dev.name.clone() });
        Ok(dev)
    }

    pub fn disconnect(&self) {
        let had = self.inner.lock().unwrap().take().is_some();
        if had {
            let _ = self.events.send(Event::Disconnected { device: self.id.clone() });
        }
    }

    fn ingest(&self, msg: &[u8]) {
        let _ = self.events.send(Event::MidiIn {
            device: self.id.clone(),
            hex: crave::hex(msg),
            bytes: msg.to_vec(),
        });
        if msg.first() == Some(&0xF0) {
            if let Ok(mut r) = self.responses.lock() {
                r.push(msg.to_vec());
            }
        }
    }

    pub fn send(&self, msg: &[u8]) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        let conn = g.as_mut().ok_or_else(|| anyhow!("no device connected"))?;
        let _ = self.events.send(Event::MidiOut {
            device: self.id.clone(),
            hex: crave::hex(msg),
            bytes: msg.to_vec(),
        });
        match conn {
            Connection::Sim { sim } => {
                if let Some(resp) = sim.handle(msg) {
                    self.ingest(&resp);
                }
                Ok(())
            }
            Connection::Hardware { conn_out, .. } => {
                conn_out.send(msg).map_err(|e| anyhow!("MIDI send failed: {:?}", e))
            }
        }
    }

    pub fn clear_responses(&self) {
        self.responses.lock().unwrap().clear();
    }

    pub fn take_responses(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.responses.lock().unwrap())
    }
}
