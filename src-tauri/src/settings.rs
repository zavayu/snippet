//! Persisted local-inference settings.

use std::{
    env, fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";
const SETTINGS_FILE_NAME: &str = "settings.json";
const MODEL_OVERRIDE_ENV: &str = "SNIPPET_OLLAMA_MODEL";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub ollama_base_url: String,
    pub model: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ollama_base_url: DEFAULT_OLLAMA_BASE_URL.into(),
            model: None,
        }
    }
}

impl AppConfig {
    fn normalized(mut self) -> Result<Self, SettingsError> {
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
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub config: AppConfig,
    pub effective_model: Option<String>,
    pub model_source: ModelSource,
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
    }
}

#[derive(Debug)]
pub enum SettingsError {
    InvalidBaseUrl,
    Read(String),
    Write(String),
}

impl SettingsError {
    pub fn user_message(&self) -> String {
        match self {
            Self::InvalidBaseUrl => "Enter a valid http:// or https:// Ollama address.".into(),
            Self::Read(error) => format!("Snippet could not read its saved settings: {error}"),
            Self::Write(error) => format!("Snippet could not save your settings: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{snapshot_for, AppConfig, ModelSource, SettingsError, DEFAULT_OLLAMA_BASE_URL};

    #[test]
    fn defaults_to_the_local_ollama_server() {
        let config = AppConfig::default();
        assert_eq!(config.ollama_base_url, DEFAULT_OLLAMA_BASE_URL);
        assert_eq!(config.model, None);
    }

    #[test]
    fn normalizes_a_configured_url_and_model() {
        let config = AppConfig {
            ollama_base_url: " http://localhost:11434/ ".into(),
            model: Some(" qwen3:8b ".into()),
        }
        .normalized()
        .unwrap();

        assert_eq!(config.ollama_base_url, DEFAULT_OLLAMA_BASE_URL);
        assert_eq!(config.model.as_deref(), Some("qwen3:8b"));
    }

    #[test]
    fn rejects_an_invalid_base_url() {
        let error = AppConfig {
            ollama_base_url: "not-a-url".into(),
            model: None,
        }
        .normalized()
        .unwrap_err();

        assert!(matches!(error, SettingsError::InvalidBaseUrl));
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
