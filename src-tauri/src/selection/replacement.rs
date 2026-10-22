//! One-use replacement authorization tied to the original window, control and text.
use super::{CaptureContext, CaptureTarget, CapturedSelection, Editability};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

struct Snapshot {
    id: u64,
    target: CaptureTarget,
    text: String,
    focus_id: Vec<i32>,
}
static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(super) fn remember(
    target: CaptureTarget,
    selection: &CapturedSelection,
    context: CaptureContext,
) -> Option<u64> {
    let mut stored = SNAPSHOT.lock().ok()?;
    *stored = None;
    if selection.editability != Editability::Editable
        || context.editability != Editability::Editable
        || selection.truncated
    {
        return None;
    }
    let focus_id = context.focus_id?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    *stored = Some(Snapshot {
        id,
        target,
        text: selection.text.clone(),
        focus_id,
    });
    Some(id)
}

pub fn replace(id: u64, text: &str) -> Result<(), String> {
    if text.trim().is_empty()
        || text.encode_utf16().count() > 32_000
        || text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
    {
        return Err("This response cannot be inserted. Use Copy instead.".into());
    }
    let snapshot = {
        let mut stored = SNAPSHOT.lock().map_err(|_| "Replacement is unavailable.")?;
        if stored.as_ref().map(|snapshot| snapshot.id) != Some(id) {
            return Err(
                "This selection is no longer available. Select the text and open Snippet again."
                    .into(),
            );
        }
        stored.take().unwrap()
    };
    #[cfg(target_os = "windows")]
    {
        snapshot.target.activate().map_err(|e| e.user_message())?;
        super::wait_for_shortcut_release(
            std::time::Instant::now() + super::SHORTCUT_RELEASE_TIMEOUT,
        )
        .map_err(|e| e.user_message())?;
        let (result, context) =
            super::uia::capture(snapshot.target, std::time::Duration::from_millis(600));
        let selection = match result {
            Ok(Some(selection)) => selection,
            Ok(None) | Err(super::CaptureError::CopyRequired)
                if context.editability == Editability::Editable =>
            {
                use super::SelectionReader;
                let mut selection = super::ClipboardCopyReader
                    .capture(snapshot.target, super::CLIPBOARD_WAIT_TIMEOUT)
                    .map_err(|_| changed())?;
                selection.editability = context.editability;
                selection
            }
            _ => return Err(changed()),
        };
        if !matches_snapshot(&snapshot.text, &snapshot.focus_id, &selection, &context) {
            return Err(changed());
        }
        snapshot.target.validate().map_err(|e| e.user_message())?;
        send_text(text)?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = snapshot;
        Err("Replacement is available on Windows only.".into())
    }
}

fn changed() -> String {
    "The original selection changed or is no longer confirmed editable. Select the text and open Snippet again, or use Copy.".into()
}

fn matches_snapshot(
    text: &str,
    focus_id: &[i32],
    selection: &CapturedSelection,
    context: &CaptureContext,
) -> bool {
    selection.editability == Editability::Editable
        && context.editability == Editability::Editable
        && !selection.truncated
        && selection.text == text
        && context.focus_id.as_deref() == Some(focus_id)
}

#[cfg(target_os = "windows")]
fn send_text(text: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    if [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(super::is_key_down)
    {
        return Err("Release the modifier keys, select the text again, and retry.".into());
    }
    // A single Unicode input batch replaces the highlighted text without touching
    // the clipboard or issuing Enter shortcuts that could submit a chat message.
    let normalized = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r");
    let mut inputs = Vec::new();
    for unit in normalized.encode_utf16() {
        for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
            let mut input = super::keyboard_input(0, flags);
            input.Anonymous.ki.wScan = unit;
            inputs.push(input);
        }
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of_val(&inputs[0]) as i32,
        )
    };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err("Windows could not insert the complete response. Check the source text before retrying; Copy is still available.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::SelectionSource;
    #[test]
    #[cfg(target_os = "windows")]
    fn only_complete_editable_identified_captures_get_one_use_tokens() {
        let target = CaptureTarget {
            window: 0,
            process_id: 0,
        };
        let mut selection = CapturedSelection {
            text: "original".into(),
            source: SelectionSource::ClipboardCopy,
            truncated: false,
            clipboard_restored: true,
            bounds: None,
            editability: Editability::Unknown,
            replacement_id: None,
        };
        let context = CaptureContext {
            editability: Editability::Editable,
            focus_id: Some(vec![1, 2, 3]),
        };
        assert!(remember(target, &selection, context.clone()).is_none());
        selection.editability = Editability::ReadOnly;
        assert!(remember(target, &selection, context.clone()).is_none());
        selection.editability = Editability::Editable;
        selection.truncated = true;
        assert!(remember(target, &selection, context.clone()).is_none());
        selection.truncated = false;
        assert!(remember(target, &selection, CaptureContext::default()).is_none());
        let id = remember(target, &selection, context).unwrap();
        assert!(replace(id + 1, "replacement")
            .unwrap_err()
            .contains("no longer available"));
        // An invalid original HWND prevents all input, and consumes the valid token.
        assert!(replace(id, "replacement").is_err());
        assert!(replace(id, "replacement")
            .unwrap_err()
            .contains("no longer available"));
    }

    #[test]
    fn replacement_requires_same_control_exact_text_and_confirmed_editability() {
        let mut selection = CapturedSelection {
            text: "original".into(),
            source: SelectionSource::UiAutomation,
            truncated: false,
            clipboard_restored: true,
            bounds: None,
            editability: Editability::Editable,
            replacement_id: None,
        };
        let mut context = CaptureContext {
            editability: Editability::Editable,
            focus_id: Some(vec![1, 2, 3]),
        };
        assert!(matches_snapshot(
            "original",
            &[1, 2, 3],
            &selection,
            &context
        ));
        assert!(!matches_snapshot(
            "changed",
            &[1, 2, 3],
            &selection,
            &context
        ));
        context.focus_id = Some(vec![1, 2, 4]);
        assert!(!matches_snapshot(
            "original",
            &[1, 2, 3],
            &selection,
            &context
        ));
        context.focus_id = Some(vec![1, 2, 3]);
        for status in [Editability::ReadOnly, Editability::Unknown] {
            selection.editability = status;
            assert!(!matches_snapshot(
                "original",
                &[1, 2, 3],
                &selection,
                &context
            ));
        }
        selection.editability = Editability::Editable;
        context.editability = Editability::Unknown;
        assert!(!matches_snapshot(
            "original",
            &[1, 2, 3],
            &selection,
            &context
        ));
        context.editability = Editability::Editable;
        selection.truncated = true;
        assert!(!matches_snapshot(
            "original",
            &[1, 2, 3],
            &selection,
            &context
        ));
    }
}
