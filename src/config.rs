use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AudioQuality {
    Best,
    High,
    Medium,
    Low,
}

impl AudioQuality {
    pub fn to_format_str(&self) -> &str {
        match self {
            AudioQuality::Best => "bestaudio",
            AudioQuality::High => "bestaudio[abr>=192]/bestaudio",
            AudioQuality::Medium => "bestaudio[abr<=192][abr>=96]/bestaudio",
            AudioQuality::Low => "bestaudio[abr<=96]/bestaudio",
        }
    }
}

impl Default for AudioQuality {
    fn default() -> Self {
        AudioQuality::Best
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub default_speed: f32,
    pub default_volume: u8,
    #[serde(default)]
    pub audio_quality: AudioQuality,
    pub active_playlist: Option<String>,
    /// Extra mpv options for *video* playback only — window management, mostly.
    ///
    /// Empty by default and deliberately so: mpv exits on an option it does not
    /// recognise, so shipping `--focus-on=never` (mpv 0.38 and up) as a default
    /// would stop playback dead on every older install. The README recommends it
    /// and the user opts in. See `player::mpv_args`.
    #[serde(default)]
    pub video_mpv_args: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_speed: 1.0,
            default_volume: 80,
            audio_quality: AudioQuality::Best,
            active_playlist: None,
            video_mpv_args: Vec::new(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("trovers")
            .join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            let config = Config::default();
            config.save().context("failed to write default config")?;
            return Ok(config);
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        toml::from_str(&raw).context("failed to parse config.toml")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("failed to create config directory")?;
        }
        let raw = toml::to_string(self).context("failed to serialize config")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &raw)
            .with_context(|| format!("failed to write tmp config at {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("failed to rename config to {}", path.display()))
    }
}
