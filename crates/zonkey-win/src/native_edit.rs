//! Narrow, diagnostic-only reads for standard Win32 `EDIT` controls.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    ES_PASSWORD, GWL_STYLE, GetClassNameW, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
};

const EM_GETSEL: u32 = 0x00b0;
const READ_TIMEOUT_MS: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeEditProbeError {
    UnsupportedControl,
    SecureControl,
    ReadFailed,
    ReadTimeout,
    ContradictorySelection,
    EvidenceChanged,
    TextMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeEditEvidence {
    pub candidate_text_units: usize,
    pub selection_empty: bool,
    pub exact_preceding_text: bool,
}

/// Reads one standard `EDIT` control without changing it.
///
/// The text and selection reads are intentionally separate and therefore are
/// not an atomic snapshot. This API is diagnostic-only and must not authorize
/// a later mutation.
pub fn probe_standard_edit(
    hwnd: HWND,
    expected: &str,
) -> Result<NativeEditEvidence, NativeEditProbeError> {
    if expected.is_empty() {
        return Err(NativeEditProbeError::TextMismatch);
    }
    let first = read_sample(hwnd)?;
    let second = read_sample(hwnd)?;
    validate_sample(ensure_stable(&first, &second)?, expected)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeEditSample {
    class_name: Vec<u16>,
    style: u32,
    text: Vec<u16>,
    start: u32,
    end: u32,
}

fn read_sample(hwnd: HWND) -> Result<NativeEditSample, NativeEditProbeError> {
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, None) };
    if thread_id == 0 {
        return Err(NativeEditProbeError::ReadFailed);
    }

    let mut class_name = [0u16; 32];
    let class_len = unsafe { GetClassNameW(hwnd, &mut class_name) };
    let class_len = usize::try_from(class_len).map_err(|_| NativeEditProbeError::ReadFailed)?;
    if class_len == 0 || String::from_utf16_lossy(&class_name[..class_len]) != "Edit" {
        return Err(NativeEditProbeError::UnsupportedControl);
    }
    let style = u32::try_from(unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) })
        .map_err(|_| NativeEditProbeError::ReadFailed)?;
    if style & ES_PASSWORD as u32 != 0 {
        return Err(NativeEditProbeError::SecureControl);
    }

    let text_len = unsafe { GetWindowTextLengthW(hwnd) };
    if text_len < 0 {
        return Err(NativeEditProbeError::ReadFailed);
    }
    let text_len = usize::try_from(text_len).map_err(|_| NativeEditProbeError::ReadFailed)?;
    let mut text = vec![0u16; text_len + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut text) };
    let copied = usize::try_from(copied).map_err(|_| NativeEditProbeError::ReadFailed)?;
    if copied > text_len {
        return Err(NativeEditProbeError::ReadFailed);
    }
    text.truncate(copied);

    let mut start = 0u32;
    let mut end = 0u32;
    let mut result = 0usize;
    let lresult = unsafe {
        SendMessageTimeoutW(
            hwnd,
            EM_GETSEL,
            windows::Win32::Foundation::WPARAM((&raw mut start) as usize),
            windows::Win32::Foundation::LPARAM((&raw mut end) as isize),
            SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0),
            READ_TIMEOUT_MS,
            Some(&raw mut result),
        )
    };
    if lresult.0 == 0 {
        return Err(NativeEditProbeError::ReadTimeout);
    }
    if start > end || end as usize > text.len() {
        return Err(NativeEditProbeError::ContradictorySelection);
    }
    Ok(NativeEditSample {
        class_name: class_name[..class_len].to_vec(),
        style,
        text,
        start,
        end,
    })
}

fn validate_sample(
    sample: &NativeEditSample,
    expected: &str,
) -> Result<NativeEditEvidence, NativeEditProbeError> {
    if sample.start != sample.end {
        return Err(NativeEditProbeError::ContradictorySelection);
    }
    let expected_units: Vec<u16> = expected.encode_utf16().collect();
    if expected_units.len() > sample.start as usize {
        return Err(NativeEditProbeError::TextMismatch);
    }
    let candidate =
        &sample.text[sample.start as usize - expected_units.len()..sample.start as usize];
    if candidate != expected_units.as_slice() {
        return Err(NativeEditProbeError::TextMismatch);
    }
    Ok(NativeEditEvidence {
        candidate_text_units: expected_units.len(),
        selection_empty: true,
        exact_preceding_text: true,
    })
}

fn ensure_stable<'a>(
    first: &NativeEditSample,
    second: &'a NativeEditSample,
) -> Result<&'a NativeEditSample, NativeEditProbeError> {
    if first == second {
        Ok(second)
    } else {
        Err(NativeEditProbeError::EvidenceChanged)
    }
}

#[cfg(test)]
mod tests {
    use super::{NativeEditProbeError, NativeEditSample, ensure_stable, validate_sample};

    fn sample(text: &str, start: u32, end: u32) -> NativeEditSample {
        NativeEditSample {
            class_name: "Edit".encode_utf16().collect(),
            style: 0,
            text: text.encode_utf16().collect(),
            start,
            end,
        }
    }

    #[test]
    fn stable_equal_samples_validate() {
        let first = sample("resume", 6, 6);
        let second = first.clone();
        let stable = ensure_stable(&first, &second).expect("equal samples");
        assert!(validate_sample(stable, "resume").is_ok());
    }

    #[test]
    fn changed_text_is_detectable_before_validation() {
        let first = sample("resume", 6, 6);
        let second = sample("resumf", 6, 6);
        assert_eq!(
            ensure_stable(&first, &second),
            Err(NativeEditProbeError::EvidenceChanged)
        );
        assert_eq!(
            validate_sample(&second, "resume"),
            Err(NativeEditProbeError::TextMismatch)
        );
    }

    #[test]
    fn changed_selection_is_detectable_before_validation() {
        let first = sample("resume", 6, 6);
        let second = sample("resume", 0, 2);
        assert_eq!(
            ensure_stable(&first, &second),
            Err(NativeEditProbeError::EvidenceChanged)
        );
        assert_eq!(
            validate_sample(&second, "resume"),
            Err(NativeEditProbeError::ContradictorySelection)
        );
    }
}
