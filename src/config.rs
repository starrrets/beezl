//! User configuration (`%APPDATA%\cuursor\config.toml`).

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Keep the cursor at the same physical spot along the edge (millimetres).
    #[default]
    Physical,
    /// Map the whole edge onto the whole neighbouring edge.
    Proportional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Default)]
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

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub mode: Mode,
    pub align: Align,
    /// Ignore synthetic input (remote desktop, tablets, automation tools).
    pub skip_injected: bool,
    /// Moves longer than this in one event are treated as teleports and left alone.
    pub max_jump_px: i32,
    #[serde(rename = "monitor")]
    pub monitors: Vec<MonitorOverride>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: Mode::Physical,
            align: Align::Auto,
            skip_injected: true,
            max_jump_px: 600,
            monitors: Vec::new(),
        }
    }
}

/// Physical size for monitors whose EDID is missing or wrong.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorOverride {
    /// Matched against the hardware id (e.g. `GSM5B08`), the model name,
    /// the device name (`\\.\DISPLAY1`) or the resolution (`1920x1080`).
    #[serde(rename = "match")]
    pub pattern: String,
    pub diagonal_in: Option<f64>,
    pub width_mm: Option<f64>,
    pub height_mm: Option<f64>,
}

impl MonitorOverride {
    pub fn matches(&self, id: &str, model: &str, device: &str, width_px: i32, height_px: i32) -> bool {
        let p = self.pattern.trim().to_ascii_lowercase();
        if p.is_empty() {
            return false;
        }
        let res = format!("{width_px}x{height_px}");
        [id, model, device, res.as_str()].iter().any(|h| !h.is_empty() && h.to_ascii_lowercase().contains(&p))
    }

    /// Physical size in mm for a monitor of the given pixel resolution.
    pub fn size_mm(&self, width_px: i32, height_px: i32) -> Option<(f64, f64)> {
        if let (Some(w), Some(h)) = (self.width_mm, self.height_mm) {
            return (w > 0.0 && h > 0.0).then_some((w, h));
        }
        let d = self.diagonal_in.filter(|d| *d > 0.0)? * 25.4;
        let (w, h) = (width_px as f64, height_px as f64);
        let diag_px = w.hypot(h);
        Some((d * w / diag_px, d * h / diag_px))
    }
}

pub fn parse(text: &str) -> Result<Config, String> {
    toml::from_str(text).map_err(|e| e.to_string())
}

pub const DEFAULT_TOML: &str = r#"# cuursor: настройки. После правки выберите в меню трея «Перечитать настройки».

# Включено ли исправление при запуске.
enabled = true

# physical     — курсор переходит в ту же физическую точку (по миллиметрам).
# proportional — весь край одного монитора растягивается на весь край соседнего.
mode = "physical"

# Как мониторы выровнены физически на столе (только для mode = "physical"):
# auto — взять из расположения в «Параметры → Дисплей» (ближайшее из top/center/bottom),
# top | center | bottom (для мониторов друг над другом: left | center | right).
align = "auto"

# Не трогать синтетический ввод (удалённый рабочий стол, графические планшеты).
skip_injected = true

# Перемещение длиннее этого за одно событие считается телепортом и не трогается.
max_jump_px = 600

# Если размер монитора определился неверно (см. меню трея «Мониторы…»),
# задайте его вручную. match ищется в ID (например GSM5B08), модели,
# имени устройства (\\.\DISPLAY1) или разрешении (1920x1080).
#
# [[monitor]]
# match = "1920x1080"
# diagonal_in = 24
#
# [[monitor]]
# match = "2560x1440"
# width_mm = 597
# height_mm = 336
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses() {
        let c = parse(DEFAULT_TOML).unwrap();
        assert!(c.enabled);
        assert_eq!(c.mode, Mode::Physical);
        assert_eq!(c.align, Align::Auto);
        assert!(c.monitors.is_empty());
    }

    #[test]
    fn overrides() {
        let c = parse(
            r#"
            align = "bottom"
            [[monitor]]
            match = "1920x1080"
            diagonal_in = 24
            "#,
        )
        .unwrap();
        assert_eq!(c.align, Align::End);
        let m = &c.monitors[0];
        assert!(m.matches("GSM5B08", "", r"\\.\DISPLAY2", 1920, 1080));
        assert!(!m.matches("GSM5B08", "", r"\\.\DISPLAY2", 2560, 1440));
        let (w, h) = m.size_mm(1920, 1080).unwrap();
        assert!((w - 531.3).abs() < 0.5 && (h - 298.9).abs() < 0.5, "{w} {h}");
    }

    #[test]
    fn unknown_keys_are_errors() {
        assert!(parse("mdoe = \"physical\"").is_err());
    }
}
