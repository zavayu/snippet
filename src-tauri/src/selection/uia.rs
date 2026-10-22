//! Read-only, on-demand UI Automation. COM objects stay on one MTA worker.
//!
//! A bounded queue and caller timeout keep a stuck provider from blocking hotkeys
//! or creating an unbounded number of threads. Late results are discarded; the
//! worker never sends keystrokes or touches the clipboard.

mod editability;
mod geometry;
mod legacy;

use std::{
    sync::{mpsc, OnceLock},
    thread,
    time::{Duration, Instant},
};

use windows::Win32::{
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    },
    UI::Accessibility::{
        CUIAutomation8, IUIAutomation2, IUIAutomationElement, IUIAutomationTextPattern,
        SupportedTextSelection_None, UIA_TextPatternId,
    },
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};

use super::{
    CaptureContext, CaptureError, CaptureTarget, CapturedSelection, Editability, SelectionBounds,
    SelectionSource, MAX_SELECTION_CHARS,
};

const PROVIDER_TIMEOUT_MS: u32 = 100;
const MAX_ANCESTORS: usize = 10;
const MAX_RANGES: i32 = 64;

type CaptureResult = Result<Option<CapturedSelection>, CaptureError>;

struct Request {
    target: CaptureTarget,
    deadline: Instant,
    reply: mpsc::SyncSender<(CaptureResult, CaptureContext)>,
}

static WORKER: OnceLock<Option<mpsc::SyncSender<Request>>> = OnceLock::new();

pub(super) fn capture(target: CaptureTarget, timeout: Duration) -> (CaptureResult, CaptureContext) {
    let Some(worker) = WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<Request>(1);
        thread::Builder::new()
            .name("selection-uia".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    if Instant::now() >= request.deadline {
                        continue;
                    }
                    let mut context = CaptureContext::default();
                    let result = probe(request.target, request.deadline, &mut context);
                    let _ = request.reply.try_send((result, context));
                }
            })
            .ok()
            .map(|_| sender)
    }) else {
        return (Ok(None), CaptureContext::default());
    };

    request_selection_context(worker, target, timeout)
}

#[cfg(test)]
fn request_selection(
    worker: &mpsc::SyncSender<Request>,
    target: CaptureTarget,
    timeout: Duration,
) -> CaptureResult {
    request_selection_context(worker, target, timeout).0
}

fn request_selection_context(
    worker: &mpsc::SyncSender<Request>,
    target: CaptureTarget,
    timeout: Duration,
) -> (CaptureResult, CaptureContext) {
    let (reply, receiver) = mpsc::sync_channel(1);
    let deadline = Instant::now() + timeout;
    if worker
        .try_send(Request {
            target,
            deadline,
            reply,
        })
        .is_err()
    {
        return (Ok(None), CaptureContext::default());
    }
    // A timeout is provider unavailability, not evidence of an empty selection.
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .unwrap_or((Ok(None), CaptureContext::default()))
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn probe(target: CaptureTarget, deadline: Instant, context: &mut CaptureContext) -> CaptureResult {
    target.validate()?;
    // This thread owns no windows. Balance both S_OK and S_FALSE with Uninitialize.
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
        return Ok(None);
    }
    let _apartment = ComApartment;
    // All interfaces are dropped before the apartment guard, including on errors.
    unsafe { probe_initialized(target, deadline, context) }
}

unsafe fn probe_initialized(
    target: CaptureTarget,
    deadline: Instant,
    context: &mut CaptureContext,
) -> CaptureResult {
    let mut search = SelectionSearch::default();
    let mut selected = None;
    let automation: windows::core::Result<IUIAutomation2> =
        unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) };
    if let Ok(automation) = automation {
        // Provider timeouts are supplemented by the caller's bounded wait.
        if unsafe { automation.SetConnectionTimeout(PROVIDER_TIMEOUT_MS) }.is_ok()
            && unsafe { automation.SetTransactionTimeout(PROVIDER_TIMEOUT_MS) }.is_ok()
            && unsafe {
                automation
                    .ElementFromHandle(windows::Win32::Foundation::HWND(target.window as *mut _))
            }
            .is_ok()
            && Instant::now() < deadline
        {
            selected =
                unsafe { probe_focused(&automation, target, deadline, &mut search, context) }?;
        }
    }
    // Like Selection Hook, direct OBJID_CLIENT access remains available even if
    // UIA could not obtain a focused element or a usable accessibility tree.
    if selected.is_none() && Instant::now() < deadline {
        target.validate()?;
        selected = search.observe(
            unsafe { legacy::from_window(target) }?,
            SelectionSource::LegacyAccessibility,
        );
    }
    target.validate()?;
    match selected {
        Some(selection) => Ok(Some(selection)),
        None => search.finish(),
    }
}

