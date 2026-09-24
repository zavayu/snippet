mod selection;

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[cfg(target_os = "windows")]
use tauri::{PhysicalPosition, Runtime};
#[cfg(target_os = "windows")]
use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

use crate::selection::{CaptureError, CapturedSelection};

const SELECTION_CAPTURED_EVENT: &str = "selection-captured";
const SELECTION_CAPTURE_FAILED_EVENT: &str = "selection-capture-failed";
const POPUP_CURSOR_OFFSET_X: i32 = 14;
const POPUP_CURSOR_OFFSET_Y: i32 = 18;

#[derive(Default)]
struct CaptureState(Arc<AtomicBool>);

#[derive(Clone, Serialize)]
struct CaptureFailure {
    message: String,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let trigger_shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);
    let registered_shortcut =
        Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);

    tauri::Builder::default()
        .manage(CaptureState::default())
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
        .setup(move |app| {
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

    // Keep the popup inside the work area (away from a monitor's taskbar) when
    // it would otherwise extend off the right or bottom edge.
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
