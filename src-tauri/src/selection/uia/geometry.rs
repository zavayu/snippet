use windows::Win32::{
    System::{
        Com::SAFEARRAY,
        Ole::{
            SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetLBound,
            SafeArrayGetUBound, SafeArrayGetVartype, SafeArrayUnaccessData,
        },
        Variant::VT_R8,
    },
    UI::Accessibility::IUIAutomationTextRange,
};

use crate::selection::{SelectionBounds, SelectionRect};

struct RectArray(*mut SAFEARRAY);
impl Drop for RectArray {
    fn drop(&mut self) {
        unsafe {
            let _ = SafeArrayDestroy(self.0);
        }
    }
}

pub(super) unsafe fn read_bounds(range: &IUIAutomationTextRange) -> Option<SelectionBounds> {
    let raw = unsafe { range.GetBoundingRectangles() }.ok()?;
    if raw.is_null() {
        return None;
    }
    let array = RectArray(raw);
    if unsafe { SafeArrayGetDim(array.0) } != 1
        || unsafe { SafeArrayGetVartype(array.0) }.ok()? != VT_R8
    {
        return None;
    }
    let lower = unsafe { SafeArrayGetLBound(array.0, 1) }.ok()? as i64;
    let upper = unsafe { SafeArrayGetUBound(array.0, 1) }.ok()? as i64;
    let count = usize::try_from(upper - lower + 1).ok()?;
    if count == 0 || count > 65_536 || count % 4 != 0 {
        return None;
    }
    let mut data = std::ptr::null_mut();
    unsafe { SafeArrayAccessData(array.0, &mut data) }.ok()?;
    let bounds = if data.is_null() {
        None
    } else {
        bounds_from_rectangles(unsafe { std::slice::from_raw_parts(data as *const f64, count) })
    };
    unsafe {
        let _ = SafeArrayUnaccessData(array.0);
    }
    bounds
}

fn bounds_from_rectangles(values: &[f64]) -> Option<SelectionBounds> {
    let mut bounds: Option<SelectionBounds> = None;
    for rect in values.chunks_exact(4) {
        let [x, y, width, height] = [rect[0], rect[1], rect[2], rect[3]];
        if ![x, y, width, height, x + width, y + height]
            .iter()
            .all(|v| v.is_finite() && *v >= i32::MIN as f64 && *v <= i32::MAX as f64)
            || width <= 0.0
            || height <= 0.0
        {
            continue;
        }
        let rect = SelectionRect {
            x,
            y,
            width,
            height,
        };
        match bounds.as_mut() {
            Some(bounds) => bounds.last = rect,
            None => {
                bounds = Some(SelectionBounds {
                    first: rect,
                    last: rect,
                })
            }
        }
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_first_and_last_visible_lines_and_supports_negative_monitor_origins() {
        let bounds =
            bounds_from_rectangles(&[-800., 100., 200., 20., -800., 120., 80., 20.]).unwrap();
        assert_eq!(bounds.first.y, 100.);
        assert_eq!(bounds.last.y + bounds.last.height, 140.);
        assert_eq!(bounds.last.x, -800.);
    }
    #[test]
    fn invalid_or_missing_rectangles_do_not_produce_an_anchor() {
        assert!(bounds_from_rectangles(&[]).is_none());
        assert!(bounds_from_rectangles(&[f64::NAN, 0., 10., 10., 0., 0., 0., 10.]).is_none());
        assert!(bounds_from_rectangles(&[f64::MAX, 0., 10., 10.]).is_none());
    }
}
