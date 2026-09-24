//! OS-facing text selection capture.
//!
//! The UI only receives a `CapturedSelection` or a user-facing failure. That
//! separation lets a future UI Automation reader replace or precede the
//! clipboard-based reader without changing the hotkey workflow or frontend.

use std::time::{Duration, Instant};

use arboard::Clipboard;
use serde::Serialize;

#[cfg(target_os = "windows")]
use std::{mem::size_of, thread};

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    System::DataExchange::GetClipboardSequenceNumber,
    UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        VK_C, VK_CONTROL, VK_SHIFT,
    },
    UI::WindowsAndMessaging::GetForegroundWindow,
};

const MAX_SELECTION_CHARS: usize = 20_000;
const CLIPBOARD_WAIT_TIMEOUT: Duration = Duration::from_millis(1_200);
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(15);
const COPY_RETRY_DELAY: Duration = Duration::from_millis(350);
const SHORTCUT_RELEASE_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SelectionSource {
    ClipboardCopy,
    // Reserved for the future `UiAutomationReader` implementation.
    #[allow(dead_code)]
    UiAutomation,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedSelection {
    pub text: String,
    pub source: SelectionSource,
    pub truncated: bool,
    pub clipboard_restored: bool,
}

#[derive(Debug)]
pub enum CaptureError {
    NoSelection,
    TimedOut,
    ClipboardUnavailable,
    InputInjectionFailed,
    ShortcutStillPressed,
    FocusChanged,
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    UnsupportedPlatform,
}

impl CaptureError {
    pub fn user_message(&self) -> String {
        match self {
            Self::NoSelection => "No text was copied. Select text in another application and try again.".into(),
            Self::TimedOut => "Snippet did not receive copied text in time. Try selecting text again.".into(),
            Self::ClipboardUnavailable => {
                "Snippet could not access the clipboard. Another application may be using it.".into()
            }
            Self::InputInjectionFailed => {
                "Snippet could not send Copy to the active application. It may be running with elevated permissions.".into()
            }
            Self::ShortcutStillPressed => {
                "Release the shortcut keys, then try again.".into()
            }
            Self::FocusChanged => {
                "The active application changed before Snippet could copy the selection. Try again.".into()
            }
            Self::UnsupportedPlatform => "Text capture is currently available on Windows only.".into(),
        }
    }
}

/// A source of selected text. The clipboard reader is the MVP implementation;
/// a UI Automation reader can implement this same contract later.
pub trait SelectionReader {
    fn capture(&self) -> Result<CapturedSelection, CaptureError>;
}

pub struct ClipboardCopyReader;

impl SelectionReader for ClipboardCopyReader {
    fn capture(&self) -> Result<CapturedSelection, CaptureError> {
        capture_from_clipboard_copy()
    }
}

pub fn capture_default_selection() -> Result<CapturedSelection, CaptureError> {
    ClipboardCopyReader.capture()
}

#[cfg(target_os = "windows")]
fn capture_from_clipboard_copy() -> Result<CapturedSelection, CaptureError> {
    let mut clipboard = Clipboard::new().map_err(|_| CaptureError::ClipboardUnavailable)?;
    let previous_plain_text = clipboard.get_text().ok();

    wait_for_shortcut_release()?;
    let target_window = foreground_window();
    if target_window.is_null() {
        return Err(CaptureError::InputInjectionFailed);
    }

    // Take this snapshot immediately before Copy. The previous implementation
    // took it while the user was still releasing the global shortcut.
    let sequence_before_copy = clipboard_sequence_number();
    send_copy_shortcut()?;

    let deadline = Instant::now() + CLIPBOARD_WAIT_TIMEOUT;
    let retry_at = Instant::now() + COPY_RETRY_DELAY;
    let mut retried_copy = false;

    loop {
        thread::sleep(CLIPBOARD_POLL_INTERVAL);
        let sequence_after_copy = clipboard_sequence_number();

        if sequence_after_copy != sequence_before_copy {
            let copied_text = clipboard
                .get_text()
                .map_err(|_| CaptureError::NoSelection)?;
            let (text, truncated) = normalize_selection(copied_text);

            if text.is_empty() {
                return Err(CaptureError::NoSelection);
            }

            let clipboard_restored = restore_plain_text_if_unchanged(
                &mut clipboard,
                previous_plain_text,
                sequence_after_copy,
            );

            return Ok(CapturedSelection {
                text,
                source: SelectionSource::ClipboardCopy,
                truncated,
                clipboard_restored,
            });
        }

        if !retried_copy && Instant::now() >= retry_at {
            // Do not issue another Copy once the user has switched apps; that
            // could place unrelated content on the clipboard.
            if foreground_window() != target_window {
                return Err(CaptureError::FocusChanged);
            }

            send_copy_shortcut()?;
            retried_copy = true;
        }

        if Instant::now() >= deadline {
            return Err(CaptureError::TimedOut);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn capture_from_clipboard_copy() -> Result<CapturedSelection, CaptureError> {
    Err(CaptureError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
fn wait_for_shortcut_release() -> Result<(), CaptureError> {
    let deadline = Instant::now() + SHORTCUT_RELEASE_TIMEOUT;

    while is_key_down(VK_CONTROL) || is_key_down(VK_SHIFT) {
        if Instant::now() >= deadline {
            return Err(CaptureError::ShortcutStillPressed);
        }
        thread::sleep(CLIPBOARD_POLL_INTERVAL);
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn is_key_down(key: u16) -> bool {
    // The high-order bit indicates whether the key is currently down.
    unsafe { GetAsyncKeyState(key as i32) < 0 }
}

#[cfg(target_os = "windows")]
fn send_copy_shortcut() -> Result<(), CaptureError> {
    let mut inputs = [
        keyboard_input(VK_CONTROL, 0),
        keyboard_input(VK_C, 0),
        keyboard_input(VK_C, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];

    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            size_of::<INPUT>() as i32,
        )
    };

    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(CaptureError::InputInjectionFailed)
    }
}

#[cfg(target_os = "windows")]
fn keyboard_input(key: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(target_os = "windows")]
fn clipboard_sequence_number() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

#[cfg(target_os = "windows")]
fn foreground_window() -> *mut core::ffi::c_void {
    unsafe { GetForegroundWindow() }
}

#[cfg(target_os = "windows")]
fn restore_plain_text_if_unchanged(
    clipboard: &mut Clipboard,
    previous_plain_text: Option<String>,
    copied_sequence_number: u32,
) -> bool {
    let Some(previous_plain_text) = previous_plain_text else {
        return false;
    };

    if clipboard_sequence_number() != copied_sequence_number {
        return false;
    }

    clipboard.set_text(previous_plain_text).is_ok()
}

fn normalize_selection(text: String) -> (String, bool) {
    let trimmed = text.trim().to_owned();
    let character_count = trimmed.chars().count();

    if character_count <= MAX_SELECTION_CHARS {
        return (trimmed, false);
    }

    let truncated = trimmed.chars().take(MAX_SELECTION_CHARS).collect();
    (truncated, true)
}

#[cfg(test)]
mod tests {
    use super::{normalize_selection, MAX_SELECTION_CHARS};

    #[test]
    fn normalizes_surrounding_whitespace() {
        assert_eq!(
            normalize_selection("  selected text\n".into()),
            ("selected text".into(), false)
        );
    }

    #[test]
    fn truncates_very_large_selections() {
        let input = "x".repeat(MAX_SELECTION_CHARS + 1);
        let (text, truncated) = normalize_selection(input);

        assert_eq!(text.chars().count(), MAX_SELECTION_CHARS);
        assert!(truncated);
    }
}
