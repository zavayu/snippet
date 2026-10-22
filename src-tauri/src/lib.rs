mod conversation;
pub mod prompt;
pub mod provider;
mod screen;
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
    time::Duration,
};

use arboard::Clipboard;
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "windows")]
use tauri::{PhysicalPosition, PhysicalSize, Runtime, WebviewUrl, WebviewWindowBuilder};
#[cfg(target_os = "windows")]
use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

use crate::conversation::{Conversation, TurnIdentity};
use crate::prompt::ChatMessage;
use crate::{
    prompt::{PromptBuilder, PromptRequest, QuickAction},
    provider::{
        ollama::OllamaProvider, GenerationEvent, GenerationRequest, LlmProvider, ModelInfo,
        OcrGenerationOptions, ProviderError,
    },
    selection::{CaptureError, CapturedSelection},
    settings::{AppConfig, SettingsSnapshot, SettingsStore, ShortcutConfig},
};

const SELECTION_CAPTURED_EVENT: &str = "selection-captured";
const POPUP_OPENED_EVENT: &str = "popup-opened";
const OPEN_SETTINGS_EVENT: &str = "open-settings";
const IMAGE_CAPTURED_EVENT: &str = "image-captured";
const SELECTION_CAPTURE_FAILED_EVENT: &str = "selection-capture-failed";
const GENERATION_STARTED_EVENT: &str = "generation-started";
const GENERATION_CHUNK_EVENT: &str = "generation-chunk";
const GENERATION_FINISHED_EVENT: &str = "generation-finished";
const GENERATION_FAILED_EVENT: &str = "generation-failed";
const GENERATION_CANCELLED_EVENT: &str = "generation-cancelled";
const SCREEN_READ_FINISHED_EVENT: &str = "screen-read-finished";
const SCREEN_READ_FAILED_EVENT: &str = "screen-read-failed";
const REGION_CAPTURE_WINDOW_LABEL: &str = "region-capture";
const POPUP_CONTENT_INSET: i32 = 24;
const POPUP_CURSOR_GAP_X: i32 = 8;
const POPUP_CURSOR_GAP_Y: i32 = 10;
const POPUP_WINDOW_OFFSET_X: i32 = POPUP_CURSOR_GAP_X - POPUP_CONTENT_INSET;
const POPUP_WINDOW_OFFSET_Y: i32 = POPUP_CURSOR_GAP_Y - POPUP_CONTENT_INSET;

#[derive(Default)]
struct CaptureState(Arc<AtomicBool>);

// Serializes installation preparation against new captures and generation.
#[derive(Default)]
struct UpdateInstallState(Mutex<bool>);

#[derive(Default, Clone)]
struct ConversationState(Arc<Mutex<Conversation>>);

impl UpdateInstallState {
    fn prepare(&self, generation: &GenerationState, capture: &CaptureState) -> Result<(), String> {
        let mut installing = self.0.lock().map_err(|_| "Update state is unavailable.")?;
        let active = generation
            .0
            .active
            .lock()
            .map_err(|_| "Generation state is unavailable.")?;
        if *installing || active.is_some() || capture.0.load(Ordering::Acquire) {
            return Err("Wait for the current response, screen reading, or capture to finish before installing.".into());
        }
        *installing = true;
        Ok(())
    }

    fn release(&self) {
        if let Ok(mut installing) = self.0.lock() {
            *installing = false;
        }
    }
}

#[tauri::command]
fn prepare_update_install(
    install: State<UpdateInstallState>,
    generation: State<GenerationState>,
    capture: State<CaptureState>,
) -> Result<(), String> {
    install.prepare(&generation, &capture)
}

#[tauri::command]
fn release_update_install(install: State<UpdateInstallState>) {
    install.release();
}

#[cfg(test)]
mod update_install_tests {
    use super::*;

