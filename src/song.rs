//! Song mode: ordered chains of library patterns with per-entry repeats and
//! transpose. The Crave has no song memory — songs live in the app and play
//! through the device's MIDI input (stop the device sequencer while a song
//! plays, or both will sound).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SongEntry {
    pub pattern_id: String,
    /// play the pattern this many times before advancing (1..999)
    #[serde(default = "one")]
    pub repeats: u16,
    /// semitones -24..24 applied to every note of the entry
    #[serde(default)]
    pub transpose: i8,
}

fn one() -> u16 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub entries: Vec<SongEntry>,
    /// loop the whole chain when the last entry finishes
    #[serde(default)]
    pub loop_song: bool,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
}

impl Song {
    pub fn normalize(&mut self) {
        self.entries.truncate(64);
        for e in &mut self.entries {
            e.repeats = e.repeats.clamp(1, 999);
            e.transpose = e.transpose.clamp(-24, 24);
        }
    }
}

/// JSON song store — same file conventions as the pattern library.
pub struct SongStore {
    dir: PathBuf,
}

fn slugify(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "song".into()
    } else {
        s.chars().take(40).collect()
    }
}

impl SongStore {
    pub fn new(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("creating song dir {:?}", dir))?;
        Ok(Self { dir })
    }

    fn path(&self, id: &str) -> PathBuf {
        let safe: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        self.dir.join(format!("{}.json", safe))
    }

    pub fn new_id(&self, name: &str) -> String {
        let base = format!("song-{}-{}", slugify(name), chrono::Utc::now().format("%Y%m%d%H%M%S"));
        let mut id = base.clone();
        let mut n = 1;
        while self.path(&id).exists() {
            id = format!("{}-{}", base, n);
            n += 1;
        }
        id
    }

    pub fn list(&self) -> Vec<Song> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "json").unwrap_or(false) {
                    if let Ok(txt) = fs::read_to_string(&p) {
                        if let Ok(song) = serde_json::from_str::<Song>(&txt) {
                            out.push(song);
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| b.updated.cmp(&a.updated));
        out
    }

    pub fn load(&self, id: &str) -> Option<Song> {
        let txt = fs::read_to_string(self.path(id)).ok()?;
        serde_json::from_str(&txt).ok()
    }

    pub fn save(&self, s: &Song) -> Result<()> {
        let txt = serde_json::to_string_pretty(s)?;
        fs::write(self.path(&s.id), txt).with_context(|| format!("writing song {}", s.id))?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let p = self.path(id);
        if p.exists() {
            fs::remove_file(p)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
