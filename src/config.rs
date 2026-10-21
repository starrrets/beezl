//! Settings, stored as `%APPDATA%\beezl\config.toml` and edited through the
//! settings window.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Keep the cursor at the same physical spot along the edge (millimetres).
    #[default]
    Physical,
    /// Map the whole edge onto the whole neighbouring edge.
    Proportional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    /// Infer from the arrangement in Windows Display Settings.
    #[default]
    Auto,
    #[serde(alias = "top", alias = "left")]
    Start,
    Center,
    #[serde(alias = "bottom", alias = "right")]
    End,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub autostart: bool,
    pub mode: Mode,
    pub align: Align,
    /// Ignore synthetic input (remote desktop, tablets, automation tools).
    pub skip_injected: bool,
    /// Moves longer than this in one event are treated as teleports and left alone.
    pub max_jump_px: i32,
    #[serde(rename = "monitor", skip_serializing_if = "Vec::is_empty")]
    pub monitors: Vec<MonitorOverride>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            autostart: true,
            mode: Mode::Physical,
            align: Align::Auto,
            skip_injected: true,
            max_jump_px: 600,
            monitors: Vec::new(),
        }
    }
}

impl Config {
    /// Manual size for the monitor with this key, replacing any earlier one;
    /// `None` goes back to the size reported by the monitor.
    pub fn set_diagonal(&mut self, key: &str, diagonal_in: Option<f64>) {
        self.monitors.retain(|m| !m.pattern.eq_ignore_ascii_case(key));
        if let Some(d) = diagonal_in.filter(|d| *d > 0.0) {
            self.monitors.push(MonitorOverride {
                pattern: key.to_string(),
                diagonal_in: Some(d),
                width_mm: None,
                height_mm: None,
            });
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }
}

/// Physical size for monitors whose EDID is missing or wrong.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorOverride {
    /// Matched against the monitor key (hardware id + instance, e.g.
    /// `GSM5B08\5&1a2b3c&0&UID4353`), the model name, the device name
    /// (`\\.\DISPLAY1`) or the resolution (`1920x1080`).
    #[serde(rename = "match")]
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagonal_in: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_mm: Option<f64>,
}

impl MonitorOverride {
    pub fn matches(&self, key: &str, model: &str, device: &str, width_px: i32, height_px: i32) -> bool {
        let p = self.pattern.trim().to_ascii_lowercase();
        if p.is_empty() {
            return false;
        }
        let res = format!("{width_px}x{height_px}");
        [key, model, device, res.as_str()].iter().any(|h| !h.is_empty() && h.to_ascii_lowercase().contains(&p))
    }

    /// Physical size in mm for a monitor of the given pixel resolution.
    pub fn size_mm(&self, width_px: i32, height_px: i32) -> Option<(f64, f64)> {
        if let (Some(w), Some(h)) = (self.width_mm, self.height_mm) {
            return (w > 0.0 && h > 0.0).then_some((w, h));
        }
        let d = self.diagonal_in.filter(|d| *d > 0.0)?;
        Some(size_from_diagonal(d, width_px, height_px))
    }
}

/// Width and height in mm of a panel with square pixels.
pub fn size_from_diagonal(diagonal_in: f64, width_px: i32, height_px: i32) -> (f64, f64) {
    let d = diagonal_in * 25.4;
    let (w, h) = (width_px as f64, height_px as f64);
    let diag_px = w.hypot(h);
    (d * w / diag_px, d * h / diag_px)
}

/// Unknown or malformed keys fall back to defaults instead of failing, so an
/// older or newer config never stops the app from starting.
pub fn parse(text: &str) -> Result<Config, String> {
    toml::from_str(text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut c = Config { align: Align::End, ..Default::default() };
        c.set_diagonal(r"GSM5B08\5&1a2b&0&UID4353", Some(24.5));
        let back = parse(&c.to_toml()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn empty_and_old_files_parse() {
        assert_eq!(parse("").unwrap(), Config::default());
        // v0.1 file with comments and the old keys.
        let c = parse(
            r#"
            # comment
            enabled = true
            mode = "physical"
            align = "bottom"
            skip_injected = true
            max_jump_px = 600
            future_key = 1
            [[monitor]]
            match = "1920x1080"
            diagonal_in = 24
            "#,
        )
        .unwrap();
        assert_eq!(c.align, Align::End);
        assert!(c.autostart);
        let m = &c.monitors[0];
        assert!(m.matches("GSM5B08", "", r"\\.\DISPLAY2", 1920, 1080));
        assert!(!m.matches("GSM5B08", "", r"\\.\DISPLAY2", 2560, 1440));
        let (w, h) = m.size_mm(1920, 1080).unwrap();
        assert!((w - 531.3).abs() < 0.5 && (h - 298.9).abs() < 0.5, "{w} {h}");
    }

    #[test]
    fn set_diagonal_replaces_and_clears() {
        let mut c = Config::default();
        c.set_diagonal("A", Some(24.0));
        c.set_diagonal("a", Some(27.0));
        assert_eq!(c.monitors.len(), 1);
        assert_eq!(c.monitors[0].diagonal_in, Some(27.0));
        c.set_diagonal("A", None);
        assert!(c.monitors.is_empty());
    }
}
