#![allow(clippy::cmp_owned, clippy::struct_excessive_bools)]

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, TextUnit_Character,
    UIA_EditControlTypeId, UIA_TextPatternId,
};
use windows::core::BSTR;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiaProbeError {
    ComInitialization,
    ProviderUnavailable,
    UnsupportedControl,
    SecureControl,
    SelectionUnavailable,
    AmbiguousRange,
    TextMismatch,
    UnknownEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiaEvidence {
    pub identity_available: bool,
    pub caret_selection_available: bool,
    pub selection_empty: bool,
    pub exact_preceding_text: bool,
    pub candidate_text_units: usize,
}

pub fn probe_focused_edit(rendered_token: &str) -> Result<UiaEvidence, UiaProbeError> {
    if rendered_token.is_empty() {
        return Err(UiaProbeError::AmbiguousRange);
    }
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_err() {
        return Err(UiaProbeError::ComInitialization);
    }
    let result = probe_inner(rendered_token);
    unsafe {
        CoUninitialize();
    }
    result
}

fn probe_inner(rendered_token: &str) -> Result<UiaEvidence, UiaProbeError> {
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| UiaProbeError::ProviderUnavailable)?;
    let element = unsafe { automation.GetFocusedElement() }
        .map_err(|_| UiaProbeError::ProviderUnavailable)?;
    inspect_element(&element, rendered_token)
}

