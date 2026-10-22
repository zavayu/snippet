//! Persisted local-inference settings.

use std::{
    collections::HashSet,
    env, fs,
    path::PathBuf,
    str::FromStr,
    sync::{Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::Shortcut;

pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";
pub const DEFAULT_OCR_MODEL: &str = "glm-ocr:latest";
pub const DEFAULT_OCR_NUM_PREDICT: u32 = 2_048;
pub const DEFAULT_OCR_NUM_CTX: u32 = 16_384;
const MIN_OCR_NUM_PREDICT: u32 = 256;
const MAX_OCR_NUM_PREDICT: u32 = 8_192;
const MIN_OCR_NUM_CTX: u32 = 4_096;
const MAX_OCR_NUM_CTX: u32 = 32_768;
const SETTINGS_FILE_NAME: &str = "settings.json";
const MODEL_OVERRIDE_ENV: &str = "SNIPPET_OLLAMA_MODEL";

const fn default_automatic_updates() -> bool {
    true
}

const fn default_web_search_enabled() -> bool {
    true
}

fn default_capture_screen_shortcut() -> String {
    "Ctrl+Shift+I".into()
}

fn default_capture_region_shortcut() -> String {
    "Ctrl+Shift+G".into()
}

fn default_read_screen_shortcut() -> String {
    "Ctrl+Shift+O".into()
}

fn default_read_region_shortcut() -> String {
    "Ctrl+Shift+T".into()
}

fn default_ocr_model() -> String {
    DEFAULT_OCR_MODEL.into()
}

const fn default_ocr_num_predict() -> u32 {
    DEFAULT_OCR_NUM_PREDICT
}

const fn default_ocr_num_ctx() -> u32 {
    DEFAULT_OCR_NUM_CTX
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutConfig {
    pub open_popup: String,
    pub summarize: String,
    pub explain: String,
    pub refine: String,
    #[serde(default = "default_capture_screen_shortcut")]
    pub capture_screen: String,
    #[serde(default = "default_capture_region_shortcut")]
    pub capture_region: String,
    #[serde(default = "default_read_screen_shortcut")]
    pub read_screen: String,
    #[serde(default = "default_read_region_shortcut")]
    pub read_region: String,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            open_popup: "Ctrl+Shift+Space".into(),
            summarize: "Ctrl+Shift+S".into(),
            explain: "Ctrl+Shift+E".into(),
            refine: "Ctrl+Shift+R".into(),
            capture_screen: default_capture_screen_shortcut(),
            capture_region: default_capture_region_shortcut(),
            read_screen: default_read_screen_shortcut(),
            read_region: default_read_region_shortcut(),
        }
    }
}

impl ShortcutConfig {
    pub fn bindings(&self) -> [(&str, &str); 8] {
        [
            ("Open popup", &self.open_popup),
            ("Summarize", &self.summarize),
            ("Explain", &self.explain),
            ("Refine", &self.refine),
            ("Capture screen", &self.capture_screen),
            ("Capture region", &self.capture_region),
            ("Read screen", &self.read_screen),
            ("Read region", &self.read_region),
        ]
    }

    pub(crate) fn normalized(mut self) -> Result<Self, SettingsError> {
        let mut ids = HashSet::new();
        for (name, value) in [
            ("Open popup", &mut self.open_popup),
            ("Summarize", &mut self.summarize),
            ("Explain", &mut self.explain),
            ("Refine", &mut self.refine),
            ("Capture screen", &mut self.capture_screen),
            ("Capture region", &mut self.capture_region),
            ("Read screen", &mut self.read_screen),
            ("Read region", &mut self.read_region),
        ] {
            *value = value.trim().to_owned();
            if value.is_empty() {
                continue;
            }

            let shortcut = Shortcut::from_str(value)
                .map_err(|_| SettingsError::InvalidShortcut(name.to_owned()))?;
            if !ids.insert(shortcut.id()) {
                return Err(SettingsError::DuplicateShortcut);
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OcrEngine {
    #[default]
    Windows,
    Ollama,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub ollama_base_url: String,
    pub model: Option<String>,
    #[serde(default = "default_ocr_model")]
    pub ocr_model: String,
    #[serde(default)]
    pub ocr_engine: OcrEngine,
    #[serde(default = "default_ocr_num_predict")]
    pub ocr_num_predict: u32,
    #[serde(default = "default_ocr_num_ctx")]
    pub ocr_num_ctx: u32,
    #[serde(default)]
    pub thinking: bool,
    #[serde(default)]
    pub vision_enabled: bool,
    #[serde(default)]
    pub has_completed_onboarding: bool,
    #[serde(default = "default_automatic_updates")]
    pub automatic_updates: bool,
    #[serde(default = "default_web_search_enabled")]
    pub web_search_enabled: bool,
    #[serde(default)]
    pub shortcuts: ShortcutConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ollama_base_url: DEFAULT_OLLAMA_BASE_URL.into(),
            model: None,
            ocr_engine: OcrEngine::Windows,
            ocr_model: DEFAULT_OCR_MODEL.into(),
            ocr_num_predict: DEFAULT_OCR_NUM_PREDICT,
            ocr_num_ctx: DEFAULT_OCR_NUM_CTX,
            thinking: false,
            vision_enabled: false,
            has_completed_onboarding: false,
            automatic_updates: true,
            web_search_enabled: true,
            shortcuts: ShortcutConfig::default(),
        }
    }
}

impl AppConfig {
    pub(crate) fn normalized(mut self) -> Result<Self, SettingsError> {
        let base_url = self.ollama_base_url.trim().trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            return Err(SettingsError::InvalidBaseUrl);
        }

        let url = reqwest::Url::parse(&base_url).map_err(|_| SettingsError::InvalidBaseUrl)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(SettingsError::InvalidBaseUrl);
        }

        self.ollama_base_url = base_url;
        self.model = self
            .model
            .and_then(|model| (!model.trim().is_empty()).then(|| model.trim().to_owned()));
        self.ocr_model = self.ocr_model.trim().to_owned();
        if self.ocr_model.is_empty() {
            return Err(SettingsError::InvalidOcrModel);
        }
        if !(MIN_OCR_NUM_PREDICT..=MAX_OCR_NUM_PREDICT).contains(&self.ocr_num_predict) {
            return Err(SettingsError::InvalidOcrNumPredict);
        }
        if !(MIN_OCR_NUM_CTX..=MAX_OCR_NUM_CTX).contains(&self.ocr_num_ctx) {
            return Err(SettingsError::InvalidOcrNumCtx);
        }
        self.shortcuts = self.shortcuts.normalized()?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub config: AppConfig,
    pub effective_model: Option<String>,
    pub model_source: ModelSource,
    pub has_search_api_key: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelSource {
    Settings,
    Environment,
    None,
}

pub struct SettingsStore {
    file_path: PathBuf,
    config: Mutex<AppConfig>,
}

impl SettingsStore {
    pub fn load(app: &AppHandle) -> Result<Self, SettingsError> {
        let directory = app
            .path()
            .app_config_dir()
            .map_err(|error| SettingsError::Read(error.to_string()))?;
        let file_path = directory.join(SETTINGS_FILE_NAME);

        let config = if file_path.exists() {
            let contents = fs::read_to_string(&file_path)
                .map_err(|error| SettingsError::Read(error.to_string()))?;
            serde_json::from_str::<AppConfig>(&contents)
                .map_err(|error| SettingsError::Read(error.to_string()))?
                .normalized()?
        } else {
            AppConfig::default()
        };

        Ok(Self {
            file_path,
            config: Mutex::new(config),
        })
    }

    pub fn snapshot(&self) -> Result<SettingsSnapshot, SettingsError> {
        let config = self.lock_config()?.clone();
        Ok(snapshot_for(config))
    }

    pub fn update(&self, config: AppConfig) -> Result<SettingsSnapshot, SettingsError> {
        let config = config.normalized()?;
        let directory = self
            .file_path
            .parent()
            .ok_or_else(|| SettingsError::Write("Settings path has no parent directory.".into()))?;
        fs::create_dir_all(directory).map_err(|error| SettingsError::Write(error.to_string()))?;

        let serialized = serde_json::to_string_pretty(&config)
            .map_err(|error| SettingsError::Write(error.to_string()))?;
        fs::write(&self.file_path, serialized)
            .map_err(|error| SettingsError::Write(error.to_string()))?;

        *self.lock_config()? = config;
        self.snapshot()
    }

    fn lock_config(&self) -> Result<MutexGuard<'_, AppConfig>, SettingsError> {
        self.config
            .lock()
            .map_err(|_| SettingsError::Read("Settings are temporarily unavailable.".into()))
    }
}

fn snapshot_for(config: AppConfig) -> SettingsSnapshot {
    let environment_model = env::var(MODEL_OVERRIDE_ENV)
        .ok()
        .filter(|model| !model.trim().is_empty())
        .map(|model| model.trim().to_owned());

    let (effective_model, model_source) = match environment_model {
        Some(model) => (Some(model), ModelSource::Environment),
        None => match config.model.clone() {
            Some(model) => (Some(model), ModelSource::Settings),
            None => (None, ModelSource::None),
        },
    };

    SettingsSnapshot {
        config,
        effective_model,
        model_source,
        has_search_api_key: crate::search_credentials::read().ok().flatten().is_some(),
    }
}

#[derive(Debug)]
pub enum SettingsError {
    InvalidBaseUrl,
    InvalidOcrModel,
    InvalidOcrNumPredict,
    InvalidOcrNumCtx,
    InvalidShortcut(String),
    DuplicateShortcut,
    Read(String),
    Write(String),
}

impl SettingsError {
    pub fn user_message(&self) -> String {
        match self {
            Self::InvalidBaseUrl => "Enter a valid http:// or https:// Ollama address.".into(),
            Self::InvalidOcrModel => "Choose an OCR model before using Read screen.".into(),
            Self::InvalidOcrNumPredict => format!(
                "OCR output limit must be between {MIN_OCR_NUM_PREDICT} and {MAX_OCR_NUM_PREDICT} tokens."
            ),
            Self::InvalidOcrNumCtx => format!(
                "OCR context must be between {MIN_OCR_NUM_CTX} and {MAX_OCR_NUM_CTX} tokens."
            ),
            Self::InvalidShortcut(action) => format!(
                "{action} needs a shortcut such as Ctrl+Shift+S, or leave it blank to disable it."
            ),
            Self::DuplicateShortcut => "Each shortcut needs a different key combination.".into(),
            Self::Read(error) => format!("Snippet could not read its saved settings: {error}"),
            Self::Write(error) => format!("Snippet could not save your settings: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        snapshot_for, AppConfig, ModelSource, OcrEngine, SettingsError, ShortcutConfig,
        DEFAULT_OCR_MODEL, DEFAULT_OCR_NUM_CTX, DEFAULT_OCR_NUM_PREDICT, DEFAULT_OLLAMA_BASE_URL,
    };

    #[test]
    fn web_search_defaults_on_and_can_be_persistently_disabled_without_storing_secrets() {
        let mut config: AppConfig =
            serde_json::from_str(r#"{"ollamaBaseUrl":"http://localhost:11434","model":null}"#)
                .unwrap();
        assert!(config.web_search_enabled);
        config.web_search_enabled = false;
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !serde_json::from_str::<AppConfig>(&json)
                .unwrap()
                .web_search_enabled
        );
        assert!(!json.contains("apiKey"));
    }

    #[test]
    fn defaults_to_the_local_ollama_server() {
        let config = AppConfig::default();
        assert_eq!(config.ollama_base_url, DEFAULT_OLLAMA_BASE_URL);
        assert_eq!(config.model, None);
        assert_eq!(config.ocr_engine, super::OcrEngine::Windows);
        assert_eq!(config.ocr_model, DEFAULT_OCR_MODEL);
        assert_eq!(config.ocr_num_predict, DEFAULT_OCR_NUM_PREDICT);
        assert_eq!(config.ocr_num_ctx, DEFAULT_OCR_NUM_CTX);
        assert!(!config.thinking);
        assert!(!config.vision_enabled);
        assert!(!config.has_completed_onboarding);
    }

    #[test]
    fn normalizes_a_configured_url_and_model() {
        let config = AppConfig {
            ollama_base_url: " http://localhost:11434/ ".into(),
            model: Some(" qwen3:8b ".into()),
            ocr_engine: OcrEngine::Windows,
            ocr_model: " glm-ocr:q8_0 ".into(),
            ocr_num_predict: 1_024,
            ocr_num_ctx: 8_192,
            thinking: true,
            vision_enabled: true,
            has_completed_onboarding: true,
            automatic_updates: true,
            web_search_enabled: true,
            shortcuts: ShortcutConfig::default(),
        }
        .normalized()
        .unwrap();

        assert_eq!(config.ollama_base_url, DEFAULT_OLLAMA_BASE_URL);
        assert_eq!(config.model.as_deref(), Some("qwen3:8b"));
        assert_eq!(config.ocr_model, "glm-ocr:q8_0");
        assert_eq!(config.ocr_num_predict, 1_024);
        assert_eq!(config.ocr_num_ctx, 8_192);
        assert!(config.thinking);
        assert!(config.vision_enabled);
        assert!(config.has_completed_onboarding);
    }

    #[test]
    fn rejects_an_invalid_base_url() {
        let error = AppConfig {
            ollama_base_url: "not-a-url".into(),
            model: None,
            ocr_engine: OcrEngine::Windows,
            ocr_model: DEFAULT_OCR_MODEL.into(),
            ocr_num_predict: DEFAULT_OCR_NUM_PREDICT,
            ocr_num_ctx: DEFAULT_OCR_NUM_CTX,
            thinking: false,
            vision_enabled: false,
            has_completed_onboarding: false,
            automatic_updates: true,
            web_search_enabled: true,
            shortcuts: ShortcutConfig::default(),
        }
        .normalized()
        .unwrap_err();

        assert!(matches!(error, SettingsError::InvalidBaseUrl));
    }

    #[test]
    fn provides_the_default_shortcuts() {
        assert_eq!(ShortcutConfig::default().open_popup, "Ctrl+Shift+Space");
        assert_eq!(ShortcutConfig::default().summarize, "Ctrl+Shift+S");
        assert_eq!(ShortcutConfig::default().explain, "Ctrl+Shift+E");
        assert_eq!(ShortcutConfig::default().refine, "Ctrl+Shift+R");
        assert_eq!(ShortcutConfig::default().capture_screen, "Ctrl+Shift+I");
        assert_eq!(ShortcutConfig::default().capture_region, "Ctrl+Shift+G");
        assert_eq!(ShortcutConfig::default().read_screen, "Ctrl+Shift+O");
        assert_eq!(ShortcutConfig::default().read_region, "Ctrl+Shift+T");
    }

    #[test]
    fn rejects_invalid_or_duplicate_shortcuts() {
        let invalid = ShortcutConfig {
            summarize: "not a shortcut".into(),
            ..ShortcutConfig::default()
        }
        .normalized()
        .unwrap_err();
        assert!(matches!(invalid, SettingsError::InvalidShortcut(_)));

        let duplicate = ShortcutConfig {
            explain: "Ctrl+Shift+S".into(),
            ..ShortcutConfig::default()
        }
        .normalized()
        .unwrap_err();
        assert!(matches!(duplicate, SettingsError::DuplicateShortcut));
    }

    #[test]
    fn adds_the_screen_capture_shortcut_to_existing_shortcut_settings() {
        let shortcuts = serde_json::from_str::<ShortcutConfig>(
            r#"{"openPopup":"Ctrl+Shift+Space","summarize":"Ctrl+Shift+S","explain":"Ctrl+Shift+E","refine":"Ctrl+Shift+R"}"#,
        )
        .unwrap();

        assert_eq!(shortcuts.capture_screen, "Ctrl+Shift+I");
        assert_eq!(shortcuts.capture_region, "Ctrl+Shift+G");
        assert_eq!(shortcuts.read_screen, "Ctrl+Shift+O");
        assert_eq!(shortcuts.read_region, "Ctrl+Shift+T");
    }

    #[test]
    fn adds_the_default_ocr_model_to_existing_settings() {
        let config = serde_json::from_str::<AppConfig>(
            r#"{"ollamaBaseUrl":"http://localhost:11434","model":null,"thinking":false,"visionEnabled":false,"shortcuts":{"openPopup":"Ctrl+Shift+Space","summarize":"Ctrl+Shift+S","explain":"Ctrl+Shift+E","refine":"Ctrl+Shift+R"}}"#,
        )
        .unwrap();

        assert_eq!(config.ocr_engine, super::OcrEngine::Windows);
        assert_eq!(config.ocr_model, DEFAULT_OCR_MODEL);
        assert!(!config.has_completed_onboarding);
        assert!(config.automatic_updates);
        assert!(config.web_search_enabled);
    }

    #[test]
    fn persists_an_explicit_ollama_engine_choice() {
        let config = AppConfig {
            ocr_engine: OcrEngine::Ollama,
            ..AppConfig::default()
        };
        let saved = serde_json::to_string(&config).unwrap();
        assert_eq!(
            serde_json::from_str::<AppConfig>(&saved)
                .unwrap()
                .ocr_engine,
            OcrEngine::Ollama
        );
    }

    #[test]
    fn rejects_an_empty_ocr_model() {
        let error = AppConfig {
            ocr_model: " ".into(),
            ..AppConfig::default()
        }
        .normalized()
        .unwrap_err();

        assert!(matches!(error, SettingsError::InvalidOcrModel));
    }

    #[test]
    fn reports_no_model_when_settings_are_empty() {
        let snapshot = snapshot_for(AppConfig::default());
        if std::env::var("SNIPPET_OLLAMA_MODEL").is_ok() {
            return;
        }

        assert_eq!(snapshot.effective_model, None);
        assert!(matches!(snapshot.model_source, ModelSource::None));
    }
}
