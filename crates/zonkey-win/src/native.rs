use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, MSG, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_QUIT,
};

use zonkey_service::{EventProcessor, EventSource, ObserveService, ProcessorClassification};
use zonkey_types::{InjectionOrigin, KeyEventKind, ObservedInputEvent, ObserverError};

use crate::{ModifierTracker, NativeKeyboardEvent, is_supported_message, map_native_event};

const CTRL_C_EVENT: u32 = 0;
const CTRL_BREAK_EVENT: u32 = 1;
const BRIDGE_WAKE_MESSAGE: u32 = WM_APP + 1;

static BRIDGE: OnceLock<Mutex<BridgeState>> = OnceLock::new();
static OBSERVER_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static CALLBACK_INVOCATIONS: AtomicU64 = AtomicU64::new(0);
static SUPPORTED_MESSAGES: AtomicU64 = AtomicU64::new(0);
static MAPPING_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static MAPPING_SUCCESS: AtomicU64 = AtomicU64::new(0);
static MAPPING_REJECTED: AtomicU64 = AtomicU64::new(0);
static BRIDGE_ENQUEUED: AtomicU64 = AtomicU64::new(0);
static BRIDGE_FULL_DROPS: AtomicU64 = AtomicU64::new(0);
static BRIDGE_LOCK_DROPS: AtomicU64 = AtomicU64::new(0);

struct BridgeState {
    events: VecDeque<NativeKeyboardEvent>,
    sequence: u64,
    dropped: u64,
    active: bool,
}

impl BridgeState {
    fn new() -> Self {
        Self {
            events: VecDeque::with_capacity(256),
            sequence: 0,
            dropped: 0,
            active: false,
        }
    }

