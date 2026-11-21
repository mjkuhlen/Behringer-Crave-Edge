//! JSON pattern library on disk (mounted volume in Docker).

use crate::crave::Pattern;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Library {
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
        "pattern".into()
    } else {
        s.chars().take(40).collect()
    }
}

impl Library {
    pub fn new(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("creating library dir {:?}", dir))?;
        Ok(Self { dir })
    }

    fn path(&self, id: &str) -> PathBuf {
        // ids are generated slugs; reject anything path-like
        let safe: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        self.dir.join(format!("{}.json", safe))
    }

    pub fn new_id(&self, name: &str) -> String {
        let base = format!("{}-{}", slugify(name), chrono::Utc::now().format("%Y%m%d%H%M%S"));
        let mut id = base.clone();
        let mut n = 1;
        while self.path(&id).exists() {
            id = format!("{}-{}", base, n);
            n += 1;
        }
        id
    }

    pub fn list(&self) -> Vec<Pattern> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "json").unwrap_or(false) {
                    if let Ok(txt) = fs::read_to_string(&p) {
                        if let Ok(pat) = serde_json::from_str::<Pattern>(&txt) {
                            out.push(pat);
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| b.updated.cmp(&a.updated));
        out
    }

    pub fn load(&self, id: &str) -> Option<Pattern> {
        let txt = fs::read_to_string(self.path(id)).ok()?;
        serde_json::from_str(&txt).ok()
    }

    pub fn save(&self, p: &Pattern) -> Result<()> {
        let txt = serde_json::to_string_pretty(p)?;
        fs::write(self.path(&p.id), txt).with_context(|| format!("writing pattern {}", p.id))?;
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

    pub fn delete_all(&self) -> Result<u32> {
        let mut n = 0;
        for e in fs::read_dir(&self.dir)?.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "json").unwrap_or(false) {
                fs::remove_file(&p)?;
                n += 1;
            }
        }
        Ok(n)
    }
}

/// Generic JSON value persistence (per-device settings files etc.)
pub fn load_value(data_dir: &Path, file: &str) -> Option<serde_json::Value> {
    fs::read_to_string(data_dir.join(file))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

pub fn save_value(data_dir: &Path, file: &str, v: &serde_json::Value) -> Result<()> {
    fs::create_dir_all(data_dir)?;
    fs::write(data_dir.join(file), serde_json::to_string_pretty(v)?)?;
    Ok(())
}
