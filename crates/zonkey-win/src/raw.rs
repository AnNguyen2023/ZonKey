use std::collections::VecDeque;
use std::mem::size_of;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Devices::HumanInterfaceDevice::{
    HID_USAGE_GENERIC_KEYBOARD, HID_USAGE_PAGE_GENERIC,
};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::{
    GetRawInputData, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK,
    RIM_TYPEKEYBOARD, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE,
    RegisterClassW, UnregisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INPUT, WM_QUIT, WNDCLASSW,
};
use windows::core::w;

use zonkey_service::{EventProcessor, EventSource, ObserveService, ProcessorClassification};
use zonkey_types::{InjectionOrigin, KeyEventKind, ObservedInputEvent, ObserverError};

use crate::{ModifierTracker, RawKeyboardEvent, map_raw_event};

const CTRL_C_EVENT: u32 = 0;
const CTRL_BREAK_EVENT: u32 = 1;
static RAW_THREAD_ID: AtomicU32 = AtomicU32::new(0);

static RAW_BRIDGE: OnceLock<Mutex<RawBridgeState>> = OnceLock::new();
static RAW_MESSAGES: AtomicU64 = AtomicU64::new(0);
static RAW_KEYBOARD_PACKETS: AtomicU64 = AtomicU64::new(0);
static RAW_MAPPING_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static RAW_MAPPING_SUCCESS: AtomicU64 = AtomicU64::new(0);
static RAW_MAPPING_REJECTED: AtomicU64 = AtomicU64::new(0);
static RAW_BRIDGE_ENQUEUED: AtomicU64 = AtomicU64::new(0);
static RAW_BRIDGE_FULL_DROPS: AtomicU64 = AtomicU64::new(0);

struct RawBridgeState {
    events: VecDeque<RawKeyboardEvent>,
    sequence: u64,
    active: bool,
}

impl RawBridgeState {
    fn new() -> Self {
        Self {
            events: VecDeque::with_capacity(256),
            sequence: 0,
            active: false,
        }
    }

    fn push(&mut self, event: RawKeyboardEvent) -> bool {
        if self.events.len() >= 256 {
            return false;
        }
        self.events.push_back(event);
        true
    }
}

fn raw_bridge() -> &'static Mutex<RawBridgeState> {
    RAW_BRIDGE.get_or_init(|| Mutex::new(RawBridgeState::new()))
}

