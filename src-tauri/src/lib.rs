pub mod prompt;
pub mod provider;
mod selection;
mod settings;

use std::{
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "windows")]
use tauri::{PhysicalPosition, Runtime};
#[cfg(target_os = "windows")]
use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

#[cfg(debug_assertions)]
use crate::prompt::ChatMessage;
use crate::{
    prompt::{PromptBuilder, PromptRequest},
    provider::{
        ollama::OllamaProvider, GenerationEvent, GenerationRequest, LlmProvider, ModelInfo,
        ProviderError,
    },
    selection::{CaptureError, CapturedSelection},
    settings::{AppConfig, SettingsSnapshot, SettingsStore},
};

const SELECTION_CAPTURED_EVENT: &str = "selection-captured";
const SELECTION_CAPTURE_FAILED_EVENT: &str = "selection-capture-failed";
const GENERATION_STARTED_EVENT: &str = "generation-started";
const GENERATION_CHUNK_EVENT: &str = "generation-chunk";
const GENERATION_FINISHED_EVENT: &str = "generation-finished";
const GENERATION_FAILED_EVENT: &str = "generation-failed";
const GENERATION_CANCELLED_EVENT: &str = "generation-cancelled";
const POPUP_CURSOR_OFFSET_X: i32 = 14;
const POPUP_CURSOR_OFFSET_Y: i32 = 18;

#[derive(Default)]
struct CaptureState(Arc<AtomicBool>);

#[derive(Default, Clone)]
struct GenerationState(Arc<GenerationStateInner>);

#[derive(Default)]
struct GenerationStateInner {
    next_request_id: AtomicU64,
    active: Mutex<Option<ActiveGeneration>>,
}

struct ActiveGeneration {
    request_id: u64,
    cancellation: CancellationToken,
}

impl GenerationState {
    fn begin(&self) -> (u64, CancellationToken) {
        let mut active = self.0.active.lock().expect("generation state poisoned");
        if let Some(previous) = active.as_ref() {
            previous.cancellation.cancel();
        }

        let request_id = self.0.next_request_id.fetch_add(1, Ordering::Relaxed) + 1;
        let cancellation = CancellationToken::new();
        *active = Some(ActiveGeneration {
            request_id,
            cancellation: cancellation.clone(),
        });
        (request_id, cancellation)
    }

    fn cancel(&self) {
        if let Ok(active) = self.0.active.lock() {
            if let Some(active) = active.as_ref() {
                active.cancellation.cancel();
            }
        }
    }

    fn is_active(&self, request_id: u64) -> bool {
        self.0
            .active
            .lock()
            .ok()
            .and_then(|active| {
                active
                    .as_ref()
                    .map(|active| active.request_id == request_id)
            })
            .unwrap_or(false)
    }

