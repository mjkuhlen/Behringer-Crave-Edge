//! Persistent patch notebooks and guided hardware setup recipes.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

fn default_schema() -> u32 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatchCable {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatchControl {
    pub control: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatchCheck {
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Patch {
    #[serde(default = "default_schema")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub device_kind: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub cables: Vec<PatchCable>,
    #[serde(default)]
    pub controls: Vec<PatchControl>,
    #[serde(default)]
    pub pattern_ids: Vec<String>,
    #[serde(default)]
    pub song_ids: Vec<String>,
    #[serde(default)]
    pub checklist: Vec<PatchCheck>,
    #[serde(default)]
    pub is_recipe: bool,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
}

impl Patch {
    pub fn normalize(&mut self) {
        self.schema_version = 1;
        self.cables.truncate(64);
        self.controls.truncate(64);
        self.pattern_ids.truncate(64);
        self.song_ids.truncate(64);
        self.checklist.truncate(64);
    }
}

fn slugify(name: &str) -> String {
    let s: String = name.to_lowercase().chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "patch".into() } else { s.chars().take(40).collect() }
}

pub struct PatchStore {
    dir: PathBuf,
}

impl PatchStore {
    pub fn new(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("creating patch dir {:?}", dir))?;
        let store = Self { dir };
        store.seed_recipes()?;
        Ok(store)
    }

    fn path(&self, id: &str) -> PathBuf {
        let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        self.dir.join(format!("{}.json", safe))
    }

    pub fn new_id(&self, name: &str) -> String {
        let base = format!("patch-{}-{}", slugify(name), chrono::Utc::now().format("%Y%m%d%H%M%S"));
        let mut id = base.clone();
        let mut n = 1;
        while self.path(&id).exists() { id = format!("{}-{}", base, n); n += 1; }
        id
    }

    pub fn list(&self) -> Vec<Patch> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "json").unwrap_or(false) {
                    if let Ok(txt) = fs::read_to_string(&p) {
                        if let Ok(patch) = serde_json::from_str::<Patch>(&txt) { out.push(patch); }
                    }
                }
            }
        }
        out.sort_by(|a, b| b.updated.cmp(&a.updated));
        out
    }

    pub fn load(&self, id: &str) -> Option<Patch> {
        serde_json::from_str(&fs::read_to_string(self.path(id)).ok()?).ok()
    }

    pub fn save(&self, patch: &Patch) -> Result<()> {
        fs::write(self.path(&patch.id), serde_json::to_string_pretty(patch)?)
            .with_context(|| format!("writing patch {}", patch.id))?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let p = self.path(id);
        if p.exists() { fs::remove_file(p)?; Ok(true) } else { Ok(false) }
    }

    fn seed_recipes(&self) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let recipe = Patch {
            schema_version: 1,
            id: "recipe-crave-first-sequenced-note".into(),
            name: "Crave — first sequenced note".into(),
            device_kind: "crave".into(),
            notes: "Start by making a simple, audible Crave sequence without patching. Only add external cables after the internal voice is confirmed. The Crave's own sequencer and app playback are two separate modes: use the device sequencer with MIDI clock, or stop it and play notes from this app.".into(),
            cables: vec![],
            controls: vec![PatchControl { control: "Clock source".into(), value: "USB/MIDI only when clocking the Crave's own sequencer".into() },
                           PatchControl { control: "MIDI channel".into(), value: "Match the rear-panel DIP switches and Device Settings".into() },
                           PatchControl { control: "Master volume".into(), value: "Low to start, then raise gradually".into() },
                           PatchControl { control: "VCO tune / range".into(), value: "Set a comfortable in-tune note before recording steps".into() },
                           PatchControl { control: "Filter cutoff".into(), value: "Open enough to hear the oscillator".into() },
                           PatchControl { control: "Filter resonance".into(), value: "Start low to avoid harsh peaks".into() },
                           PatchControl { control: "Envelope sustain / release".into(), value: "Set Sustain and Decay for a clear first note; adjust Release to taste".into() }],
            pattern_ids: vec![], song_ids: vec![],
            checklist: vec![PatchCheck { text: "Connect the Crave over USB and verify it appears in the Device bar".into(), done: false },
                            PatchCheck { text: "Set Master Volume low, open Filter Cutoff, and lower Resonance".into(), done: false },
                            PatchCheck { text: "Set the VCO tune/range and confirm that the analog voice is audible before sequencing".into(), done: false },
                            PatchCheck { text: "Set App MIDI channel to match the rear-panel DIP switches".into(), done: false },
                            PatchCheck { text: "For app playback, stop the Crave sequencer and press the app Play control".into(), done: false },
                            PatchCheck { text: "For Clock mode, set Clock Source to USB/MIDI and use the device clock transport".into(), done: false }],
            is_recipe: true,
            created: now.clone(), updated: now,
        };
        match self.load(&recipe.id) {
            Some(existing) if existing.is_recipe && (existing.notes.starts_with("Starter hardware-and-software setup") || existing.cables.iter().any(|c| c.from == "No external cable required")) => self.save(&recipe),
            Some(_) => Ok(()),
            None => self.save(&recipe),
        }
    }
}