    #[test]
    fn installation_waits_for_generation_or_ocr_to_finish() {
        let install = UpdateInstallState::default();
        let generation = GenerationState::default();
        let capture = CaptureState::default();
        let (request_id, _) = generation.begin();
        assert!(install.prepare(&generation, &capture).is_err());
        assert!(generation.finish(request_id));
        assert!(install.prepare(&generation, &capture).is_ok());
    }

    #[test]
    fn installation_waits_for_region_capture() {
        let install = UpdateInstallState::default();
        let generation = GenerationState::default();
        let capture = CaptureState::default();
        capture.0.store(true, Ordering::Release);
        assert!(install.prepare(&generation, &capture).is_err());
        capture.0.store(false, Ordering::Release);
        assert!(install.prepare(&generation, &capture).is_ok());
    }

    #[test]
    fn failed_install_can_release_the_reservation_for_retry() {
        let install = UpdateInstallState::default();
        let generation = GenerationState::default();
        let capture = CaptureState::default();
        assert!(install.prepare(&generation, &capture).is_ok());
        assert!(install.prepare(&generation, &capture).is_err());
        install.release();
        assert!(install.prepare(&generation, &capture).is_ok());
    }
}

#[derive(Default)]
struct RegionCaptureStore(Arc<Mutex<Option<PendingRegionCapture>>>);

struct PendingRegionCapture {
    snapshot: screen::MonitorSnapshot,
    auto_read: bool,
}

impl RegionCaptureStore {
    fn start(&self, snapshot: screen::MonitorSnapshot, auto_read: bool) {
        if let Ok(mut active) = self.0.lock() {
            *active = Some(PendingRegionCapture {
                snapshot,
                auto_read,
            });
        }
    }

    fn crop(
        &self,
        region: screen::NormalizedRegion,
    ) -> Result<(screen::CapturedImage, screen::ImagePreview, bool), screen::ScreenCaptureError>
    {
        let mut active = self
            .0
            .lock()
            .map_err(|_| screen::ScreenCaptureError::Capture)?;
        let pending = active
            .as_ref()
            .ok_or(screen::ScreenCaptureError::InvalidRegion)?;
        let (image, preview) = screen::crop_region(&pending.snapshot, region)?;
        let auto_read = pending.auto_read;
        *active = None;
        Ok((image, preview, auto_read))
    }

    fn clear(&self) {
        if let Ok(mut active) = self.0.lock() {
            *active = None;
        }
    }
}

#[derive(Default)]
struct ImageCaptureStore {
    next_id: AtomicU64,
    active: Mutex<Option<StoredImage>>,
}

struct StoredImage {
    id: u64,
    png_base64: String,
}

impl ImageCaptureStore {
    fn store(&self, image: screen::CapturedImage) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut active) = self.active.lock() {
            *active = Some(StoredImage {
                id,
                png_base64: image.png_base64,
            });
        }
        id
    }

    fn get(&self, id: u64) -> Option<String> {
        let active = self.active.lock().ok()?;
        active
            .as_ref()
            .filter(|image| image.id == id)
            .map(|image| image.png_base64.clone())
    }

    fn clear(&self) {
        if let Ok(mut active) = self.active.lock() {
            *active = None;
        }
    }
}

