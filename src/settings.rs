use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub terminal_font_family: String,
    pub terminal_font_size: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            terminal_font_family: "Menlo".into(),
            terminal_font_size: 11.,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        if let Ok(data) = std::fs::read_to_string(&path) {
            return serde_json::from_str::<Self>(&data)
                .map(Self::clamped)
                .unwrap_or_else(|error| {
                    eprintln!("settings: {}: {error}", path.display());
                    Self::default()
                });
        }
        let settings = Self::default();
        settings.save();
        settings
    }

    fn clamped(mut self) -> Self {
        if self.terminal_font_family.trim().is_empty() {
            self.terminal_font_family = Self::default().terminal_font_family;
        }
        self.terminal_font_size = self.terminal_font_size.clamp(8., 24.);
        self
    }

    fn save(&self) {
        let Some(path) = path() else {
            return;
        };
        if let Some(directory) = path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, data);
        }
    }
}

pub fn path() -> Option<PathBuf> {
    crate::data_dir().map(|directory| directory.join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn clamps_font_settings() {
        let settings = Settings {
            terminal_font_family: "  ".into(),
            terminal_font_size: 200.,
        }
        .clamped();
        assert_eq!(settings.terminal_font_family, "Menlo");
        assert_eq!(settings.terminal_font_size, 24.);
    }

    #[test]
    fn partial_json_uses_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"terminal_font_size": 15}"#).unwrap();
        assert_eq!(settings.terminal_font_family, "Menlo");
        assert_eq!(settings.terminal_font_size, 15.);
    }
}