    fn finish(&self, request_id: u64) -> bool {
        let Ok(mut active) = self.0.active.lock() else {
            return false;
        };
        if active
            .as_ref()
            .is_some_and(|active| active.request_id == request_id)
        {
            *active = None;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Serialize)]
struct CaptureFailure {
    message: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateForSelectionRequest {
    selected_text: String,
    intent: crate::prompt::PromptIntent,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationStarted {
    request_id: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationChunk {
    request_id: u64,
    delta: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationFinished {
    request_id: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationFailure {
    request_id: u64,
    message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
enum ProviderStatus {
    Ready {
        models: Vec<ModelInfo>,
    },
    Unavailable {
        message: String,
    },
    ModelNotConfigured {
        models: Vec<ModelInfo>,
    },
    ModelMissing {
        model: String,
        models: Vec<ModelInfo>,
    },
}

/// Development-only bridge for inspecting provider-neutral messages. This
/// command is omitted from release builds.
#[cfg(debug_assertions)]
#[tauri::command]
fn preview_prompt(request: PromptRequest) -> Result<Vec<ChatMessage>, String> {
    PromptBuilder::build(request).map_err(|error| error.user_message().into())
}

#[cfg(debug_assertions)]
macro_rules! snippet_command_handler {
    () => {
        tauri::generate_handler![
            get_settings,
            save_settings,
            get_provider_status,
            generate_for_selection,
            cancel_generation,
            preview_prompt,
        ]
    };
}

#[cfg(not(debug_assertions))]
macro_rules! snippet_command_handler {
    () => {
        tauri::generate_handler![
            get_settings,
            save_settings,
            get_provider_status,
            generate_for_selection,
            cancel_generation,
        ]
    };
}

#[tauri::command]
fn get_settings(settings: State<SettingsStore>) -> Result<SettingsSnapshot, String> {
    settings.snapshot().map_err(|error| error.user_message())
}

#[tauri::command]
fn save_settings(
    settings: State<SettingsStore>,
    config: AppConfig,
) -> Result<SettingsSnapshot, String> {
    settings
        .update(config)
        .map_err(|error| error.user_message())
}

#[tauri::command]
async fn get_provider_status(settings: State<'_, SettingsStore>) -> Result<ProviderStatus, String> {
    let snapshot = settings.snapshot().map_err(|error| error.user_message())?;
    let provider = OllamaProvider::new(&snapshot.config.ollama_base_url)
        .map_err(|error| error.user_message())?;

    match provider.list_models().await {
        Ok(models) => match snapshot.effective_model {
            Some(model) if models.iter().any(|installed| installed.name == model) => {
                Ok(ProviderStatus::Ready { models })
            }
            Some(model) => Ok(ProviderStatus::ModelMissing { model, models }),
            None => Ok(ProviderStatus::ModelNotConfigured { models }),
        },
        Err(error) => Ok(ProviderStatus::Unavailable {
            message: error.user_message(),
        }),
    }
}

#[tauri::command]
fn generate_for_selection(
    app: AppHandle,
    settings: State<SettingsStore>,
    generation_state: State<GenerationState>,
    request: GenerateForSelectionRequest,
) -> Result<GenerationStarted, String> {
    let snapshot = settings.snapshot().map_err(|error| error.user_message())?;
    let model = snapshot
        .effective_model
        .ok_or_else(|| "Choose an Ollama model in Settings before generating.".to_owned())?;
    let messages = PromptBuilder::build(PromptRequest {
        selected_text: request.selected_text,
        intent: request.intent,
        context: None,
    })
    .map_err(|error| error.user_message())?;
    let provider = OllamaProvider::new(&snapshot.config.ollama_base_url)
        .map_err(|error| error.user_message())?;

    let (request_id, cancellation) = generation_state.begin();
    let started = GenerationStarted { request_id };
    let _ = app.emit(GENERATION_STARTED_EVENT, started.clone());

    let app_handle = app.clone();
    let state = (*generation_state).clone();
    tauri::async_runtime::spawn(async move {
        let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
        let provider_task = tauri::async_runtime::spawn(async move {
            provider
                .stream_generation(
                    GenerationRequest {
                        model,
                        messages,
                        thinking: snapshot.config.thinking,
                    },
                    cancellation,
                    event_sender,
                )
                .await
        });

        while let Some(event) = event_receiver.recv().await {
            if !state.is_active(request_id) {
                break;
            }

            match event {
                GenerationEvent::Delta(delta) => {
                    let _ = app_handle.emit(
                        GENERATION_CHUNK_EVENT,
                        GenerationChunk { request_id, delta },
                    );
                }
            }
        }

        let result = match provider_task.await {
            Ok(result) => result,
            Err(error) => Err(ProviderError::RequestFailed(error.to_string())),
        };
        if !state.finish(request_id) {
            return;
        }

        match result {
            Ok(()) => {
                let _ =
                    app_handle.emit(GENERATION_FINISHED_EVENT, GenerationFinished { request_id });
            }
            Err(ProviderError::Cancelled) => {
                let _ = app_handle.emit(
                    GENERATION_CANCELLED_EVENT,
                    GenerationFinished { request_id },
                );
            }
            Err(error) => {
                let _ = app_handle.emit(
                    GENERATION_FAILED_EVENT,
                    GenerationFailure {
                        request_id,
                        message: error.user_message(),
                    },
                );
            }
        }
    });

    Ok(started)
}

#[tauri::command]
fn cancel_generation(generation_state: State<GenerationState>) {
    generation_state.cancel();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let trigger_shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);
    let registered_shortcut =
        Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);

    let builder = tauri::Builder::default()
        .manage(CaptureState::default())
        .manage(GenerationState::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if shortcut != &trigger_shortcut || event.state() != ShortcutState::Released {
                        return;
                    }

                    let state = app.state::<CaptureState>().0.clone();
                    if state.swap(true, Ordering::AcqRel) {
                        return;
                    }

                    let app_handle = app.clone();
                    thread::spawn(move || {
                        let capture_result = selection::capture_default_selection();
                        publish_capture_result(&app_handle, capture_result);
                        state.store(false, Ordering::Release);
                    });
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(snippet_command_handler!());

    builder
        .setup(move |app| {
            let settings = SettingsStore::load(&app.handle())
                .map_err(|error| io::Error::other(error.user_message()))?;
            app.manage(settings);
            app.global_shortcut().register(registered_shortcut)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn publish_capture_result(
    app: &AppHandle,
    capture_result: Result<CapturedSelection, CaptureError>,
) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        position_popup_near_cursor(&window);

        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }

    match capture_result {
        Ok(selection) => {
            let _ = app.emit(SELECTION_CAPTURED_EVENT, selection);
        }
        Err(error) => {
            let _ = app.emit(
                SELECTION_CAPTURE_FAILED_EVENT,
                CaptureFailure {
                    message: error.user_message(),
                },
            );
        }
    }
}

#[cfg(target_os = "windows")]
fn position_popup_near_cursor<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) } == 0 {
        return;
    }

    let mut x = cursor.x + POPUP_CURSOR_OFFSET_X;
    let mut y = cursor.y + POPUP_CURSOR_OFFSET_Y;

    if let (Ok(Some(monitor)), Ok(size)) = (
        window.monitor_from_point(cursor.x as f64, cursor.y as f64),
        window.outer_size(),
    ) {
        let work_area = monitor.work_area();
        let max_x = (work_area.position.x + work_area.size.width as i32 - size.width as i32)
            .max(work_area.position.x);
        let max_y = (work_area.position.y + work_area.size.height as i32 - size.height as i32)
            .max(work_area.position.y);

        x = x.clamp(work_area.position.x, max_x);
        y = y.clamp(work_area.position.y, max_y);
    }

    let _ = window.set_position(PhysicalPosition::new(x, y));
}
