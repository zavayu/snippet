//! Conservative evidence of editability; unsupported and mixed attributes stay unknown.
use crate::selection::Editability;
use std::time::Instant;
use windows::Win32::{
    System::{
        Ole::{
            SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetLBound,
            SafeArrayGetUBound, SafeArrayGetVartype, SafeArrayUnaccessData,
        },
        Variant::{VariantClear, VT_BOOL, VT_I4},
    },
    UI::Accessibility::{
        IUIAutomationElement, IUIAutomationLegacyIAccessiblePattern, IUIAutomationTextPattern,
        IUIAutomationValuePattern, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
        UIA_IsReadOnlyAttributeId, UIA_LegacyIAccessiblePatternId, UIA_TextPatternId,
        UIA_ValuePatternId, ROLE_SYSTEM_TEXT,
    },
};

pub(super) unsafe fn read(element: &IUIAutomationElement, deadline: Instant) -> Editability {
    if Instant::now() >= deadline
        || !unsafe { element.CurrentIsEnabled() }.is_ok_and(|v| v.as_bool())
        || !unsafe { element.CurrentIsPassword() }.is_ok_and(|v| !v.as_bool())
    {
        return Editability::Unknown;
    }
    if let Ok(pattern) =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
    {
        if let Ok(ranges) = unsafe { pattern.GetSelection() } {
            let count = unsafe { ranges.Length() }.unwrap_or(0);
            // Multiple selections cannot be replaced safely as one text insertion.
            if count > 1 {
                return Editability::Unknown;
            }
            if count == 1 {
                if let Ok(range) = unsafe { ranges.GetElement(0) } {
                    if let Ok(mut value) =
                        unsafe { range.GetAttributeValue(UIA_IsReadOnlyAttributeId) }
                    {
                        let result = unsafe {
                            (value.Anonymous.Anonymous.vt == VT_BOOL).then(|| {
                                if value.Anonymous.Anonymous.Anonymous.boolVal.0 != 0 {
                                    Editability::ReadOnly
                                } else {
                                    Editability::Editable
                                }
                            })
                        };
                        unsafe {
                            let _ = VariantClear(&mut value);
                        }
                        if let Some(result) = result {
                            return result;
                        }
                    }
                }
            }
        }
    }
    if Instant::now() >= deadline {
        return Editability::Unknown;
    }
    let text_control = unsafe { element.CurrentControlType() }
        .is_ok_and(|kind| kind == UIA_EditControlTypeId || kind == UIA_DocumentControlTypeId);
    if text_control {
        if let Ok(pattern) =
            unsafe { element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) }
        {
            if let Ok(read_only) = unsafe { pattern.CurrentIsReadOnly() } {
                return if read_only.as_bool() {
                    Editability::ReadOnly
                } else {
                    Editability::Editable
                };
            }
        }
    }
    if Instant::now() >= deadline {
        return Editability::Unknown;
    }
    if let Ok(pattern) = unsafe {
        element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
            UIA_LegacyIAccessiblePatternId,
        )
    } {
        if let (Ok(role), Ok(state)) = (unsafe { pattern.CurrentRole() }, unsafe {
            pattern.CurrentState()
        }) {
            if role == ROLE_SYSTEM_TEXT {
                // Do not interpret a missing flag on an arbitrary document as editable.
                if state & 0x40 != 0 {
                    return Editability::ReadOnly;
                }
                if unsafe { element.CurrentControlType() }
                    .is_ok_and(|kind| kind == UIA_EditControlTypeId)
                    && state & (0x1 | 0x2000_0000) == 0
                {
                    return Editability::Editable;
                }
            }
        }
    }
    Editability::Unknown
}

pub(super) unsafe fn runtime_id(element: &IUIAutomationElement) -> Option<Vec<i32>> {
    let raw = unsafe { element.GetRuntimeId() }.ok()?;
    if raw.is_null() {
        return None;
    }
    let result = (|| {
        if unsafe { SafeArrayGetDim(raw) } != 1
            || unsafe { SafeArrayGetVartype(raw) }.ok()? != VT_I4
        {
            return None;
        }
        let count = i64::from(unsafe { SafeArrayGetUBound(raw, 1) }.ok()?)
            - i64::from(unsafe { SafeArrayGetLBound(raw, 1) }.ok()?)
            + 1;
        if !(1..=128).contains(&count) {
            return None;
        }
        let mut data = std::ptr::null_mut();
        unsafe { SafeArrayAccessData(raw, &mut data) }.ok()?;
        let id = if data.is_null() {
            None
        } else {
            Some(unsafe { std::slice::from_raw_parts(data as *const i32, count as usize) }.to_vec())
        };
        unsafe {
            let _ = SafeArrayUnaccessData(raw);
        }
        id
    })();
    unsafe {
        let _ = SafeArrayDestroy(raw);
    }
    result
}
