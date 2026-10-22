//! MSAA compatibility reads, following Selection Hook's legacy lookup behavior.
//! Only a provider-returned selection string is text. Names and values are never
//! substituted for highlighted text, and selected UI objects prevent Copy.

use windows::{
    core::Interface,
    Win32::{
        Foundation::HWND,
        System::Variant::{
            VariantClear, VARIANT, VT_ARRAY, VT_BSTR, VT_DISPATCH, VT_I4, VT_UNKNOWN,
        },
        UI::Accessibility::{
            AccessibleObjectFromWindow, IAccessible, IUIAutomationElement,
            IUIAutomationLegacyIAccessiblePattern, UIA_LegacyIAccessiblePatternId,
            ROLE_SYSTEM_OUTLINEBUTTON, ROLE_SYSTEM_TEXT, ROLE_SYSTEM_TITLEBAR,
        },
    },
};

use super::{CaptureError, CaptureTarget, ElementSelection};
use crate::selection::normalize_selection;

// MSAA's OBJID_CLIENT and STATE_SYSTEM_PROTECTED, defined by the Windows SDK.
const OBJID_CLIENT: u32 = 0xffff_fffc;
const STATE_SYSTEM_PROTECTED: i32 = 0x2000_0000;

struct OwnedVariant(VARIANT);

impl Drop for OwnedVariant {
    fn drop(&mut self) {
        // windows-rs exposes this ABI type without automatic VariantClear.
        unsafe {
            let _ = VariantClear(&mut self.0);
        }
    }
}

fn child_id(id: i32) -> OwnedVariant {
    let mut value = OwnedVariant(VARIANT::default());
    unsafe {
        let inner = &mut *value.0.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = id;
    }
    value
}

pub(super) unsafe fn from_bridge(
    element: &IUIAutomationElement,
) -> Result<ElementSelection, CaptureError> {
    let accessible = unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
                UIA_LegacyIAccessiblePatternId,
            )
            .and_then(|pattern| pattern.GetIAccessible())
    };
    match accessible {
        Ok(accessible) => unsafe { read_selection(&accessible) },
        Err(_) => Ok(ElementSelection::Unavailable),
    }
}

pub(super) unsafe fn from_window(target: CaptureTarget) -> Result<ElementSelection, CaptureError> {
    let mut raw = std::ptr::null_mut();
    if unsafe {
        AccessibleObjectFromWindow(
            HWND(target.window as *mut _),
            OBJID_CLIENT,
            &IAccessible::IID,
            &mut raw,
        )
    }
    .is_err()
        || raw.is_null()
    {
        return Ok(ElementSelection::Unavailable);
    }
    // The returned interface owns a COM reference, released when this wrapper drops.
    let accessible = unsafe { IAccessible::from_raw(raw) };
    unsafe { read_selection(&accessible) }
}

unsafe fn read_selection(accessible: &IAccessible) -> Result<ElementSelection, CaptureError> {
    let self_id = child_id(0);
    if let Ok(state) = unsafe { accessible.get_accState(&self_id.0) } {
        let state = OwnedVariant(state);
        if unsafe { state.0.Anonymous.Anonymous.vt } == VT_I4
            && unsafe { state.0.Anonymous.Anonymous.Anonymous.lVal } & STATE_SYSTEM_PROTECTED != 0
        {
            return Err(CaptureError::ProtectedField);
        }
    }
    let selection = match unsafe { accessible.accSelection() } {
        Ok(value) => OwnedVariant(value),
        Err(_) => return Ok(ElementSelection::Unavailable),
    };
    Ok(unsafe { decode_selection(&selection.0, |child| classify_role(accessible, child)) })
}

unsafe fn classify_role(accessible: &IAccessible, child: &VARIANT) -> ElementSelection {
    let role = match unsafe { accessible.get_accRole(child) } {
        Ok(value) => OwnedVariant(value),
        Err(_) => return ElementSelection::Unavailable,
    };
    if unsafe { role.0.Anonymous.Anonymous.vt } != VT_I4 {
        return ElementSelection::Unavailable;
    }
    let role = unsafe { role.0.Anonymous.Anonymous.Anonymous.lVal };
    if role != ROLE_SYSTEM_TEXT as i32
        && (ROLE_SYSTEM_TITLEBAR as i32..=ROLE_SYSTEM_OUTLINEBUTTON as i32).contains(&role)
    {
        ElementSelection::NonTextSelection
    } else {
        ElementSelection::Unavailable
    }
}

unsafe fn decode_selection(
    value: &VARIANT,
    classify_child: impl FnOnce(&VARIANT) -> ElementSelection,
) -> ElementSelection {
    let value = unsafe { &value.Anonymous.Anonymous };
    match value.vt {
        VT_BSTR => {
            let text = unsafe { &value.Anonymous.bstrVal }
                .to_string()
                .replace('\u{fffc}', "");
            if text.trim().is_empty() {
                return ElementSelection::Unavailable;
            }
            let (text, truncated) = normalize_selection(text);
            ElementSelection::Text(text, truncated)
        }
        VT_UNKNOWN => ElementSelection::NonTextSelection,
        VT_I4 => classify_child(&child_id(unsafe { value.Anonymous.lVal }).0),
        VT_DISPATCH => {
            let selected = unsafe { &value.Anonymous.pdispVal }
                .as_ref()
                .and_then(|dispatch| dispatch.cast::<IAccessible>().ok());
            match selected {
                Some(accessible) => unsafe { classify_role(&accessible, &child_id(0).0) },
                None => ElementSelection::Unavailable,
            }
        }
        vt if vt.0 & VT_ARRAY.0 != 0 => ElementSelection::NonTextSelection,
        _ => ElementSelection::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::ManuallyDrop;
    use windows::core::BSTR;

    #[test]
    fn reads_only_explicit_selection_strings_preserving_unicode_and_nuls() {
        let mut value = OwnedVariant(VARIANT::default());
        unsafe {
            let inner = &mut *value.0.Anonymous.Anonymous;
            inner.vt = VT_BSTR;
            inner.Anonymous.bstrVal = ManuallyDrop::new(BSTR::from("  selected\0\u{1f980}\n"));
        }
        match unsafe {
            decode_selection(&value.0, |_| {
                panic!("String selection needs no object lookup")
            })
        } {
            ElementSelection::Text(text, truncated) => {
                assert_eq!(text, "  selected\0\u{1f980}\n");
                assert!(!truncated);
            }
            _ => panic!("Expected selected text"),
        }
    }

    #[test]
    fn empty_msaa_selection_is_unavailable_not_proof_of_no_highlight() {
        let value = OwnedVariant(VARIANT::default());
        assert!(matches!(
            unsafe { decode_selection(&value.0, |_| unreachable!()) },
            ElementSelection::Unavailable
        ));
    }

    #[test]
    fn selected_objects_are_not_returned_as_text() {
        let mut value = OwnedVariant(VARIANT::default());
        unsafe {
            (*value.0.Anonymous.Anonymous).vt = VT_UNKNOWN;
        }
        assert!(matches!(
            unsafe { decode_selection(&value.0, |_| unreachable!()) },
            ElementSelection::NonTextSelection
        ));
        let value = child_id(5);
        assert!(matches!(
            unsafe { decode_selection(&value.0, |_| ElementSelection::NonTextSelection) },
            ElementSelection::NonTextSelection
        ));
    }
}