unsafe fn probe_focused(
    automation: &IUIAutomation2,
    target: CaptureTarget,
    deadline: Instant,
    search: &mut SelectionSearch,
    context: &mut CaptureContext,
) -> CaptureResult {
    let focused = match unsafe { automation.GetFocusedElement() } {
        Ok(element) => element,
        Err(_) => return Ok(None),
    };
    target.validate()?;
    if !unsafe { within_target(&focused, target) } {
        return Err(CaptureError::FocusChanged);
    }
    if unsafe { focused.CurrentIsPassword() }.is_ok_and(|value| value.as_bool()) {
        return Err(CaptureError::ProtectedField);
    }
    // Exact compatibility order: focused TextPattern, focused legacy bridge,
    // then ancestor TextPatterns. No whole-document text probes.
    let mut selected = search.observe(
        unsafe { read_element(&focused, deadline) },
        SelectionSource::UiAutomation,
    );
    if let Some(selection) = selected.as_mut() {
        selection.editability = unsafe { editability::read(&focused, deadline) };
    }
    if selected.is_none() && Instant::now() < deadline {
        target.validate()?;
        selected = search.observe(
            unsafe { legacy::from_bridge(&focused) }?,
            SelectionSource::LegacyAccessibility,
        );
    }
    if selected.is_none() {
        if let Ok(walker) = unsafe { automation.ControlViewWalker() } {
            let mut element = focused.clone();
            for _ in 0..MAX_ANCESTORS {
                target.validate()?;
                if Instant::now() >= deadline {
                    break;
                }
                element = match unsafe { walker.GetParentElement(&element) } {
                    Ok(parent) => parent,
                    Err(_) => break,
                };
                if !unsafe { within_target(&element, target) } {
                    break;
                }
                if unsafe { element.CurrentIsPassword() }.is_ok_and(|value| value.as_bool()) {
                    return Err(CaptureError::ProtectedField);
                }
                selected = search.observe(
                    unsafe { read_element(&element, deadline) },
                    SelectionSource::UiAutomation,
                );
                if let Some(selection) = selected.as_mut() {
                    selection.editability = unsafe { editability::read(&element, deadline) };
                    break;
                }
            }
        }
    }
    target.validate()?;
    let current_focus = match unsafe { automation.GetFocusedElement() } {
        Ok(element) => element,
        Err(_) => return Ok(None),
    };
    if !unsafe { automation.CompareElements(&focused, &current_focus) }
        .is_ok_and(|same| same.as_bool())
    {
        return Err(CaptureError::FocusChanged);
    }
    if Instant::now() < deadline {
        context.editability = unsafe { editability::read(&focused, deadline) };
        context.focus_id = unsafe { editability::runtime_id(&focused) };
    }
    if let Some(selection) = selected.as_mut() {
        if matches!(selection.source, SelectionSource::LegacyAccessibility) {
            selection.editability = context.editability;
        }
    }
    Ok(selected)
}

unsafe fn within_target(element: &IUIAutomationElement, target: CaptureTarget) -> bool {
    // Virtual browser elements need not have a native window handle.
    match unsafe { element.CurrentNativeWindowHandle() } {
        Ok(hwnd) if !hwnd.0.is_null() => {
            (unsafe { GetAncestor(hwnd.0, GA_ROOT) }) as usize == target.window
        }
        _ => true,
    }
}

#[derive(Default)]
struct SelectionSearch {
    empty: bool,
    non_text: bool,
    virtual_editor: bool,
}

impl SelectionSearch {
    fn observe(
        &mut self,
        result: ElementSelection,
        source: SelectionSource,
    ) -> Option<CapturedSelection> {
        match result {
            ElementSelection::Text(text, truncated) => {
                return Some(CapturedSelection {
                    text,
                    source,
                    truncated,
                    clipboard_restored: true,
                    bounds: None,
                    editability: Editability::Unknown,
                    replacement_id: None,
                })
            }
            ElementSelection::PositionedText(text, truncated, bounds) => {
                return Some(CapturedSelection {
                    text,
                    source,
                    truncated,
                    clipboard_restored: true,
                    bounds: Some(bounds),
                    editability: Editability::Unknown,
                    replacement_id: None,
                })
            }
            ElementSelection::Empty => self.empty = true,
            ElementSelection::NonTextSelection => self.non_text = true,
            ElementSelection::CopyRequired => self.virtual_editor = true,
            ElementSelection::Unavailable => {}
        }
        None
    }

