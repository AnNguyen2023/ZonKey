#![allow(clippy::doc_markdown, clippy::struct_excessive_bools)]

//! M3D-18 design-only capability probe: read-only UIA measurements of a
//! real VS Code desktop text editor element.
//!
//! The probe walks the UIA control-view subtree under the editor host window
//! with a bounded budget and reports sanitized structural facts only (depth,
//! control type, class, pattern availability, `GetActiveComposition`
//! status). It never reads element names or document text and never mutates
//! any state.
//!
//! `IUIAutomationTextPattern2` has no binding in windows-rs 0.62, so
//! TextPattern2 availability is intentionally not measured here; it is a
//! caret-evidence signal, not a composition signal, and does not affect the
//! M3D-18 verdict.

#[cfg(windows)]
mod probe {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextEditPattern,
        IUIAutomationTextPattern, IUIAutomationTreeWalker, UIA_DocumentControlTypeId,
        UIA_TextEditPatternId, UIA_TextPatternId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, GetClassNameW, GetWindowTextW, IsWindowVisible,
    };
    use windows::core::BOOL;

    const TOP_CLASS: &str = "Chrome_WidgetWin_1";
    const HOST_CLASS: &str = "Chrome_RenderWidgetHostHWND";
    const TITLE_MARKER: &str = "zonkey-uia-probe";
    const MAX_DEPTH: usize = 8;
    const MAX_VISITED: usize = 400;
    const MAX_REPORTS: usize = 6;

    /// Sanitized structural facts for one visited element; no document text.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ElementReport {
        pub depth: usize,
        pub control_type: i32,
        pub is_document_control: bool,
        pub element_class: String,
        pub automation_id: Option<String>,
        pub text_pattern: bool,
        /// TextPattern2 is not measured (no windows-rs binding); see module docs.
        pub text_pattern2_measured: bool,
        pub text_edit_pattern: bool,
        pub active_composition: Option<CompositionProbe>,
    }

    /// Outcome of `GetActiveComposition` without reading any range text.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum CompositionProbe {
        /// The provider returned a range handle; dropped unread.
        ReturnedRange,
        /// HRESULT `S_OK` with a null range: the provider answered that no
        /// composition is active. This is the measurable NONE state.
        NoActiveComposition,
        /// The provider returned a non-zero HRESULT; the raw code is
        /// preserved so ERROR versus UNKNOWN can be classified from evidence.
        Erred(i32),
    }

    /// Bounded sanitized snapshot of the editor subtree capability.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ProbeOutcome {
        pub top_window_class: String,
        pub editor_host_class: String,
        pub visited: usize,
        pub reports: Vec<ElementReport>,
    }

    fn window_class(hwnd: HWND) -> String {
        let mut buffer = [0u16; 64];
        let len = usize::try_from(unsafe { GetClassNameW(hwnd, &mut buffer) }).unwrap_or(0);
        String::from_utf16_lossy(&buffer[..len.min(buffer.len())])
    }

    fn window_title(hwnd: HWND) -> String {
        let mut buffer = [0u16; 256];
        let len = usize::try_from(unsafe { GetWindowTextW(hwnd, &mut buffer) }).unwrap_or(0);
        String::from_utf16_lossy(&buffer[..len.min(buffer.len())])
    }

    unsafe extern "system" fn enum_top_windows(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let found = &mut *(lparam.0 as *mut Vec<HWND>);
            if IsWindowVisible(hwnd).as_bool()
                && window_class(hwnd) == TOP_CLASS
                && window_title(hwnd).contains(TITLE_MARKER)
            {
                found.push(hwnd);
            }
            BOOL::from(true)
        }
    }

    unsafe extern "system" fn enum_child_windows_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let found = &mut *(lparam.0 as *mut Vec<HWND>);
            if window_class(hwnd) == HOST_CLASS {
                found.push(hwnd);
            }
            BOOL::from(true)
        }
    }

    fn sanitize_automation_id(raw: String) -> Option<String> {
        (raw.len() <= 64
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .then_some(raw)
    }

    fn measure_element(element: &IUIAutomationElement, depth: usize) -> ElementReport {
        let control_type = unsafe { element.CurrentControlType() }.map_or(-1, |value| value.0);
        let element_class = unsafe { element.CurrentClassName() }
            .map(|value| value.to_string())
            .unwrap_or_default();
        let automation_id = unsafe { element.CurrentAutomationId() }
            .ok()
            .map(|value| value.to_string())
            .and_then(sanitize_automation_id);
        let text_pattern =
            unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
                .is_ok();
        let (text_edit_pattern, active_composition) = match unsafe {
            element.GetCurrentPatternAs::<IUIAutomationTextEditPattern>(UIA_TextEditPatternId)
        } {
            Ok(pattern) => match unsafe { pattern.GetActiveComposition() } {
                // The range handle is dropped unread.
                Ok(_) => (true, Some(CompositionProbe::ReturnedRange)),
                Err(error) => {
                    let code = error.code().0;
                    let probe = if code == 0 {
                        // S_OK with a null range means no composition.
                        CompositionProbe::NoActiveComposition
                    } else {
                        CompositionProbe::Erred(code)
                    };
                    (true, Some(probe))
                }
            },
            Err(_) => (false, None),
        };
        ElementReport {
            depth,
            control_type,
            is_document_control: control_type == UIA_DocumentControlTypeId.0,
            element_class,
            automation_id,
            text_pattern,
            text_pattern2_measured: false,
            text_edit_pattern,
            active_composition,
        }
    }

    struct WalkState {
        walker: IUIAutomationTreeWalker,
        visited: usize,
        reports: Vec<ElementReport>,
    }

    fn walk(element: &IUIAutomationElement, depth: usize, state: &mut WalkState) {
        if depth > MAX_DEPTH || state.visited >= MAX_VISITED || state.reports.len() >= MAX_REPORTS {
            return;
        }
        state.visited += 1;
        let report = measure_element(element, depth);
        let candidate = report.is_document_control || report.text_edit_pattern;
        if candidate {
            state.reports.push(report);
        }
        let walker = state.walker.clone();
        let mut child = unsafe { walker.GetFirstChildElement(element) }.ok();
        while let Some(current) = child {
            walk(&current, depth + 1, state);
            if state.reports.len() >= MAX_REPORTS {
                return;
            }
            child = unsafe { walker.GetNextSiblingElement(&current) }.ok();
        }
    }

    /// Walks the bounded editor subtree of the visible VS Code window whose
    /// title contains the probe file name. The caller must have COM active.
    pub fn probe_editor() -> Result<ProbeOutcome, String> {
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
                .map_err(|error| format!("automation unavailable: {error}"))?;
        let mut tops: Vec<HWND> = Vec::new();
        let _ = unsafe { EnumWindows(Some(enum_top_windows), LPARAM(&raw mut tops as isize)) };
        let top = tops
            .first()
            .ok_or_else(|| "no visible VS Code window titled with the probe file".to_string())?;
        let mut hosts: Vec<HWND> = Vec::new();
        let _ = unsafe {
            EnumChildWindows(
                Some(*top),
                Some(enum_child_windows_cb),
                LPARAM(&raw mut hosts as isize),
            )
        };
        let host = hosts
            .first()
            .ok_or_else(|| "no Chrome_RenderWidgetHostHWND child window".to_string())?;
        let element: IUIAutomationElement = unsafe { automation.ElementFromHandle(*host) }
            .map_err(|error| format!("element from handle: {error}"))?;
        let walker = unsafe { automation.ControlViewWalker() }
            .map_err(|error| format!("control view walker unavailable: {error}"))?;
        let mut state = WalkState {
            walker,
            visited: 0,
            reports: Vec::new(),
        };
        walk(&element, 0, &mut state);
        Ok(ProbeOutcome {
            top_window_class: TOP_CLASS.to_owned(),
            editor_host_class: HOST_CLASS.to_owned(),
            visited: state.visited,
            reports: state.reports,
        })
    }

    /// Initializes COM, runs the read-only probe, and releases COM.
    pub fn run_probe() -> Result<ProbeOutcome, String> {
        if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
            return Err("COM initialization failed".to_string());
        }
        let report = probe_editor();
        unsafe { CoUninitialize() };
        report
    }
}