unsafe extern "system" fn raw_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_INPUT {
        RAW_MESSAGES.fetch_add(1, Ordering::Relaxed);
        let hrawinput = windows::Win32::UI::Input::HRAWINPUT(lparam.0 as _);
        let mut size = 0_u32;
        let header_size =
            u32::try_from(size_of::<RAWINPUTHEADER>()).expect("RAWINPUTHEADER size fits in u32");
        // SAFETY: The handle and size query are supplied by Windows in WM_INPUT.
        let queried =
            unsafe { GetRawInputData(hrawinput, RID_INPUT, None, &raw mut size, header_size) };
        if queried != u32::MAX && size > 0 && (size as usize) <= size_of::<RAWINPUT>() {
            let mut raw = RAWINPUT::default();
            // SAFETY: `raw` is writable storage large enough for the bounded packet.
            let copied = unsafe {
                GetRawInputData(
                    hrawinput,
                    RID_INPUT,
                    Some((&raw mut raw).cast()),
                    &raw mut size,
                    header_size,
                )
            };
            if copied != u32::MAX && raw.header.dwType == RIM_TYPEKEYBOARD.0 {
                // SAFETY: The packet type was checked before reading its keyboard union arm.
                let keyboard = unsafe { raw.data.keyboard };
                RAW_KEYBOARD_PACKETS.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut state) = raw_bridge().try_lock()
                    && state.active
                {
                    if let Some(sequence) = state.sequence.checked_add(1) {
                        state.sequence = sequence;
                        let event = RawKeyboardEvent {
                            virtual_key: u32::from(keyboard.VKey),
                            flags: u32::from(keyboard.Flags),
                            message: keyboard.Message,
                            sequence,
                        };
                        if state.push(event) {
                            RAW_BRIDGE_ENQUEUED.fetch_add(1, Ordering::Relaxed);
                        } else {
                            RAW_BRIDGE_FULL_DROPS.fetch_add(1, Ordering::Relaxed);
                        }
                    } else {
                        RAW_BRIDGE_FULL_DROPS.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }
    // SAFETY: Unhandled messages are delegated to the default window procedure.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn console_handler(control_type: u32) -> windows::core::BOOL {
    if matches!(control_type, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
        let thread_id = RAW_THREAD_ID.load(Ordering::Relaxed);
        if thread_id != 0 {
            // SAFETY: The id belongs to the active Raw Input message-loop thread.
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    thread_id,
                    WM_QUIT,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }
        return windows::core::BOOL(1);
    }
    windows::core::BOOL(0)
}

struct RawWindowGuard {
    hwnd: HWND,
    class_instance: HINSTANCE,
}

impl Drop for RawWindowGuard {
    fn drop(&mut self) {
        // SAFETY: The window was created by this observer and is destroyed once.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = UnregisterClassW(w!("ZonKeyRawInputWindow"), Some(self.class_instance));
        }
        if let Ok(mut state) = raw_bridge().lock() {
            state.active = false;
            state.events.clear();
        }
        RAW_THREAD_ID.store(0, Ordering::Relaxed);
    }
}

pub struct RawInputSource {
    _window: RawWindowGuard,
    tracker: ModifierTracker,
    stopped: bool,
    shutdown_requested: bool,
}

impl RawInputSource {
    fn new() -> Result<Self, ObserverError> {
        let instance =
            unsafe { GetModuleHandleW(None) }.map_err(|_| ObserverError::StartupFailed)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(raw_window_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: w!("ZonKeyRawInputWindow"),
            ..Default::default()
        };
        // SAFETY: The class structure contains a static callback and valid module/name.
        let _ = unsafe { RegisterClassW(&raw const class) };
        // SAFETY: HWND_MESSAGE creates a hidden message-only window with no visible UI.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("ZonKeyRawInputWindow"),
                w!("ZonKeyRawInputWindow"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(HINSTANCE(instance.0)),
                None,
            )
        }
        .map_err(|_| ObserverError::StartupFailed)?;
        let devices = [RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC,
            usUsage: HID_USAGE_GENERIC_KEYBOARD,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        }];
        // SAFETY: The device descriptor is a valid keyboard registration and does not use
        // RIDEV_NOLEGACY, so normal focused-application keyboard delivery is preserved.
        let device_size =
            u32::try_from(size_of::<RAWINPUTDEVICE>()).expect("RAWINPUTDEVICE size fits in u32");
        unsafe { RegisterRawInputDevices(&devices, device_size) }
            .map_err(|_| ObserverError::StartupFailed)?;
        if let Ok(mut state) = raw_bridge().lock() {
            state.events.clear();
            state.sequence = 0;
            state.active = true;
        }
        Ok(Self {
            _window: RawWindowGuard {
                hwnd,
                class_instance: HINSTANCE(instance.0),
            },
            tracker: ModifierTracker::default(),
            stopped: false,
            shutdown_requested: false,
        })
    }

    fn next_mapped_event(&mut self) -> Option<ObservedInputEvent> {
        let raw = raw_bridge().try_lock().ok()?.events.pop_front()?;
        RAW_MAPPING_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        let Some((event, tracker)) = map_raw_event(raw, self.tracker) else {
            RAW_MAPPING_REJECTED.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        RAW_MAPPING_SUCCESS.fetch_add(1, Ordering::Relaxed);
        self.tracker = tracker;
        Some(event)
    }

    fn stop_accepting_events() {
        if let Ok(mut state) = raw_bridge().lock() {
            state.active = false;
        }
    }
}

impl EventSource for RawInputSource {
    fn next_event(&mut self) -> Result<Option<ObservedInputEvent>, ObserverError> {
        if self.stopped {
            return Ok(None);
        }
        loop {
            if let Some(event) = self.next_mapped_event() {
                return Ok(Some(event));
            }
            if self.shutdown_requested {
                self.stopped = true;
                return Ok(None);
            }
            let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            // SAFETY: The message buffer is valid and belongs to this message-loop thread.
            let result = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
            if result.0 == -1 {
                self.stopped = true;
                Self::stop_accepting_events();
                return Err(ObserverError::EventSourceClosed);
            }
            if result.0 == 0 || message.message == WM_QUIT {
                Self::stop_accepting_events();
                self.shutdown_requested = true;
            } else {
                // SAFETY: Dispatches the message to the registered message-only window.
                unsafe { DispatchMessageW(&raw const message) };
            }
        }
    }
}

struct RawProcessor;

impl EventProcessor for RawProcessor {
    fn reset_after_discontinuity(&mut self) {}

