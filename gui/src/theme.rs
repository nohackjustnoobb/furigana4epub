use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::{App, Window};
use tempfile::NamedTempFile;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub const CHOICES: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub fn load() -> Self {
        let Some(value) = config_directory()
            .and_then(|directory| fs::read_to_string(directory.join("theme")).ok())
        else {
            return Self::default();
        };
        Self::CHOICES
            .into_iter()
            .find(|theme| theme.label() == value.trim())
            .unwrap_or_default()
    }

    pub fn save(self) -> Result<()> {
        let directory =
            config_directory().context("Could not find the user configuration directory")?;
        fs::create_dir_all(&directory)?;
        let mut file = NamedTempFile::new_in(&directory)?;
        writeln!(file, "{}", self.label())?;
        file.persist(directory.join("theme"))?;
        Ok(())
    }

    pub fn apply(self, window: &Window, cx: &mut App) {
        let mode = match self {
            Self::System => window.appearance().into(),
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
        };
        if cx.theme().mode != mode {
            Theme::change(mode, None, cx);
        }
    }
}

fn config_directory() -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join("furigana4epub"))
}