fn inspect_element(
    element: &IUIAutomationElement,
    rendered_token: &str,
) -> Result<UiaEvidence, UiaProbeError> {
    let control_type =
        unsafe { element.CurrentControlType() }.map_err(|_| UiaProbeError::UnknownEvidence)?;
    if control_type != UIA_EditControlTypeId {
        return Err(UiaProbeError::UnsupportedControl);
    }
    let class_name =
        unsafe { element.CurrentClassName() }.map_err(|_| UiaProbeError::UnknownEvidence)?;
    if class_name.to_string() != "Edit" {
        return Err(UiaProbeError::UnsupportedControl);
    }
    if unsafe { element.CurrentIsPassword() }
        .map_err(|_| UiaProbeError::UnknownEvidence)?
        .as_bool()
    {
        return Err(UiaProbeError::SecureControl);
    }
    let pattern: IUIAutomationTextPattern =
        unsafe { element.GetCurrentPatternAs(UIA_TextPatternId) }
            .map_err(|_| UiaProbeError::ProviderUnavailable)?;
    let selections =
        unsafe { pattern.GetSelection() }.map_err(|_| UiaProbeError::SelectionUnavailable)?;
    if unsafe { selections.Length() }.map_err(|_| UiaProbeError::SelectionUnavailable)? != 1 {
        return Err(UiaProbeError::AmbiguousRange);
    }
    let caret =
        unsafe { selections.GetElement(0) }.map_err(|_| UiaProbeError::SelectionUnavailable)?;
    let selection_len = unsafe {
        caret.CompareEndpoints(
            TextPatternRangeEndpoint_Start,
            &caret,
            TextPatternRangeEndpoint_End,
        )
    }
    .map_err(|_| UiaProbeError::SelectionUnavailable)?;
    if selection_len != 0 {
        return Err(UiaProbeError::SelectionUnavailable);
    }
    let candidate = unsafe { caret.Clone() }.map_err(|_| UiaProbeError::AmbiguousRange)?;
    let count =
        i32::try_from(rendered_token.chars().count()).map_err(|_| UiaProbeError::AmbiguousRange)?;
    let moved = unsafe {
        candidate.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -count)
    }
    .map_err(|_| UiaProbeError::AmbiguousRange)?;
    if moved != count {
        return Err(UiaProbeError::AmbiguousRange);
    }
    let text: BSTR =
        unsafe { candidate.GetText(count) }.map_err(|_| UiaProbeError::TextMismatch)?;
    if text.to_string() != rendered_token {
        return Err(UiaProbeError::TextMismatch);
    }
    Ok(UiaEvidence {
        identity_available: true,
        caret_selection_available: true,
        selection_empty: true,
        exact_preceding_text: true,
        candidate_text_units: rendered_token.chars().count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    const EM_SETSEL: u32 = 0x00b1;
    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    use windows::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, ES_LEFT, ES_MULTILINE, ES_PASSWORD, SW_SHOW, SendMessageW,
        SetForegroundWindow, SetWindowTextW, ShowWindow, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    use windows::core::HSTRING;

    struct Fixture {
        parent: windows::Win32::Foundation::HWND,
        hwnd: windows::Win32::Foundation::HWND,
    }

    impl Fixture {
        fn new(class: windows::core::PCWSTR, text: &str, password: bool) -> Self {
            let parent = unsafe {
                CreateWindowExW(
                    windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
                    windows::core::w!("STATIC"),
                    windows::core::w!("ZonKey UIA fixture"),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    0,
                    0,
                    400,
                    80,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .expect("fixture parent creation");
            let style = WS_CHILD
                | WS_VISIBLE
                | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(ES_MULTILINE as u32)
                | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(ES_LEFT as u32)
                | if password {
                    windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(ES_PASSWORD as u32)
                } else {
                    windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(0)
                };
            let hwnd = unsafe {
                CreateWindowExW(
                    windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
                    class,
                    windows::core::w!("ZonKey UIA fixture"),
                    style,
                    0,
                    0,
                    400,
                    80,
                    Some(parent),
                    None,
                    None,
                    None,
                )
            }
            .expect("fixture window creation");
            unsafe { SetWindowTextW(hwnd, &HSTRING::from(text)) }.expect("fixture text");
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetActiveWindow(parent);
                let _ = SetForegroundWindow(parent);
                let _ = SetFocus(Some(hwnd));
            }
            Self { parent, hwnd }
        }

        fn set_selection(&self, start: usize, end: usize) {
            unsafe {
                SendMessageW(
                    self.hwnd,
                    EM_SETSEL,
                    Some(windows::Win32::Foundation::WPARAM(start)),
                    Some(windows::Win32::Foundation::LPARAM(
                        isize::try_from(end).expect("fixture selection fits LPARAM"),
                    )),
                );
            }
        }

        fn element(&self) -> IUIAutomationElement {
            let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            while unsafe {
                windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                    &raw mut message,
                    None,
                    0,
                    0,
                    windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
                )
            }
            .as_bool()
            {
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(
                        &raw const message,
                    );
                    windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&raw const message);
                }
            }
            let automation: IUIAutomation =
                unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
                    .expect("UIA automation");
            let parent = unsafe { automation.ElementFromHandle(self.parent) }
                .expect("fixture parent element");
            let condition = unsafe { automation.CreateTrueCondition() }.expect("fixture condition");
            unsafe {
                parent.FindFirst(
                    windows::Win32::UI::Accessibility::TreeScope_Children,
                    &condition,
                )
            }
            .expect("fixture element")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = unsafe { DestroyWindow(self.hwnd) };
            let _ = unsafe { DestroyWindow(self.parent) };
        }
    }

    fn with_com() {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        assert!(hr.is_ok());
    }

    #[test]
    #[ignore = "requires an interactive UI Automation text provider"]
    fn fixture_edit_exact_match_is_read_only() {
        let _lock = test_lock();
        with_com();
        let fixture = Fixture::new(windows::core::w!("EDIT"), "resume", false);
        fixture.set_selection(6, 6);
        let element = fixture.element();
        let result = inspect_element(&element, "resume").expect("exact fixture match");
        assert!(result.exact_preceding_text);
        assert_eq!(
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(fixture.hwnd) },
            6
        );
        unsafe {
            CoUninitialize();
        }
    }

    #[test]
    #[ignore = "requires an interactive UI Automation text provider"]
    fn fixture_text_mismatch_rejects_without_mutation() {
        let _lock = test_lock();
        with_com();
        let fixture = Fixture::new(windows::core::w!("EDIT"), "resume", false);
        fixture.set_selection(6, 6);
        let element = fixture.element();
        assert_eq!(
            inspect_element(&element, "other"),
            Err(UiaProbeError::TextMismatch)
        );
        assert_eq!(
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(fixture.hwnd) },
            6
        );
        unsafe {
            CoUninitialize();
        }
    }

    #[test]
    #[ignore = "requires an interactive UI Automation text provider"]
    fn fixture_non_empty_selection_rejects() {
        let _lock = test_lock();
        with_com();
        let fixture = Fixture::new(windows::core::w!("EDIT"), "resume", false);
        fixture.set_selection(0, 2);
        let element = fixture.element();
        assert_eq!(
            inspect_element(&element, "resume"),
            Err(UiaProbeError::SelectionUnavailable)
        );
        unsafe {
            CoUninitialize();
        }
    }

    #[test]
    #[ignore = "requires an interactive UI Automation text provider"]
    fn fixture_non_edit_rejects() {
        let _lock = test_lock();
        with_com();
        let fixture = Fixture::new(windows::core::w!("STATIC"), "resume", false);
        let element = fixture.element();
        assert_eq!(
            inspect_element(&element, "resume"),
            Err(UiaProbeError::UnsupportedControl)
        );
        unsafe {
            CoUninitialize();
        }
    }

    #[test]
    #[ignore = "requires an interactive UI Automation text provider"]
    fn fixture_password_rejects() {
        let _lock = test_lock();
        with_com();
        let fixture = Fixture::new(windows::core::w!("EDIT"), "resume", true);
        fixture.set_selection(6, 6);
        let element = fixture.element();
        assert_eq!(
            inspect_element(&element, "resume"),
            Err(UiaProbeError::SecureControl)
        );
        unsafe {
            CoUninitialize();
        }
    }

    #[test]
    fn native_fixture_exact_match_is_read_only() {
        let _lock = test_lock();
        let fixture = Fixture::new(windows::core::w!("EDIT"), "resume", false);
        fixture.set_selection(6, 6);
        let result = crate::probe_standard_edit(fixture.hwnd, "resume").expect("native match");
        assert!(result.exact_preceding_text && result.selection_empty);
        assert_eq!(
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(fixture.hwnd) },
            6
        );
    }

    #[test]
    fn native_fixture_mismatch_rejects_without_mutation() {
        let _lock = test_lock();
        let fixture = Fixture::new(windows::core::w!("EDIT"), "resume", false);
        fixture.set_selection(6, 6);
        assert_eq!(
            crate::probe_standard_edit(fixture.hwnd, "other"),
            Err(crate::native_edit::NativeEditProbeError::TextMismatch)
        );
        assert_eq!(
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(fixture.hwnd) },
            6
        );
    }

    #[test]
    fn native_fixture_selection_rejects() {
        let _lock = test_lock();
        let fixture = Fixture::new(windows::core::w!("EDIT"), "resume", false);
        fixture.set_selection(0, 2);
        assert_eq!(
            crate::probe_standard_edit(fixture.hwnd, "resume"),
            Err(crate::native_edit::NativeEditProbeError::ContradictorySelection)
        );
    }

    #[test]
    fn native_fixture_non_edit_rejects() {
        let _lock = test_lock();
        let fixture = Fixture::new(windows::core::w!("STATIC"), "resume", false);
        assert_eq!(
            crate::probe_standard_edit(fixture.hwnd, "resume"),
            Err(crate::native_edit::NativeEditProbeError::UnsupportedControl)
        );
    }

    #[test]
    fn native_fixture_password_rejects() {
        let _lock = test_lock();
        let fixture = Fixture::new(windows::core::w!("EDIT"), "resume", true);
        fixture.set_selection(6, 6);
        assert_eq!(
            crate::probe_standard_edit(fixture.hwnd, "resume"),
            Err(crate::native_edit::NativeEditProbeError::SecureControl)
        );
    }
}