    fn finish(self) -> CaptureResult {
        if self.non_text {
            Err(CaptureError::NonTextSelection)
        } else if self.virtual_editor {
            Err(CaptureError::CopyRequired)
        } else if self.empty {
            Err(CaptureError::NoSelection)
        } else {
            Ok(None)
        }
    }
}

enum ElementSelection {
    Text(String, bool),
    PositionedText(String, bool, SelectionBounds),
    Empty,
    CopyRequired,
    Unavailable,
    NonTextSelection,
}

fn is_virtual_editor(class_name: &str) -> bool {
    let classes = class_name.split_whitespace().collect::<Vec<_>>();
    // Match Monaco controls, not the Code.exe process: its search boxes and
    // terminal have different selection semantics. Also covers Monaco in a browser.
    classes.contains(&"native-edit-context")
        || classes.contains(&"monaco-editor")
        || (classes.contains(&"inputarea") && classes.contains(&"monaco-mouse-cursor-text"))
}

unsafe fn read_element(element: &IUIAutomationElement, deadline: Instant) -> ElementSelection {
    let virtual_editor = unsafe { element.CurrentClassName() }
        .is_ok_and(|name| is_virtual_editor(&name.to_string()));
    // Never use DocumentRange, ValuePattern, or Name as a selection substitute.
    let read = || -> windows::core::Result<ElementSelection> {
        let pattern: IUIAutomationTextPattern =
            unsafe { element.GetCurrentPatternAs(UIA_TextPatternId)? };
        if unsafe { pattern.SupportedTextSelection()? } == SupportedTextSelection_None {
            return Ok(ElementSelection::Unavailable);
        }
        let ranges = unsafe { pattern.GetSelection()? };
        let count = unsafe { ranges.Length()? };
        if count < 0 {
            return Ok(ElementSelection::Unavailable);
        }
        let mut text = SelectionText::default();
        let mut bounds: Option<SelectionBounds> = None;
        for index in 0..count.min(MAX_RANGES) {
            if Instant::now() >= deadline {
                return Ok(ElementSelection::Unavailable);
            }
            let range = unsafe { ranges.GetElement(index)? };
            // UIA lengths use UTF-16. Allow surrogate pairs and one extra Unicode
            // scalar to detect truncation without requesting an entire document.
            let limit = 2 * (MAX_SELECTION_CHARS - text.chars + 1);
            let value = unsafe { range.GetText(limit as i32)? }.to_string();
            text.append_range(&value);
            // Coordinates are optional; an unsupported geometry provider must
            // never turn usable selected text into a failed capture.
            if !value.is_empty()
                && deadline.saturating_duration_since(Instant::now()) > Duration::from_millis(30)
            {
                if let Some(range_bounds) = unsafe { geometry::read_bounds(&range) } {
                    match bounds.as_mut() {
                        Some(bounds) => bounds.last = range_bounds.last,
                        None => bounds = Some(range_bounds),
                    }
                }
            }
            if text.truncated {
                break;
            }
        }
        if count > MAX_RANGES {
            text.truncated = true;
        }
        if text.text.trim().is_empty() {
            // Unvisited ranges may contain text, so don't claim no selection.
            Ok(if count > MAX_RANGES {
                ElementSelection::Unavailable
            } else {
                ElementSelection::Empty
            })
        } else {
            Ok(match bounds {
                Some(bounds) => ElementSelection::PositionedText(text.text, text.truncated, bounds),
                None => ElementSelection::Text(text.text, text.truncated),
            })
        }
    };
    match read().unwrap_or(ElementSelection::Unavailable) {
        ElementSelection::Empty | ElementSelection::Unavailable if virtual_editor => {
            ElementSelection::CopyRequired
        }
        result => result,
    }
}

#[derive(Default)]
struct SelectionText {
    text: String,
    chars: usize,
    truncated: bool,
}