    fn process(&mut self, event: &ObservedInputEvent) -> ProcessorClassification {
        let kind = match event.kind {
            KeyEventKind::KeyDown => "down",
            KeyEventKind::KeyUp => "up",
            KeyEventKind::SystemKeyDown => "sysdown",
            KeyEventKind::SystemKeyUp => "sysup",
        };
        let key = event
            .key
            .letter_value()
            .map(|value| value.to_string())
            .or_else(|| event.key.digit_value().map(|value| value.to_string()))
            .or_else(|| event.key.punctuation_value().map(|value| value.to_string()))
            .or_else(|| event.key.modifier_value().map(|_| "modifier".to_owned()))
            .unwrap_or_else(|| "other".to_owned());
        let mut modifiers = String::new();
        if event.modifiers.shift() {
            modifiers.push_str("SHIFT ");
        }
        if event.modifiers.control() {
            modifiers.push_str("CTRL ");
        }
        if event.modifiers.alt() {
            modifiers.push_str("ALT ");
        }
        if event.modifiers.meta() {
            modifiers.push_str("META ");
        }
        if modifiers.is_empty() {
            modifiers.push_str("none");
        }
        let injected = match event.injection_origin {
            InjectionOrigin::PhysicalOrUnmarked | InjectionOrigin::Unknown => "no",
            InjectionOrigin::MarkedInjected | InjectionOrigin::LowerIntegrityInjected => "yes",
        };
        println!(
            "seq={} kind={} key={} mods={} injected={}",
            event.sequence.get(),
            kind,
            key,
            modifiers,
            injected
        );
        ProcessorClassification::Observed
    }
}

pub fn run_observe_raw() -> Result<(), &'static str> {
    RAW_MESSAGES.store(0, Ordering::Relaxed);
    RAW_KEYBOARD_PACKETS.store(0, Ordering::Relaxed);
    RAW_MAPPING_ATTEMPTS.store(0, Ordering::Relaxed);
    RAW_MAPPING_SUCCESS.store(0, Ordering::Relaxed);
    RAW_MAPPING_REJECTED.store(0, Ordering::Relaxed);
    RAW_BRIDGE_ENQUEUED.store(0, Ordering::Relaxed);
    RAW_BRIDGE_FULL_DROPS.store(0, Ordering::Relaxed);
    RAW_THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    // SAFETY: The handler only posts WM_QUIT and does not alter input.
    unsafe { SetConsoleCtrlHandler(Some(console_handler), true) }
        .map_err(|_| "failed to install raw stop handler")?;
    let Ok(mut source) = RawInputSource::new() else {
        let _ = unsafe { SetConsoleCtrlHandler(Some(console_handler), false) };
        RAW_THREAD_ID.store(0, Ordering::Relaxed);
        return Err("failed to install Raw Input observer");
    };
    println!("ZonKey observe-only Raw Input spike");
    println!("raw_registration=keyboard input_sink=true message_loop=running");
    println!("Press Ctrl+C to stop");
    let mut processor = RawProcessor;
    let mut service = ObserveService::new(zonkey_service::ObserveQueue::default());
    let report = service.run(&mut source, &mut processor);
    let _ = unsafe { SetConsoleCtrlHandler(Some(console_handler), false) };
    println!(
        "stopped status={:?} raw_messages={} raw_keyboard_packets={} mapping_attempts={} mapping_success={} mapping_rejected={} bridge_enqueued={} bridge_full_drops={} received={} processed={}",
        service.status(),
        RAW_MESSAGES.load(Ordering::Relaxed),
        RAW_KEYBOARD_PACKETS.load(Ordering::Relaxed),
        RAW_MAPPING_ATTEMPTS.load(Ordering::Relaxed),
        RAW_MAPPING_SUCCESS.load(Ordering::Relaxed),
        RAW_MAPPING_REJECTED.load(Ordering::Relaxed),
        RAW_BRIDGE_ENQUEUED.load(Ordering::Relaxed),
        RAW_BRIDGE_FULL_DROPS.load(Ordering::Relaxed),
        report.received,
        report.processed,
    );
    if service.status() == zonkey_types::ObserverStatus::Failed {
        Err("Raw Input observer failed")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{RawBridgeState, RawKeyboardEvent};

    #[test]
    fn raw_bridge_is_bounded_and_rejects_newest_event() {
        let mut bridge = RawBridgeState::new();
        for sequence in 1..=256 {
            assert!(bridge.push(RawKeyboardEvent {
                virtual_key: 0x41,
                flags: 0,
                message: 0x0100,
                sequence,
            }));
        }
        assert!(!bridge.push(RawKeyboardEvent {
            virtual_key: 0x4b,
            flags: 0,
            message: 0x0100,
            sequence: 257,
        }));
        assert_eq!(bridge.events.len(), 256);
        assert_eq!(bridge.events.front().map(|event| event.sequence), Some(1));
    }
}
