//! OS-facing text selection capture.
//!
//! The UI only receives a `CapturedSelection` or a user-facing failure.
//! Windows UI Automation reads selections without touching the clipboard.
//! Unsupported providers fall back to the existing clipboard reader.

#[cfg(target_os = "windows")]
mod uia;

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
    UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
    },
};

const MAX_SELECTION_CHARS: usize = 20_000;
const CLIPBOARD_WAIT_TIMEOUT: Duration = Duration::from_millis(1_200);
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(15);
const COPY_RETRY_DELAY: Duration = Duration::from_millis(350);
const SHORTCUT_RELEASE_TIMEOUT: Duration = Duration::from_millis(300);
const POPUP_CAPTURE_BUDGET: Duration = Duration::from_millis(120);
const POPUP_COPY_BUDGET: Duration = Duration::from_millis(180);
const ACTION_UIA_BUDGET: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SelectionSource {
    ClipboardCopy,
    UiAutomation,
    LegacyAccessibility,
}

/// Physical screen coordinates, including negative origins on secondary monitors.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct SelectionRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct SelectionBounds {
    pub first: SelectionRect,
    pub last: SelectionRect,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedSelection {
    pub text: String,
    pub source: SelectionSource,
    pub truncated: bool,
    pub clipboard_restored: bool,
    pub bounds: Option<SelectionBounds>,
}

#[derive(Debug)]
pub enum CaptureError {
    NoSelection,
    CopyRequired,
    CopyUnavailable,
    ProtectedField,
    NonTextSelection,
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
            Self::NoSelection => "No text is selected. Select text in another application and try again.".into(),
            Self::CopyRequired => "This editor needs a copy attempt to read its selection.".into(),
            Self::CopyUnavailable => "The app did not provide selected text when asked to copy.".into(),
            Self::ProtectedField => "This protected field does not allow selection capture.".into(),
            Self::NonTextSelection => "Select text rather than a message, file, or other interface item.".into(),
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
                "The active application changed before Snippet could capture the selection. Try again.".into()
            }
            Self::UnsupportedPlatform => "Text capture is currently available on Windows only.".into(),
        }
    }
}

/// An invocation's original foreground window. Store the handle as an integer
/// so the snapshot can be sent to the capture worker without sharing COM objects.
#[derive(Clone, Copy)]
pub struct CaptureTarget {
    #[cfg(target_os = "windows")]
    window: usize,
    #[cfg(target_os = "windows")]
    process_id: u32,
}