#[cfg(windows)]
mod tests {
    use super::probe::run_probe;

    /// M3D-18 manual evidence: launch the cached test-electron VS Code with
    /// the probe file open, then run with `cargo test -p zonkey-win
    /// vscode_editor -- --ignored --nocapture`. Chromium enables renderer
    /// accessibility asynchronously after the first UIA contact, so the
    /// probe retries with a delay until the subtree is observable.
    #[test]
    #[ignore = "requires a real VS Code desktop window showing zonkey-uia-probe.txt"]
    fn vscode_editor_uia_capability_probe_reports_sanitized_facts() {
        let mut report = None;
        for attempt in 1..=4 {
            let outcome = run_probe().expect("read-only UIA probe over the VS Code editor");
            println!(
                "zonkey_uia_attempt={attempt} visited={} reports={}",
                outcome.visited,
                outcome.reports.len()
            );
            if outcome.visited > 1 && !outcome.reports.is_empty() {
                report = Some(outcome);
                break;
            }
            report = Some(outcome);
            std::thread::sleep(std::time::Duration::from_millis(2500));
        }
        let report = report.expect("at least one probe attempt");
        println!("zonkey_uia_report={report:#?}");
        // Observation-only: no assertion on availability; the printed report
        // is the M3D-18 evidence and must contain no document text.
    }
}