    fn push(&mut self, event: NativeKeyboardEvent) -> bool {
        if self.events.len() >= 256 {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        self.events.push_back(event);
        true
    }
}

fn bridge() -> &'static Mutex<BridgeState> {
    BRIDGE.get_or_init(|| Mutex::new(BridgeState::new()))
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        CALLBACK_INVOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
    if code >= 0 && lparam.0 != 0 {
        // SAFETY: Windows supplies lParam as a pointer to KBDLLHOOKSTRUCT for
        // WH_KEYBOARD_LL callbacks. The value is copied immediately and never
        // retained beyond this callback.
        let native = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if let Ok(message) = u32::try_from(wparam.0) {
            if is_supported_message(message) {
                SUPPORTED_MESSAGES.fetch_add(1, Ordering::Relaxed);
            } else {
                MAPPING_REJECTED.fetch_add(1, Ordering::Relaxed);
            }
            if let Ok(mut state) = bridge().try_lock() {
                if state.active {
                    if let Some(sequence) = state.sequence.checked_add(1) {
                        state.sequence = sequence;
                        if state.push(NativeKeyboardEvent {
                            message,
                            virtual_key: native.vkCode,
                            flags: native.flags.0,
                            sequence,
                        }) {
                            BRIDGE_ENQUEUED.fetch_add(1, Ordering::Relaxed);
                            let thread_id = OBSERVER_THREAD_ID.load(Ordering::Relaxed);
                            if thread_id != 0 {
                                // SAFETY: The thread id belongs to the message-pumping
                                // observer thread. Posting a private wake message is
                                // non-blocking and does not alter keyboard input.
                                let _ = unsafe {
                                    PostThreadMessageW(
                                        thread_id,
                                        BRIDGE_WAKE_MESSAGE,
                                        WPARAM(0),
                                        LPARAM(0),
                                    )
                                };
                            }
                        } else {
                            BRIDGE_FULL_DROPS.fetch_add(1, Ordering::Relaxed);
                        }
                    } else {
                        state.dropped = state.dropped.saturating_add(1);
                        BRIDGE_FULL_DROPS.fetch_add(1, Ordering::Relaxed);
                    }
                }
            } else {
                BRIDGE_LOCK_DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    // SAFETY: Chaining the hook is required by the Windows hook contract. No
    // non-zero result is returned, so this observer never suppresses input.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn console_handler(control_type: u32) -> windows::core::BOOL {
    if matches!(control_type, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
        let thread_id = OBSERVER_THREAD_ID.load(Ordering::Relaxed);
        if thread_id != 0 {
            // SAFETY: The thread id was captured from the active observer
            // thread, and WM_QUIT is a process-local message-loop shutdown.
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

struct HookGuard(HHOOK);

impl Drop for HookGuard {
    fn drop(&mut self) {
        // SAFETY: The handle was returned by SetWindowsHookExW in this guard.
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
    }
}

pub struct WindowsObserveSource {
    _hook: HookGuard,
    tracker: ModifierTracker,
    stopped: bool,
    shutdown_requested: bool,
}

impl WindowsObserveSource {
    fn new() -> Result<Self, ObserverError> {
        {
            let mut state = bridge().lock().map_err(|_| ObserverError::Internal)?;
            if state.active {
                return Err(ObserverError::StartupFailed);
            }
            state.events.clear();
            state.sequence = 0;
            state.dropped = 0;
            state.active = false;
        }
        CALLBACK_INVOCATIONS.store(0, Ordering::Relaxed);
        SUPPORTED_MESSAGES.store(0, Ordering::Relaxed);
        MAPPING_ATTEMPTS.store(0, Ordering::Relaxed);
        MAPPING_SUCCESS.store(0, Ordering::Relaxed);
        MAPPING_REJECTED.store(0, Ordering::Relaxed);
        BRIDGE_ENQUEUED.store(0, Ordering::Relaxed);
        BRIDGE_FULL_DROPS.store(0, Ordering::Relaxed);
        BRIDGE_LOCK_DROPS.store(0, Ordering::Relaxed);
        // SAFETY: Passing None requests the current executable module handle;
        // it is used only to satisfy the global hook module contract.
        let module = unsafe { GetModuleHandleW(None) }.map_err(|error| {
            eprintln!(
                "hook=failed stage=module_handle error_code=0x{:08X}",
                error.code().0.cast_unsigned()
            );
            ObserverError::StartupFailed
        })?;
        // SAFETY: The callback has the required system ABI and is static for
        // the lifetime of the hook. The spike installs a desktop-wide
        // low-level hook and never suppresses input.
        let hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(low_level_keyboard_proc),
                Some(HINSTANCE(module.0)),
                0,
            )
        }
        .map_err(|error| {
            eprintln!(
                "hook=failed stage=install error_code=0x{:08X}",
                error.code().0.cast_unsigned()
            );
            ObserverError::StartupFailed
        })?;
        bridge().lock().map_err(|_| ObserverError::Internal)?.active = true;
        Ok(Self {
            _hook: HookGuard(hook),
            tracker: ModifierTracker::default(),
            stopped: false,
            shutdown_requested: false,
        })
    }

    fn dropped_events() -> u64 {
        bridge().lock().map_or(u64::MAX, |state| state.dropped)
    }

    fn next_mapped_event(&mut self) -> Option<ObservedInputEvent> {
        let native = if let Ok(mut state) = bridge().try_lock() {
            state.events.pop_front()?
        } else {
            BRIDGE_LOCK_DROPS.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        MAPPING_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        let Some((event, tracker)) = map_native_event(native, self.tracker) else {
            MAPPING_REJECTED.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        MAPPING_SUCCESS.fetch_add(1, Ordering::Relaxed);
        self.tracker = tracker;
        Some(event)
    }

    fn stop_accepting_events() {
        if let Ok(mut state) = bridge().lock() {
            state.active = false;
        }
    }
}

impl EventSource for WindowsObserveSource {
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
            let mut message = MSG::default();
            // SAFETY: `message` is a valid writable MSG and the call is made
            // on the thread that installed the hook, as required by Windows.
            let result = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
            if result.0 == -1 {
                self.stopped = true;
                Self::stop_accepting_events();
                return Err(ObserverError::EventSourceClosed);
            }
            if result.0 == 0 || message.message == WM_QUIT {
                Self::stop_accepting_events();
                self.shutdown_requested = true;
            }
        }
    }
}

impl Drop for WindowsObserveSource {
    fn drop(&mut self) {
        self.stopped = true;
        if let Ok(mut state) = bridge().lock() {
            state.active = false;
            state.events.clear();
        }
        OBSERVER_THREAD_ID.store(0, Ordering::Relaxed);
    }
}

struct DiagnosticProcessor;

impl EventProcessor for DiagnosticProcessor {
    fn reset_after_discontinuity(&mut self) {
        println!("discontinuity=true");
    }

    fn process(&mut self, event: &ObservedInputEvent) -> ProcessorClassification {
        let kind = match event.kind {
            KeyEventKind::KeyDown => "down",
            KeyEventKind::KeyUp => "up",
            KeyEventKind::SystemKeyDown => "sysdown",
            KeyEventKind::SystemKeyUp => "sysup",
        };
        let injected = match event.injection_origin {
            InjectionOrigin::PhysicalOrUnmarked | InjectionOrigin::Unknown => "no",
            InjectionOrigin::MarkedInjected | InjectionOrigin::LowerIntegrityInjected => "yes",
        };
        // Category only: never print the actual key, which would be a raw
        // keystroke/document-content diagnostic.
        let key = if event.key.letter_value().is_some() {
            "letter"
        } else if event.key.digit_value().is_some() {
            "digit"
        } else if event.key.punctuation_value().is_some() {
            "punctuation"
        } else if event.key.modifier_value().is_some() {
            "modifier"
        } else {
            "other"
        };
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

pub fn run_observe() -> Result<(), &'static str> {
    OBSERVER_THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    // SAFETY: The handler is a static function with the required system ABI;
    // it only posts WM_QUIT for Ctrl+C/Ctrl+Break and never edits input.
    unsafe { SetConsoleCtrlHandler(Some(console_handler), true) }
        .map_err(|_| "failed to install console stop handler")?;
    let Ok(mut source) = WindowsObserveSource::new() else {
        OBSERVER_THREAD_ID.store(0, Ordering::Relaxed);
        let _ = unsafe { SetConsoleCtrlHandler(Some(console_handler), false) };
        return Err("failed to install keyboard hook");
    };
    println!("ZonKey observe-only Windows spike");
    println!("observer_thread_started");
    println!("hook=installed message_loop=running");
    println!("Press Ctrl+C to stop");
    let mut processor = DiagnosticProcessor;
    let mut service = ObserveService::new(zonkey_service::ObserveQueue::default());
    let report = service.run(&mut source, &mut processor);
    let bridge_dropped = WindowsObserveSource::dropped_events();
    let callbacks = CALLBACK_INVOCATIONS.load(Ordering::Relaxed);
    let supported_messages = SUPPORTED_MESSAGES.load(Ordering::Relaxed);
    let mapping_attempts = MAPPING_ATTEMPTS.load(Ordering::Relaxed);
    let mapping_success = MAPPING_SUCCESS.load(Ordering::Relaxed);
    let mapping_rejected = MAPPING_REJECTED.load(Ordering::Relaxed);
    let bridge_enqueued = BRIDGE_ENQUEUED.load(Ordering::Relaxed);
    let bridge_full_drops = BRIDGE_FULL_DROPS.load(Ordering::Relaxed);
    let bridge_lock_drops = BRIDGE_LOCK_DROPS.load(Ordering::Relaxed);
    let _ = unsafe { SetConsoleCtrlHandler(Some(console_handler), false) };
    println!(
        "stopped status={:?} callbacks={} supported_messages={} mapping_attempts={} mapping_success={} mapping_rejected={} bridge_enqueued={} bridge_full_drops={} bridge_lock_drops={} received={} accepted={} dropped={} processed={}",
        service.status(),
        callbacks,
        supported_messages,
        mapping_attempts,
        mapping_success,
        mapping_rejected,
        bridge_enqueued,
        bridge_full_drops,
        bridge_lock_drops,
        report.received,
        report.accepted,
        report.dropped,
        report.processed
    );
    if bridge_dropped == u64::MAX || service.status() == zonkey_types::ObserverStatus::Failed {
        Err("observe source failed")
    } else {
        Ok(())
    }
}

/// Runs the same Windows observe source with a caller-owned, platform-neutral
/// processor. The adapter only orchestrates capture and service lifecycle.
pub fn run_observe_with_processor<P: EventProcessor>(mut processor: P) -> Result<(), &'static str> {
    OBSERVER_THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    unsafe { SetConsoleCtrlHandler(Some(console_handler), true) }
        .map_err(|_| "failed to install console stop handler")?;
    let Ok(mut source) = WindowsObserveSource::new() else {
        OBSERVER_THREAD_ID.store(0, Ordering::Relaxed);
        let _ = unsafe { SetConsoleCtrlHandler(Some(console_handler), false) };
        return Err("failed to install keyboard hook");
    };
    println!("ZonKey observe-only diagnostic mode");
    println!("observer_thread_started");
    println!("hook=installed message_loop=running");
    println!("Press Ctrl+C to stop");
    let mut service = ObserveService::new(zonkey_service::ObserveQueue::default());
    let report = service.run(&mut source, &mut processor);
    let bridge_dropped = WindowsObserveSource::dropped_events();
    let _ = unsafe { SetConsoleCtrlHandler(Some(console_handler), false) };
    println!(
        "stopped status={:?} received={} accepted={} dropped={} processed={} discontinuities={} source_failures={} unsupported_events={}",
        service.status(),
        report.received,
        report.accepted,
        report.dropped,
        report.processed,
        report.discontinuities,
        report.source_failures,
        report.unsupported_events,
    );
    if bridge_dropped == u64::MAX || service.status() == zonkey_types::ObserverStatus::Failed {
        Err("observe source failed")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{BridgeState, HookGuard, WindowsObserveSource, bridge};
    use crate::{ModifierTracker, NativeKeyboardEvent};
    use windows::Win32::UI::WindowsAndMessaging::HHOOK;
    use zonkey_service::EventSource;

    fn source_for_shutdown_drain() -> WindowsObserveSource {
        WindowsObserveSource {
            _hook: HookGuard(HHOOK::default()),
            tracker: ModifierTracker::default(),
            stopped: false,
            shutdown_requested: true,
        }
    }

    fn reset_bridge() {
        let mut state = bridge().lock().expect("bridge lock");
        *state = BridgeState::new();
    }

    #[test]
    fn queued_bridge_events_are_mapped_fifo_before_shutdown_exhaustion() {
        reset_bridge();
        {
            let mut state = bridge().lock().expect("bridge lock");
            assert!(state.push(NativeKeyboardEvent {
                message: 0x0100,
                virtual_key: 0x41,
                flags: 0,
                sequence: 1,
            }));
            assert!(state.push(NativeKeyboardEvent {
                message: 0x0101,
                virtual_key: 0x41,
                flags: 0,
                sequence: 2,
            }));
        }
        let mut source = source_for_shutdown_drain();
        let first = source.next_event().expect("first event").expect("event");
        let second = source.next_event().expect("second event").expect("event");
        assert_eq!(first.key.letter_value(), Some('A'));
        assert_eq!(first.kind, zonkey_types::KeyEventKind::KeyDown);
        assert_eq!(second.kind, zonkey_types::KeyEventKind::KeyUp);
        assert!(source.next_event().expect("exhaustion").is_none());
        assert!(source.next_event().expect("terminal exhaustion").is_none());
        reset_bridge();
    }
}