pub fn capture_target() -> Result<CaptureTarget, CaptureError> {
    #[cfg(target_os = "windows")]
    {
        let window = foreground_window();
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if window.is_null() || process_id == 0 {
            return Err(CaptureError::FocusChanged);
        }
        Ok(CaptureTarget {
            window: window as usize,
            process_id,
        })
    }
    #[cfg(not(target_os = "windows"))]
    Err(CaptureError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
impl CaptureTarget {
    fn validate(self) -> Result<(), CaptureError> {
        let current = capture_target()?;
        if current.window != self.window || current.process_id != self.process_id {
            return Err(CaptureError::FocusChanged);
        }
        Ok(())
    }

    pub fn activate(self) -> Result<(), CaptureError> {
        let window = self.window as *mut core::ffi::c_void;
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if unsafe { IsWindow(window) } == 0 || process_id != self.process_id {
            return Err(CaptureError::FocusChanged);
        }
        unsafe { SetForegroundWindow(window) };
        let deadline = Instant::now() + Duration::from_millis(150);
        loop {
            if self.validate().is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(CaptureError::FocusChanged);
            }
            thread::sleep(CLIPBOARD_POLL_INTERVAL);
        }
    }
}

#[cfg(not(target_os = "windows"))]
impl CaptureTarget {
    pub fn activate(self) -> Result<(), CaptureError> {
        Err(CaptureError::UnsupportedPlatform)
    }
}

/// A source of selected text for the original foreground window.
pub trait SelectionReader {
    fn capture(
        &self,
        target: CaptureTarget,
        budget: Duration,
    ) -> Result<CapturedSelection, CaptureError>;
}

pub struct ClipboardCopyReader;

impl SelectionReader for ClipboardCopyReader {
    fn capture(
        &self,
        target: CaptureTarget,
        budget: Duration,
    ) -> Result<CapturedSelection, CaptureError> {
        capture_from_clipboard_copy(target, budget)
    }
}

pub fn capture_default_selection(target: CaptureTarget) -> Result<CapturedSelection, CaptureError> {
    capture_selection(target, CapturePolicy::Required)
}

/// Opening the composer uses a brief accessibility check and, when inconclusive,
/// a short copy attempt instead of requiring the user to retry manually.
pub fn capture_for_popup(target: CaptureTarget) -> Result<CapturedSelection, CaptureError> {
    capture_selection(target, CapturePolicy::BestEffort)
}

#[derive(Clone, Copy)]
enum CapturePolicy {
    BestEffort,
    Required,
}

fn capture_selection(
    target: CaptureTarget,
    policy: CapturePolicy,
) -> Result<CapturedSelection, CaptureError> {
    #[cfg(target_os = "windows")]
    {
        target.validate()?;
        let captured = capture_with_policy(
            policy,
            |budget| uia::capture(target, budget),
            |budget| ClipboardCopyReader.capture(target, budget),
        )?;
        target.validate()?;
        Ok(captured)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (target, policy);
        Err(CaptureError::UnsupportedPlatform)
    }
}

fn capture_with_policy(
    policy: CapturePolicy,
    probe: impl FnOnce(Duration) -> Result<Option<CapturedSelection>, CaptureError>,
    copy: impl FnOnce(Duration) -> Result<CapturedSelection, CaptureError>,
) -> Result<CapturedSelection, CaptureError> {
    let budget = match policy {
        CapturePolicy::BestEffort => POPUP_CAPTURE_BUDGET,
        CapturePolicy::Required => ACTION_UIA_BUDGET,
    };
    match probe(budget) {
        Ok(Some(selection)) => Ok(selection),
        Ok(None) | Err(CaptureError::CopyRequired) => copy(match policy {
            CapturePolicy::BestEffort => POPUP_COPY_BUDGET,
            CapturePolicy::Required => SHORTCUT_RELEASE_TIMEOUT + CLIPBOARD_WAIT_TIMEOUT,
        }),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "windows")]
fn capture_from_clipboard_copy(
    target: CaptureTarget,
    budget: Duration,
) -> Result<CapturedSelection, CaptureError> {
    let deadline = Instant::now() + budget;
    target.validate()?;
    wait_for_shortcut_release(deadline)?;
    target.validate()?;

    let mut clipboard = Clipboard::new().map_err(|_| CaptureError::ClipboardUnavailable)?;
    let previous_plain_text = clipboard.get_text().ok();
    target.validate()?;

    // Take this snapshot immediately before Copy. The previous implementation
    // took it while the user was still releasing the global shortcut.
    let sequence_before_copy = clipboard_sequence_number();
    if Instant::now() >= deadline {
        return Err(CaptureError::TimedOut);
    }
    send_copy_shortcut()?;

    let deadline = deadline.min(Instant::now() + CLIPBOARD_WAIT_TIMEOUT);
    let retry_at = Instant::now() + COPY_RETRY_DELAY;
    let mut retried_copy = false;

    loop {
        thread::sleep(
            CLIPBOARD_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
        );
        let sequence_after_copy = clipboard_sequence_number();
        target.validate()?;

        if sequence_after_copy != sequence_before_copy {
            let copied_text = clipboard
                .get_text()
                .map_err(|_| CaptureError::CopyUnavailable)?;
            let (text, truncated) = normalize_selection(copied_text);

            let clipboard_restored = restore_plain_text_if_unchanged(
                &mut clipboard,
                previous_plain_text,
                sequence_after_copy,
            );

            if text.trim().is_empty() {
                return Err(CaptureError::CopyUnavailable);
            }

            return Ok(CapturedSelection {
                text,
                source: SelectionSource::ClipboardCopy,
                truncated,
                clipboard_restored,
                bounds: None,
            });
        }

        if !retried_copy && Instant::now() >= retry_at {
            // Do not issue another Copy once the user has switched apps; that
            // could place unrelated content on the clipboard.
            target.validate()?;

            send_copy_shortcut()?;
            retried_copy = true;
        }

        if Instant::now() >= deadline {
            return Err(CaptureError::TimedOut);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn capture_from_clipboard_copy(
    _target: CaptureTarget,
    _budget: Duration,
) -> Result<CapturedSelection, CaptureError> {
    Err(CaptureError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
fn wait_for_shortcut_release(deadline: Instant) -> Result<(), CaptureError> {
    let deadline = deadline.min(Instant::now() + SHORTCUT_RELEASE_TIMEOUT);

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
    let character_count = text.chars().count();

    if character_count <= MAX_SELECTION_CHARS {
        return (text, false);
    }

    let truncated = text.chars().take(MAX_SELECTION_CHARS).collect();
    (truncated, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected_text() -> CapturedSelection {
        CapturedSelection {
            text: "selected".into(),
            source: SelectionSource::ClipboardCopy,
            truncated: false,
            clipboard_restored: true,
            bounds: None,
        }
    }

    #[test]
    fn opening_popup_captures_unknown_and_virtual_editor_selections_on_first_attempt() {
        for probe_result in [Ok(None), Err(CaptureError::CopyRequired)] {
            let result = capture_with_policy(
                CapturePolicy::BestEffort,
                |budget| {
                    assert_eq!(budget, Duration::from_millis(120));
                    probe_result
                },
                |budget| {
                    assert_eq!(budget, Duration::from_millis(180));
                    Ok(selected_text())
                },
            );
            assert_eq!(result.unwrap().text, "selected");
        }
    }

    #[test]
    fn explicit_actions_and_retries_copy_when_accessibility_is_inconclusive() {
        for probe_result in [Ok(None), Err(CaptureError::CopyRequired)] {
            let result = capture_with_policy(
                CapturePolicy::Required,
                |budget| {
                    assert_eq!(budget, ACTION_UIA_BUDGET);
                    probe_result
                },
                |budget| {
                    assert_eq!(budget, SHORTCUT_RELEASE_TIMEOUT + CLIPBOARD_WAIT_TIMEOUT);
                    Ok(selected_text())
                },
            );
            assert_eq!(result.unwrap().text, "selected");
        }
    }

    #[test]
    fn confirmed_empty_and_capture_failures_are_distinct_and_never_inject_copy() {
        for policy in [CapturePolicy::BestEffort, CapturePolicy::Required] {
            for error in [
                CaptureError::NoSelection,
                CaptureError::FocusChanged,
                CaptureError::ProtectedField,
                CaptureError::NonTextSelection,
            ] {
                let expected = std::mem::discriminant(&error);
                let actual = capture_with_policy(
                    policy,
                    |_| Err(error),
                    |_| panic!("Known empty or invalid targets must not send Copy"),
                );
                assert_eq!(std::mem::discriminant(&actual.unwrap_err()), expected);
            }
        }
    }

    #[test]
    fn successful_accessibility_capture_never_uses_clipboard() {
        for policy in [CapturePolicy::BestEffort, CapturePolicy::Required] {
            let result = capture_with_policy(
                policy,
                |_| Ok(Some(selected_text())),
                |_| panic!("Selected text is already available"),
            );
            assert_eq!(result.unwrap().text, "selected");
        }
    }

    #[test]
    fn preserves_code_indentation_and_trailing_newline() {
        assert_eq!(
            normalize_selection("  selected text\n".into()),
            ("  selected text\n".into(), false)
        );
    }

    #[test]
    fn popup_copy_timeout_remains_inconclusive_instead_of_claiming_no_selection() {
        let result = capture_with_policy(
            CapturePolicy::BestEffort,
            |_| Ok(None),
            |_| Err(CaptureError::TimedOut),
        );
        assert!(matches!(result, Err(CaptureError::TimedOut)));
    }

    #[test]
    fn truncates_very_large_selections() {
        let input = "x".repeat(MAX_SELECTION_CHARS + 1);
        let (text, truncated) = normalize_selection(input);

        assert_eq!(text.chars().count(), MAX_SELECTION_CHARS);
        assert!(truncated);
    }
}