impl SelectionText {
    fn append_range(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        // Separate disjoint selections so their words cannot run together.
        let separator = (!self.text.is_empty()).then_some('\n');
        for ch in separator.into_iter().chain(value.chars()) {
            if self.chars == MAX_SELECTION_CHARS {
                self.truncated = true;
                break;
            }
            self.text.push(ch);
            self.chars += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_pattern_does_not_skip_a_successful_legacy_read() {
        let mut search = SelectionSearch::default();
        assert!(search
            .observe(ElementSelection::Empty, SelectionSource::UiAutomation)
            .is_none());
        let captured = search
            .observe(
                ElementSelection::Text("  legacy selection\n".into(), false),
                SelectionSource::LegacyAccessibility,
            )
            .unwrap();
        assert_eq!(captured.text, "  legacy selection\n");
        assert_eq!(
            serde_json::to_value(captured.source).unwrap(),
            "legacy-accessibility"
        );
        assert!(captured.clipboard_restored);
    }

    #[test]
    fn non_text_objects_block_copy_but_do_not_prevent_later_text_retrieval() {
        let mut search = SelectionSearch::default();
        search.observe(
            ElementSelection::NonTextSelection,
            SelectionSource::LegacyAccessibility,
        );
        assert!(matches!(
            search.finish(),
            Err(CaptureError::NonTextSelection)
        ));
        let mut search = SelectionSearch::default();
        search.observe(
            ElementSelection::NonTextSelection,
            SelectionSource::LegacyAccessibility,
        );
        assert!(search
            .observe(
                ElementSelection::Text("ancestor selection".into(), false),
                SelectionSource::UiAutomation
            )
            .is_some());
    }

    #[test]
    fn virtual_editor_empty_ranges_remain_inconclusive_after_all_readers() {
        let mut search = SelectionSearch::default();
        search.observe(
            ElementSelection::CopyRequired,
            SelectionSource::UiAutomation,
        );
        search.observe(ElementSelection::Empty, SelectionSource::UiAutomation);
        search.observe(
            ElementSelection::Unavailable,
            SelectionSource::LegacyAccessibility,
        );
        assert!(matches!(search.finish(), Err(CaptureError::CopyRequired)));
        let mut search = SelectionSearch::default();
        search.observe(ElementSelection::Empty, SelectionSource::UiAutomation);
        search.observe(
            ElementSelection::Unavailable,
            SelectionSource::LegacyAccessibility,
        );
        assert!(matches!(search.finish(), Err(CaptureError::NoSelection)));
    }

    #[test]
    fn identifies_virtual_editor_inputs_without_disabling_accessibility_reads() {
        for class_name in [
            "inputarea monaco-mouse-cursor-text",
            "inputarea monaco-mouse-cursor-text ime-input",
            "native-edit-context",
            "monaco-editor no-user-select vs-dark",
        ] {
            assert!(is_virtual_editor(class_name), "{class_name}");
        }
    }

    #[test]
    fn editor_compatibility_does_not_apply_to_terminals_or_normal_inputs() {
        for class_name in [
            "EDIT",
            "",
            "monaco-inputbox",
            "inputarea",
            "xterm-helper-textarea",
            "xterm-screen",
            "not-monaco-editor",
            "search-input",
        ] {
            assert!(!is_virtual_editor(class_name), "{class_name}");
        }
    }

    fn dummy_target() -> CaptureTarget {
        CaptureTarget {
            window: 0,
            process_id: 0,
        }
    }

    #[test]
    fn stuck_provider_times_out_and_late_result_is_discarded() {
        let (sender, requests) = mpsc::sync_channel(1);
        // Keeping the receiver alive simulates a worker blocked in a provider.
        assert!(
            request_selection(&sender, dummy_target(), Duration::from_millis(10))
                .unwrap()
                .is_none()
        );
        let request = requests.recv().unwrap();
        assert!(request
            .reply
            .try_send((Err(CaptureError::NoSelection), CaptureContext::default()))
            .is_err());
    }

    #[test]
    fn busy_worker_does_not_queue_unbounded_requests() {
        let (sender, _requests) = mpsc::sync_channel(1);
        let (reply, _receiver) = mpsc::sync_channel(1);
        sender
            .try_send(Request {
                target: dummy_target(),
                deadline: Instant::now(),
                reply,
            })
            .unwrap();
        assert!(
            request_selection(&sender, dummy_target(), Duration::from_secs(1))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn empty_selection_does_not_become_clipboard_fallback() {
        let (sender, requests) = mpsc::sync_channel::<Request>(1);
        let provider = thread::spawn(move || {
            requests
                .recv()
                .unwrap()
                .reply
                .send((Err(CaptureError::NoSelection), CaptureContext::default()))
                .unwrap();
        });
        assert!(matches!(
            request_selection(&sender, dummy_target(), Duration::from_secs(1)),
            Err(CaptureError::NoSelection)
        ));
        provider.join().unwrap();
    }

    #[test]
    fn joins_disjoint_ranges_without_losing_indentation() {
        let mut text = SelectionText::default();
        text.append_range(""); // A caret range is not selected text.
        text.append_range("    let a = 1;\n");
        text.append_range("    let b = 2;");
        assert_eq!(text.text, "    let a = 1;\n\n    let b = 2;");
        assert!(!text.truncated);
    }

    #[test]
    fn caps_combined_ranges_on_unicode_scalar_boundaries() {
        let mut text = SelectionText::default();
        text.append_range(&"\u{1f980}".repeat(MAX_SELECTION_CHARS - 2));
        text.append_range("ab");
        assert_eq!(text.text.chars().count(), MAX_SELECTION_CHARS);
        assert!(text.text.ends_with("\na"));
        assert!(text.truncated);
    }

    #[test]
    fn exact_limit_is_not_truncated_until_more_text_is_selected() {
        let mut text = SelectionText::default();
        text.append_range(&"x".repeat(MAX_SELECTION_CHARS));
        assert!(!text.truncated);
        text.append_range("");
        assert!(!text.truncated);
        text.append_range("y");
        assert!(text.truncated);
        assert_eq!(text.text.len(), MAX_SELECTION_CHARS);
    }

    #[test]
    fn reads_only_selected_text_from_a_real_windows_edit_control() {
        use windows::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::Controls::{EM_SETREADONLY, EM_SETSEL};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW,
            GetWindowThreadProcessId, PostThreadMessageW, SendMessageW, TranslateMessage,
            ES_MULTILINE, MSG, WM_QUIT, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        };

        // A hidden parent and a normal EDIT child exercise the actual Windows
        // accessibility provider without taking focus or changing the clipboard.
        struct TestControl {
            hwnd: usize,
            thread_id: u32,
            owner: Option<thread::JoinHandle<()>>,
        }
        impl Drop for TestControl {
            fn drop(&mut self) {
                unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
                self.owner.take().unwrap().join().unwrap();
            }
        }
        let (ready, receive) = mpsc::sync_channel(1);
        let owner = thread::spawn(move || unsafe {
            let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let parent = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                400,
                200,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!parent.is_null());
            let edit = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                wide("prefix selected suffix").as_ptr(),
                WS_CHILD | WS_VISIBLE | ES_MULTILINE as u32,
                0,
                0,
                300,
                100,
                parent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!edit.is_null());
            SendMessageW(edit, EM_SETSEL, 7, 15);
            let thread_id = GetWindowThreadProcessId(edit, std::ptr::null_mut());
            ready.send((edit as usize, thread_id)).unwrap();
            let mut message = MSG::default();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            DestroyWindow(parent);
        });
        let (hwnd, thread_id) = receive.recv().unwrap();
        let control = TestControl {
            hwnd,
            thread_id,
            owner: Some(owner),
        };
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .unwrap();
        let _apartment = ComApartment;
        let automation: IUIAutomation2 =
            unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }.unwrap();
        let element =
            unsafe { automation.ElementFromHandle(HWND(control.hwnd as *mut _)) }.unwrap();
        assert_eq!(
            unsafe { editability::read(&element, Instant::now() + Duration::from_secs(3)) },
            Editability::Editable
        );
        let first_id =
            unsafe { editability::runtime_id(&element) }.expect("EDIT has a runtime identity");
        assert_eq!(Some(first_id), unsafe { editability::runtime_id(&element) });
        unsafe { SendMessageW(control.hwnd as *mut _, EM_SETREADONLY, 1, 0) };
        assert_eq!(
            unsafe { editability::read(&element, Instant::now() + Duration::from_secs(3)) },
            Editability::ReadOnly
        );
        unsafe { SendMessageW(control.hwnd as *mut _, EM_SETREADONLY, 0, 0) };
        match unsafe { read_element(&element, Instant::now() + Duration::from_secs(3)) } {
            ElementSelection::Text(text, truncated)
            | ElementSelection::PositionedText(text, truncated, _) => {
                assert_eq!(text, "selected");
                assert!(!truncated);
            }
            _ => panic!("Windows EDIT provider did not return its selected text"),
        }
        unsafe { SendMessageW(control.hwnd as *mut _, EM_SETSEL, 7, 7) };
        assert!(matches!(
            unsafe { read_element(&element, Instant::now() + Duration::from_secs(3)) },
            ElementSelection::Empty
        ));
        // Exercise both legacy entry points against a real provider. Its MSAA
        // object must not substitute the EDIT control's full value for selection.
        assert!(matches!(
            unsafe { legacy::from_bridge(&element) }.unwrap(),
            ElementSelection::Unavailable | ElementSelection::NonTextSelection
        ));
        assert!(matches!(
            unsafe {
                legacy::from_window(CaptureTarget {
                    window: control.hwnd,
                    process_id: std::process::id(),
                })
            }
            .unwrap(),
            ElementSelection::Unavailable | ElementSelection::NonTextSelection
        ));
    }
}