#[derive(Clone, Copy)]
enum ShortcutAction {
    OpenPopup,
    QuickAction(QuickAction),
    CaptureScreen,
    CaptureRegion,
    ReadScreen,
    ReadRegion,
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageCaptureEvent {
    image_id: u64,
    preview: screen::ImagePreview,
    source: ImageCaptureSource,
    auto_read: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ImageCaptureSource {
    Screen,
    Region,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateForSelectionRequest {
    identity: TurnIdentity,
    selected_text: Option<String>,
    image_id: Option<u64>,
    intent: crate::prompt::PromptIntent,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContinueConversationRequest {
    identity: TurnIdentity,
    instruction: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadScreenRequest {
    image_id: u64,
    conversation_id: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationStarted {
    request_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<TurnIdentity>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationChunk {
    request_id: u64,
    delta: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<TurnIdentity>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationFinished {
    request_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<TurnIdentity>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationFailure {
    request_id: u64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<TurnIdentity>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenReadFinished {
    request_id: u64,
    image_id: u64,
    text: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenReadFailure {
    request_id: u64,
    image_id: u64,
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
            prepare_update_install,
            release_update_install,
            get_settings,
            save_settings,
            get_provider_status,
            generate_for_selection,
            continue_conversation,
            reset_conversation,
            read_screen,
            cancel_generation,
            copy_response,
            discard_image_capture,
            complete_region_capture,
            cancel_region_capture,
            preview_prompt,
        ]
    };
}

#[cfg(not(debug_assertions))]
macro_rules! snippet_command_handler {
    () => {
        tauri::generate_handler![
            prepare_update_install,
            release_update_install,
            get_settings,
            save_settings,
            get_provider_status,
            generate_for_selection,
            continue_conversation,
            reset_conversation,
            read_screen,
            cancel_generation,
            copy_response,
            discard_image_capture,
            complete_region_capture,
            cancel_region_capture,
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
    install: State<UpdateInstallState>,
    settings: State<SettingsStore>,
    generation_state: State<GenerationState>,
    image_store: State<ImageCaptureStore>,
    conversation: State<ConversationState>,
    request: GenerateForSelectionRequest,
) -> Result<GenerationStarted, String> {
    let installing = install
        .0
        .lock()
        .map_err(|_| "Update state is unavailable.")?;
    if *installing {
        return Err("Snippet is installing an update. Try again after it restarts.".into());
    }
    let snapshot = settings.snapshot().map_err(|error| error.user_message())?;
    let model = snapshot
        .effective_model
        .ok_or_else(|| "Choose an Ollama model in Settings before generating.".to_owned())?;
    if request.image_id.is_some() && !snapshot.config.vision_enabled {
        return Err("Enable image input in Settings before analyzing a screen capture.".into());
    }
    let image = request
        .image_id
        .map(|image_id| {
            image_store.get(image_id).ok_or_else(|| {
                "That screen capture is no longer available. Capture it again and retry.".to_owned()
            })
        })
        .transpose()?;
    let mut messages = PromptBuilder::build(PromptRequest {
        selected_text: request.selected_text,
        has_image: image.is_some(),
        intent: request.intent,
        context: None,
    })
    .map_err(|error| error.user_message())?;
    if let Some(image) = image {
        if let Some(user_message) = messages
            .iter_mut()
            .find(|message| message.role == crate::prompt::ChatRole::User)
        {
            user_message.images = Some(vec![image]);
        }
    }
    let provider = OllamaProvider::new(&snapshot.config.ollama_base_url)
        .map_err(|error| error.user_message())?;

    let mut session = conversation
        .0
        .lock()
        .map_err(|_| "Conversation is unavailable.")?;
    if session.reset(request.identity.conversation_id) {
        generation_state.cancel();
    }
    let messages = session.begin(request.identity, Some(messages), None)?;
    Ok(stream_conversation(
        app,
        (*generation_state).clone(),
        (*conversation).clone(),
        provider,
        GenerationRequest {
            model,
            messages,
            thinking: snapshot.config.thinking,
        },
        request.identity,
    ))
}

#[tauri::command]
fn continue_conversation(
    app: AppHandle,
    install: State<UpdateInstallState>,
    settings: State<SettingsStore>,
    generation_state: State<GenerationState>,
    conversation: State<ConversationState>,
    request: ContinueConversationRequest,
) -> Result<GenerationStarted, String> {
    let installing = install
        .0
        .lock()
        .map_err(|_| "Update state is unavailable.")?;
    if *installing {
        return Err("Snippet is installing an update. Try again after it restarts.".into());
    }
    let snapshot = settings.snapshot().map_err(|error| error.user_message())?;
    let model = snapshot
        .effective_model
        .ok_or_else(|| "Choose an Ollama model in Settings before generating.".to_owned())?;
    let provider = OllamaProvider::new(&snapshot.config.ollama_base_url)
        .map_err(|error| error.user_message())?;
    let mut session = conversation
        .0
        .lock()
        .map_err(|_| "Conversation is unavailable.")?;
    let messages = session.begin(request.identity, None, Some(request.instruction))?;
    Ok(stream_conversation(
        app,
        (*generation_state).clone(),
        (*conversation).clone(),
        provider,
        GenerationRequest {
            model,
            messages,
            thinking: snapshot.config.thinking,
        },
        request.identity,
    ))
}

#[tauri::command]
fn reset_conversation(
    conversation_id: u64,
    conversation: State<ConversationState>,
    generation: State<GenerationState>,
) -> Result<(), String> {
    let mut session = conversation
        .0
        .lock()
        .map_err(|_| "Conversation is unavailable.")?;
    if session.reset(conversation_id) {
        generation.cancel();
    }
    Ok(())
}

fn stream_conversation(
    app: AppHandle,
    generation_state: GenerationState,
    conversation: ConversationState,
    provider: OllamaProvider,
    request: GenerationRequest,
    identity: TurnIdentity,
) -> GenerationStarted {
    let (request_id, cancellation) = generation_state.begin();
    let context = Some(identity);
    let started = GenerationStarted {
        request_id,
        context,
    };
    let _ = app.emit(GENERATION_STARTED_EVENT, started.clone());

    let app_handle = app.clone();
    let state = generation_state;
    tauri::async_runtime::spawn(async move {
        let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
        let provider_task = tauri::async_runtime::spawn(async move {
            provider
                .stream_generation(request, cancellation, event_sender)
                .await
        });

        let mut response = String::new();
        while let Some(event) = event_receiver.recv().await {
            if !state.is_active(request_id) {
                break;
            }

            match event {
                GenerationEvent::Delta(delta) => {
                    response.push_str(&delta);
                    let _ = app_handle.emit(
                        GENERATION_CHUNK_EVENT,
                        GenerationChunk {
                            request_id,
                            delta,
                            context,
                        },
                    );
                }
            }
        }

        let mut result = match provider_task.await {
            Ok(result) => result,
            Err(error) => Err(ProviderError::RequestFailed(error.to_string())),
        };
        if !state.finish(request_id) {
            return;
        }
        if result.is_ok() && response.trim().is_empty() {
            result = Err(ProviderError::InvalidResponse(
                "Empty assistant response".into(),
            ));
        }
        let Ok(mut session) = conversation.0.lock() else {
            return;
        };
        if !session.finish(identity, result.is_ok().then_some(response)) {
            return;
        }
        drop(session);

        match result {
            Ok(()) => {
                let _ = app_handle.emit(
                    GENERATION_FINISHED_EVENT,
                    GenerationFinished {
                        request_id,
                        context,
                    },
                );
            }
            Err(ProviderError::Cancelled) => {
                let _ = app_handle.emit(
                    GENERATION_CANCELLED_EVENT,
                    GenerationFinished {
                        request_id,
                        context,
                    },
                );
            }
            Err(error) => {
                let _ = app_handle.emit(
                    GENERATION_FAILED_EVENT,
                    GenerationFailure {
                        request_id,
                        message: error.user_message(),
                        context,
                    },
                );
            }
        }
    });

    started
}

#[tauri::command]
fn read_screen(
    app: AppHandle,
    install: State<UpdateInstallState>,
    settings: State<SettingsStore>,
    generation_state: State<GenerationState>,
    image_store: State<ImageCaptureStore>,
    conversation: State<ConversationState>,
    request: ReadScreenRequest,
) -> Result<GenerationStarted, String> {
    let installing = install
        .0
        .lock()
        .map_err(|_| "Update state is unavailable.")?;
    if *installing {
        return Err("Snippet is installing an update. Try again after it restarts.".into());
    }
    let snapshot = settings.snapshot().map_err(|error| error.user_message())?;
    let image = image_store.get(request.image_id).ok_or_else(|| {
        "That screen capture is no longer available. Capture it again and retry.".to_owned()
    })?;
    let provider = OllamaProvider::new(&snapshot.config.ollama_base_url)
        .map_err(|error| error.user_message())?;
    let ocr_options = OcrGenerationOptions {
        num_predict: snapshot.config.ocr_num_predict,
        num_ctx: snapshot.config.ocr_num_ctx,
    };
    let model = snapshot.config.ocr_model;
    let model_for_error = model.clone();

    let mut session = conversation
        .0
        .lock()
        .map_err(|_| "Conversation is unavailable.")?;
    if session.reset(request.conversation_id) {
        generation_state.cancel();
    }
    session.ensure_current(request.conversation_id)?;
    let (request_id, cancellation) = generation_state.begin();
    drop(session);
    let started = GenerationStarted {
        request_id,
        context: None,
    };

    let app_handle = app.clone();
    let state = (*generation_state).clone();
    tauri::async_runtime::spawn(async move {
        let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
        let provider_task = tauri::async_runtime::spawn(async move {
            provider
                .stream_ocr(model, image, ocr_options, cancellation, event_sender)
                .await
        });

        let mut extracted_text = String::new();
        while let Some(event) = event_receiver.recv().await {
            if !state.is_active(request_id) {
                break;
            }

            match event {
                GenerationEvent::Delta(delta) => extracted_text.push_str(&delta),
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
            Ok(()) if !extracted_text.trim().is_empty() => {
                let _ = app_handle.emit(
                    SCREEN_READ_FINISHED_EVENT,
                    ScreenReadFinished {
                        request_id,
                        image_id: request.image_id,
                        text: extracted_text,
                    },
                );
            }
            Ok(()) => {
                let _ = app_handle.emit(
                    SCREEN_READ_FAILED_EVENT,
                    ScreenReadFailure {
                        request_id,
                        image_id: request.image_id,
                        message: "The OCR model did not return readable text. Try a smaller region or another OCR model.".into(),
                    },
                );
            }
            Err(ProviderError::Cancelled) => {
                // A new capture or closing the popup cancels OCR. Neither needs a visible error.
            }
            Err(ProviderError::RequestFailed(message))
                if message == "The selected Ollama model is not installed." =>
            {
                let _ = app_handle.emit(
                    SCREEN_READ_FAILED_EVENT,
                    ScreenReadFailure {
                        request_id,
                        image_id: request.image_id,
                        message: format!(
                            "The OCR model '{model_for_error}' is not installed. Run `ollama pull {model_for_error}`, or choose another OCR model in Settings."
                        ),
                    },
                );
            }
            Err(error) => {
                let _ = app_handle.emit(
                    SCREEN_READ_FAILED_EVENT,
                    ScreenReadFailure {
                        request_id,
                        image_id: request.image_id,
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

#[tauri::command]
fn discard_image_capture(image_store: State<ImageCaptureStore>) {
    image_store.clear();
}

#[tauri::command]
fn complete_region_capture(
    app: AppHandle,
    capture_state: State<CaptureState>,
    region_store: State<RegionCaptureStore>,
    image_store: State<ImageCaptureStore>,
    region: screen::NormalizedRegion,
) -> Result<(), String> {
    let (image, preview, auto_read) = region_store
        .crop(region)
        .map_err(|error| error.user_message())?;
    let image_id = image_store.store(image);
    close_region_overlay(&app);
    show_popup(&app);
    let _ = app.emit(
        IMAGE_CAPTURED_EVENT,
        ImageCaptureEvent {
            image_id,
            preview,
            source: ImageCaptureSource::Region,
            auto_read,
        },
    );
    capture_state.0.store(false, Ordering::Release);
    Ok(())
}

#[tauri::command]
fn cancel_region_capture(
    app: AppHandle,
    capture_state: State<CaptureState>,
    region_store: State<RegionCaptureStore>,
) {
    region_store.clear();
    close_region_overlay(&app);
    capture_state.0.store(false, Ordering::Release);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_popup(app);
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(UpdateInstallState::default())
        .manage(CaptureState::default())
        .manage(RegionCaptureStore::default())
        .manage(ImageCaptureStore::default())
        .manage(GenerationState::default())
        .manage(ConversationState::default())
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
            configure_main_window(&app.handle());
            build_tray(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Snippet", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let updates = MenuItem::with_id(app, "updates", "Check for updates…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Snippet", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &settings, &updates, &quit])?;
    let icon = tauri::include_image!("icons/tray.png");

    let _tray = TrayIconBuilder::with_id("snippet-tray")
        .icon(icon)
        .tooltip("Snippet")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => open_popup(app),
            "settings" => {
                open_popup(app);
                let _ = app.emit(OPEN_SETTINGS_EVENT, ());
            }
            "quit" => app.exit(0),
            "updates" => {
                show_popup(app);
                let _ = app.emit("open-updates", ());
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

fn configure_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let window_for_events = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window_for_events.hide();
        }
    });
}

fn start_capture(app: &AppHandle, action: ShortcutAction) {
    let install = app.state::<UpdateInstallState>();
    let Ok(installing) = install.0.lock() else {
        return;
    };
    if *installing {
        return;
    }
    let state = app.state::<CaptureState>().0.clone();
    if state.swap(true, Ordering::AcqRel) {
        return;
    }

    let app_handle = app.clone();
    match action {
        ShortcutAction::CaptureScreen | ShortcutAction::ReadScreen => {
            let auto_read = matches!(action, ShortcutAction::ReadScreen);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            thread::spawn(move || {
                // Give the compositor a moment to remove Snippet before taking the image.
                thread::sleep(Duration::from_millis(75));
                let capture_result = screen::capture_current_monitor();
                publish_image_capture_result(
                    &app_handle,
                    capture_result,
                    ImageCaptureSource::Screen,
                    auto_read,
                );
                state.store(false, Ordering::Release);
            });
        }
        ShortcutAction::CaptureRegion | ShortcutAction::ReadRegion => {
            let auto_read = matches!(action, ShortcutAction::ReadRegion);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            thread::spawn(move || {
                // Capture before showing the overlay so the overlay is never part of the image.
                let capture_result = screen::capture_current_monitor_snapshot();
                match capture_result {
                    Ok(snapshot) => {
                        let callback_app = app_handle.clone();
                        let callback_state = state.clone();
                        let opened = app_handle.run_on_main_thread(move || {
                            callback_app
                                .state::<RegionCaptureStore>()
                                .start(snapshot, auto_read);
                            if let Err(message) = open_region_overlay(&callback_app) {
                                callback_app.state::<RegionCaptureStore>().clear();
                                publish_capture_failure(&callback_app, message);
                                callback_state.store(false, Ordering::Release);
                            }
                        });
                        if opened.is_err() {
                            publish_capture_failure(
                                &app_handle,
                                "Snippet could not open the region selector. Try again.".into(),
                            );
                            state.store(false, Ordering::Release);
                        }
                    }
                    Err(error) => {
                        publish_capture_failure(&app_handle, error.user_message());
                        state.store(false, Ordering::Release);
                    }
                }
            });
        }
        action => {
            thread::spawn(move || {
                let capture_result = selection::capture_default_selection();
                publish_capture_result(&app_handle, capture_result, action);
                state.store(false, Ordering::Release);
            });
        }
    };
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
        ShortcutAction::CaptureScreen,
        ShortcutAction::CaptureRegion,
        ShortcutAction::ReadScreen,
        ShortcutAction::ReadRegion,
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
    app.state::<ImageCaptureStore>().clear();
    show_popup(app);

    match capture_result {
        Ok(selection) => {
            let action = match action {
                ShortcutAction::OpenPopup => None,
                ShortcutAction::QuickAction(action) => Some(action),
                ShortcutAction::CaptureScreen => None,
                ShortcutAction::CaptureRegion => None,
                ShortcutAction::ReadScreen => None,
                ShortcutAction::ReadRegion => None,
            };
            let _ = app.emit(
                SELECTION_CAPTURED_EVENT,
                SelectionCaptureEvent { selection, action },
            );
        }
        Err(CaptureError::NoSelection | CaptureError::TimedOut)
            if matches!(action, ShortcutAction::OpenPopup) =>
        {
            let _ = app.emit(POPUP_OPENED_EVENT, ());
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

fn publish_image_capture_result(
    app: &AppHandle,
    capture_result: Result<
        (screen::CapturedImage, screen::ImagePreview),
        screen::ScreenCaptureError,
    >,
    source: ImageCaptureSource,
    auto_read: bool,
) {
    show_popup(app);

    match capture_result {
        Ok((image, preview)) => {
            let image_id = app.state::<ImageCaptureStore>().store(image);
            let _ = app.emit(
                IMAGE_CAPTURED_EVENT,
                ImageCaptureEvent {
                    image_id,
                    preview,
                    source,
                    auto_read,
                },
            );
        }
        Err(error) => {
            publish_capture_failure(app, error.user_message());
        }
    }
}

fn publish_capture_failure(app: &AppHandle, message: String) {
    show_popup(app);
    let _ = app.emit(SELECTION_CAPTURE_FAILED_EVENT, CaptureFailure { message });
}

#[cfg(target_os = "windows")]
fn open_region_overlay(app: &AppHandle) -> Result<(), String> {
    let region_store = app.state::<RegionCaptureStore>();
    let (x, y, width, height) = {
        let active = region_store
            .0
            .lock()
            .map_err(|_| "Snippet could not prepare the region selector.".to_owned())?;
        let pending = active
            .as_ref()
            .ok_or_else(|| "Snippet could not prepare the region selector.".to_owned())?;
        let snapshot = &pending.snapshot;
        (snapshot.x, snapshot.y, snapshot.width, snapshot.height)
    };

    if let Some(existing) = app.get_webview_window(REGION_CAPTURE_WINDOW_LABEL) {
        let _ = existing.close();
    }

    let overlay = WebviewWindowBuilder::new(
        app,
        REGION_CAPTURE_WINDOW_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("Select a region")
    .inner_size(1.0, 1.0)
    .visible(false)
    .focused(true)
    .decorations(false)
    .resizable(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .build()
    .map_err(|error| format!("Snippet could not open the region selector: {error}"))?;

    overlay
        .set_size(PhysicalSize::new(width, height))
        .map_err(|error| format!("Snippet could not size the region selector: {error}"))?;
    overlay
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| format!("Snippet could not position the region selector: {error}"))?;
    overlay
        .show()
        .map_err(|error| format!("Snippet could not show the region selector: {error}"))?;
    overlay
        .set_focus()
        .map_err(|error| format!("Snippet could not focus the region selector: {error}"))?;

    let region_store = region_store.0.clone();
    let capture_state = app.state::<CaptureState>().0.clone();
    overlay.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            if let Ok(mut active) = region_store.lock() {
                *active = None;
            }
            capture_state.store(false, Ordering::Release);
        }
    });
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn open_region_overlay(_app: &AppHandle) -> Result<(), String> {
    Err("Region capture is currently available on Windows only.".into())
}

fn close_region_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(REGION_CAPTURE_WINDOW_LABEL) {
        let _ = window.close();
    }
}

fn open_popup(app: &AppHandle) {
    show_popup(app);
    let _ = app.emit(POPUP_OPENED_EVENT, ());
}

fn show_popup(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        position_popup_near_cursor(&window);

        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
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
