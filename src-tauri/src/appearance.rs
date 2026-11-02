//! Persisted, validated appearance preferences. Colors are scoped to each mode.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ColorMode {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MotionPreference {
    #[default]
    System,
    On,
    Off,
}
#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UiFont {
    #[default]
    System,
    SegoeUi,
    Arial,
    Georgia,
}
#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ResponseFont {
    #[default]
    Same,
    SegoeUi,
    Arial,
    Georgia,
}
#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeFont {
    #[default]
    CascadiaCode,
    Consolas,
    CourierNew,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Appearance {
    pub mode: ColorMode,
    pub accent: String,
    pub light_background: Option<String>,
    pub light_foreground: Option<String>,
    pub dark_background: Option<String>,
    pub dark_foreground: Option<String>,
    pub ui_font: UiFont,
    pub response_font: ResponseFont,
    pub code_font: CodeFont,
    pub ui_font_size: u8,
    pub response_font_size: u8,
    pub code_font_size: u8,
    pub window_width: u16,
    pub chat_max_height: u16,
    pub settings_max_height: u16,
    pub reduce_motion: MotionPreference,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            mode: ColorMode::System,
            accent: "#8b5cf6".into(),
            light_background: None,
            light_foreground: None,
            dark_background: None,
            dark_foreground: None,
            ui_font: UiFont::System,
            response_font: ResponseFont::Same,
            code_font: CodeFont::CascadiaCode,
            ui_font_size: 13,
            response_font_size: 14,
            code_font_size: 12,
            window_width: 510,
            chat_max_height: 900,
            settings_max_height: 680,
            reduce_motion: MotionPreference::System,
        }
    }
}

impl Appearance {
    pub fn normalized(mut self) -> Result<Self, String> {
        fn color(value: &mut String) -> Result<(), String> {
            *value = value.trim().to_ascii_lowercase();
            if value.len() != 7
                || !value.starts_with('#')
                || !value[1..].bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err("Use a color in #RRGGBB format.".into());
            }
            Ok(())
        }
        color(&mut self.accent)?;
        for value in [
            &mut self.light_background,
            &mut self.light_foreground,
            &mut self.dark_background,
            &mut self.dark_foreground,
        ]
        .into_iter()
        .flatten()
        {
            color(value)?;
        }
        if !(12..=18).contains(&self.ui_font_size) {
            return Err("Interface font size must be 12–18 px.".into());
        }
        if !(12..=22).contains(&self.response_font_size) {
            return Err("Response font size must be 12–22 px.".into());
        }
        if !(10..=20).contains(&self.code_font_size) {
            return Err("Code font size must be 10–20 px.".into());
        }
        if !(460..=1000).contains(&self.window_width) {
            return Err("Window width must be 460-1000 px.".into());
        }
        if !(480..=1400).contains(&self.chat_max_height) {
            return Err("Maximum chat height must be 480-1400 px.".into());
        }
        if !(480..=1200).contains(&self.settings_max_height) {
            return Err("Maximum settings height must be 480-1200 px.".into());
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_and_partial_settings_get_defaults() {
        let appearance: Appearance = serde_json::from_str(r#"{"mode":"light"}"#).unwrap();
        assert_eq!(appearance.mode, ColorMode::Light);
        assert_eq!(appearance.ui_font_size, 13);
        assert_eq!(appearance.window_width, 510);
        assert_eq!(appearance.chat_max_height, 900);
        assert_eq!(appearance.settings_max_height, 680);
        assert_eq!(appearance.accent, "#8b5cf6");
    }
    #[test]
    fn persists_independent_mode_colors_and_normalizes_hex() {
        let value = Appearance {
            light_background: Some(" #FAFAFA ".into()),
            dark_background: Some("#18181B".into()),
            ..Default::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(value.light_background.as_deref(), Some("#fafafa"));
        assert_eq!(
            serde_json::from_str::<Appearance>(&serde_json::to_string(&value).unwrap()).unwrap(),
            value
        );
    }
    #[test]
    fn validates_and_persists_window_dimensions() {
        let value = Appearance {
            window_width: 800,
            chat_max_height: 1200,
            settings_max_height: 1000,
            ..Default::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Appearance>(&serde_json::to_string(&value).unwrap()).unwrap(),
            value
        );
        for (width, chat, settings) in [
            (459, 900, 680),
            (1001, 900, 680),
            (510, 479, 680),
            (510, 1401, 680),
            (510, 900, 479),
            (510, 900, 1201),
        ] {
            assert!(Appearance {
                window_width: width,
                chat_max_height: chat,
                settings_max_height: settings,
                ..Default::default()
            }
            .normalized()
            .is_err());
        }
    }
    #[test]
    fn rejects_css_injection_and_out_of_range_sizes() {
        assert!(Appearance {
            accent: "red; background:url(x)".into(),
            ..Default::default()
        }
        .normalized()
        .is_err());
        assert!(Appearance {
            ui_font_size: 0,
            ..Default::default()
        }
        .normalized()
        .is_err());
        assert!(Appearance {
            response_font_size: 23,
            ..Default::default()
        }
        .normalized()
        .is_err());
        assert!(Appearance {
            code_font_size: 21,
            ..Default::default()
        }
        .normalized()
        .is_err());
    }
}
