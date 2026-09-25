pub mod prompt;
pub mod provider;
mod selection;
mod settings;

use std::{
    io,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
};

use arboard::Clipboard;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "windows")]
use tauri::{PhysicalPosition, Runtime};
#[cfg(target_os = "windows")]
use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

#[cfg(debug_assertions)]
use crate::prompt::ChatMessage;
use crate::{
    prompt::{PromptBuilder, PromptRequest, QuickAction},
    provider::{
        ollama::OllamaProvider, GenerationEvent, GenerationRequest, LlmProvider, ModelInfo,
        ProviderError,
    },
    selection::{CaptureError, CapturedSelection},
    settings::{AppConfig, SettingsSnapshot, SettingsStore, ShortcutConfig},
};

const SELECTION_CAPTURED_EVENT: &str = "selection-captured";
const SELECTION_CAPTURE_FAILED_EVENT: &str = "selection-capture-failed";
const GENERATION_STARTED_EVENT: &str = "generation-started";
const GENERATION_CHUNK_EVENT: &str = "generation-chunk";
const GENERATION_FINISHED_EVENT: &str = "generation-finished";
const GENERATION_FAILED_EVENT: &str = "generation-failed";
const GENERATION_CANCELLED_EVENT: &str = "generation-cancelled";
const POPUP_CONTENT_INSET: i32 = 24;
const POPUP_CURSOR_GAP_X: i32 = 8;
const POPUP_CURSOR_GAP_Y: i32 = 10;
const POPUP_WINDOW_OFFSET_X: i32 = POPUP_CURSOR_GAP_X - POPUP_CONTENT_INSET;
const POPUP_WINDOW_OFFSET_Y: i32 = POPUP_CURSOR_GAP_Y - POPUP_CONTENT_INSET;

#[derive(Default)]
struct CaptureState(Arc<AtomicBool>);

#[derive(Clone, Copy)]
enum ShortcutAction {
    OpenPopup,
    QuickAction(QuickAction),
}

#[derive(Default)]
struct ShortcutBindings(Mutex<Vec<(Shortcut, ShortcutAction)>>);

impl ShortcutBindings {
    fn action_for(&self, shortcut: &Shortcut) -> Option<ShortcutAction> {
        self.0
            .lock()
            .ok()?
            .iter()
            .find_map(|(registered, action)| (registered == shortcut).then_some(*action))
    }

    fn replace(&self, bindings: Vec<(Shortcut, ShortcutAction)>) {
        if let Ok(mut current) = self.0.lock() {
            *current = bindings;
        }
    }
}

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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectionCaptureEvent {
    selection: CapturedSelection,
    action: Option<QuickAction>,
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
            copy_response,
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
            copy_response,
        ]
    };
}

#[tauri::command]
fn get_settings(settings: State<SettingsStore>) -> Result<SettingsSnapshot, String> {
    settings.snapshot().map_err(|error| error.user_message())
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    settings: State<SettingsStore>,
    shortcut_bindings: State<ShortcutBindings>,
    config: AppConfig,
) -> Result<SettingsSnapshot, String> {
    let previous = settings.snapshot().map_err(|error| error.user_message())?;
    let config = config.normalized().map_err(|error| error.user_message())?;

    apply_shortcuts(&app, &shortcut_bindings, &config.shortcuts)?;
    match settings.update(config) {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => {
            let _ = apply_shortcuts(&app, &shortcut_bindings, &previous.config.shortcuts);
            Err(error.user_message())
        }
    }
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

#[tauri::command]
fn copy_response(text: String) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("There is no response to copy yet.".into());
    }

    let mut clipboard = Clipboard::new().map_err(|_| {
        "Snippet could not access the clipboard. Another application may be using it.".to_owned()
    })?;
    clipboard
        .set_text(text)
        .map_err(|_| "Snippet could not copy the response. Try again.".to_owned())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .manage(CaptureState::default())
        .manage(GenerationState::default())
        .manage(ShortcutBindings::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Released {
                        return;
                    }

                    let Some(action) = app
                        .try_state::<ShortcutBindings>()
                        .and_then(|bindings| bindings.action_for(shortcut))
                    else {
                        return;
                    };
                    start_capture(app, action);
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(snippet_command_handler!());

    builder
        .setup(move |app| {
            let settings = SettingsStore::load(&app.handle())
                .map_err(|error| io::Error::other(error.user_message()))?;
            let shortcut_config = settings
                .snapshot()
                .map_err(|error| io::Error::other(error.user_message()))?
                .config
                .shortcuts;
            app.manage(settings);
            let bindings = app.state::<ShortcutBindings>();
            apply_shortcuts(&app.handle(), &bindings, &shortcut_config)
                .map_err(io::Error::other)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn start_capture(app: &AppHandle, action: ShortcutAction) {
    let state = app.state::<CaptureState>().0.clone();
    if state.swap(true, Ordering::AcqRel) {
        return;
    }

    let app_handle = app.clone();
    thread::spawn(move || {
        let capture_result = selection::capture_default_selection();
        publish_capture_result(&app_handle, capture_result, action);
        state.store(false, Ordering::Release);
    });
}

fn apply_shortcuts(
    app: &AppHandle,
    bindings: &ShortcutBindings,
    config: &ShortcutConfig,
) -> Result<(), String> {
    let actions = [
        ShortcutAction::OpenPopup,
        ShortcutAction::QuickAction(QuickAction::Summarize),
        ShortcutAction::QuickAction(QuickAction::Explain),
        ShortcutAction::QuickAction(QuickAction::Refine),
    ];
    let parsed = config
        .bindings()
        .into_iter()
        .zip(actions)
        .filter_map(|((name, value), action)| {
            (!value.is_empty())
                .then(|| Shortcut::from_str(value).map(|shortcut| (name, shortcut, action)))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "One of the configured shortcuts is invalid.".to_owned())?;

    app.global_shortcut()
        .unregister_all()
        .map_err(|error| format!("Snippet could not update its shortcuts: {error}"))?;

    let shortcuts = parsed
        .iter()
        .map(|(_, shortcut, _)| *shortcut)
        .collect::<Vec<_>>();
    if let Err(error) = app.global_shortcut().register_multiple(shortcuts) {
        bindings.replace(Vec::new());
        return Err(format!(
            "Snippet could not register the requested shortcut. Another application may already use it: {error}"
        ));
    }

    bindings.replace(
        parsed
            .into_iter()
            .map(|(_, shortcut, action)| (shortcut, action))
            .collect(),
    );
    Ok(())
}

fn publish_capture_result(
    app: &AppHandle,
    capture_result: Result<CapturedSelection, CaptureError>,
    action: ShortcutAction,
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
            let action = match action {
                ShortcutAction::OpenPopup => None,
                ShortcutAction::QuickAction(action) => Some(action),
            };
            let _ = app.emit(
                SELECTION_CAPTURED_EVENT,
                SelectionCaptureEvent { selection, action },
            );
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

    let mut x = cursor.x + POPUP_WINDOW_OFFSET_X;
    let mut y = cursor.y + POPUP_WINDOW_OFFSET_Y;

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
