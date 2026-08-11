//! Windows observe-only adapter boundary.

#![cfg_attr(windows, allow(unsafe_code))]

use zonkey_types::EditPlan;

use zonkey_types::{
    EventSequence, InjectionOrigin, KeyEventKind, ModifierKey, ModifierState, ObservedInputEvent,
    ObservedKey,
};

/// Records that an edit plan reached the unimplemented Windows boundary.
///
/// # Errors
///
/// Returns an error when insertion text contains a NUL character.
pub fn validate_plan(plan: &EditPlan) -> Result<(), &'static str> {
    if plan.insert_text.contains('\0') {
        return Err("insert_text must not contain NUL");
    }
    Ok(())
}

/// Minimal native metadata copied before domain mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeKeyboardEvent {
    /// One of the keyboard message values delivered to the low-level hook.
    pub message: u32,
    /// Virtual-key value from the native event.
    pub virtual_key: u32,
    /// Native low-level keyboard flags.
    pub flags: u32,
    /// Monotonic sequence assigned by the adapter.
    pub sequence: u64,
}

pub(crate) const fn is_supported_message(message: u32) -> bool {
    matches!(message, 0x0100 | 0x0101 | 0x0104 | 0x0105)
}

/// Tracks only modifier transitions needed by the neutral event contract.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifierTracker {
    shift_generic: bool,
    shift_left: bool,
    shift_right: bool,
    control_generic: bool,
    control_left: bool,
    control_right: bool,
    alt_generic: bool,
    alt_left: bool,
    alt_right: bool,
    meta_left: bool,
    meta_right: bool,
}

impl ModifierTracker {
    /// Applies a modifier transition and returns the resulting state.
    #[must_use]
    pub const fn observe(mut self, virtual_key: u32, kind: KeyEventKind) -> Self {
        let active = !matches!(kind, KeyEventKind::KeyUp | KeyEventKind::SystemKeyUp);
        match virtual_key {
            0x10 => self.shift_generic = active,
            0xa0 => self.shift_left = active,
            0xa1 => self.shift_right = active,
            0x11 => self.control_generic = active,
            0xa2 => self.control_left = active,
            0xa3 => self.control_right = active,
            0x12 => self.alt_generic = active,
            0xa4 => self.alt_left = active,
            0xa5 => self.alt_right = active,
            0x5b => self.meta_left = active,
            0x5c => self.meta_right = active,
            _ => {}
        }
        self
    }

    /// Returns the neutral modifier value.
    #[must_use]
    pub const fn state(self) -> ModifierState {
        ModifierState::new()
            .with_shift(self.shift_generic || self.shift_left || self.shift_right)
            .with_control(self.control_generic || self.control_left || self.control_right)
            .with_alt(self.alt_generic || self.alt_left || self.alt_right)
            .with_meta(self.meta_left || self.meta_right)
    }
}

/// Converts copied native metadata into the existing neutral event contract.
///
/// The function performs no Unicode reconstruction and has no Windows API
/// dependency, so its mapping behavior is deterministic on every platform.
#[must_use]
pub fn map_native_event(
    native: NativeKeyboardEvent,
    tracker: ModifierTracker,
) -> Option<(ObservedInputEvent, ModifierTracker)> {
    let kind = match native.message {
        0x0100 => KeyEventKind::KeyDown,
        0x0101 => KeyEventKind::KeyUp,
        0x0104 => KeyEventKind::SystemKeyDown,
        0x0105 => KeyEventKind::SystemKeyUp,
        _ => return None,
    };
    let next_tracker = tracker.observe(native.virtual_key, kind);
    let key = map_key(native.virtual_key)?;
    let sequence = EventSequence::new(native.sequence).ok()?;
    let injection_origin = if native.flags & 0x02 != 0 {
        InjectionOrigin::LowerIntegrityInjected
    } else if native.flags & 0x10 != 0 {
        InjectionOrigin::MarkedInjected
    } else {
        InjectionOrigin::PhysicalOrUnmarked
    };
    Some((
        ObservedInputEvent {
            key,
            kind,
            modifiers: next_tracker.state(),
            injection_origin,
            sequence,
        },
        next_tracker,
    ))
}

fn map_key(virtual_key: u32) -> Option<ObservedKey> {
    match virtual_key {
        0x41..=0x5a => ObservedKey::letter(char::from_u32(virtual_key)?).ok(),
        0x30..=0x39 => ObservedKey::digit(u8::try_from(virtual_key - 0x30).ok()?).ok(),
        0x20 => Some(ObservedKey::space()),
        0x0d => Some(ObservedKey::enter()),
        0x09 => Some(ObservedKey::tab()),
        0x08 => Some(ObservedKey::backspace()),
        0x1b => Some(ObservedKey::escape()),
        0x10 | 0xa0 | 0xa1 => Some(ObservedKey::modifier(ModifierKey::Shift)),
        0x11 | 0xa2 | 0xa3 => Some(ObservedKey::modifier(ModifierKey::Control)),
        0x12 | 0xa4 | 0xa5 => Some(ObservedKey::modifier(ModifierKey::Alt)),
        0x5b | 0x5c => Some(ObservedKey::modifier(ModifierKey::Meta)),
        _ => Some(ObservedKey::other()),
    }
}

#[cfg(windows)]
mod native;

/// Runs the Windows observe-only manual spike.
///
/// # Errors
///
/// Returns a redacted error when the Windows hook, stop handler, or observer
/// service cannot start or complete.
#[cfg(windows)]
pub fn run_observe() -> Result<(), &'static str> {
    native::run_observe()
}

/// Windows observation is unavailable on non-Windows hosts.
///
/// # Errors
///
/// Always returns an unsupported-platform error on non-Windows hosts.
#[cfg(not(windows))]
pub fn run_observe() -> Result<(), &'static str> {
    Err("Windows observe-only spike requires a Windows host")
}

#[cfg(test)]
mod tests {
    use super::{ModifierTracker, NativeKeyboardEvent, map_native_event, validate_plan};
    use zonkey_types::EditPlan;

    #[test]
    fn rejects_nul_text() {
        let plan = EditPlan {
            delete_graphemes: 0,
            insert_text: "a\0b".into(),
        };
        assert!(validate_plan(&plan).is_err());
    }

    #[test]
    fn maps_key_transitions_and_injection_flags() {
        let native = NativeKeyboardEvent {
            message: 0x0100,
            virtual_key: 0x41,
            flags: 0x10,
            sequence: 1,
        };
        let (event, _) = map_native_event(native, ModifierTracker::default()).unwrap();
        assert_eq!(event.key.letter_value(), Some('A'));
        assert_eq!(event.kind, zonkey_types::KeyEventKind::KeyDown);
        assert_eq!(
            event.injection_origin,
            zonkey_types::InjectionOrigin::MarkedInjected
        );
        assert_eq!(event.sequence.get(), 1);
    }

    #[test]
    fn tracks_modifier_transitions_without_async_state_queries() {
        let shift_down = NativeKeyboardEvent {
            message: 0x0100,
            virtual_key: 0x10,
            flags: 0,
            sequence: 1,
        };
        let (event, tracker) = map_native_event(shift_down, ModifierTracker::default()).unwrap();
        assert!(event.modifiers.shift());
        let letter = NativeKeyboardEvent {
            message: 0x0100,
            virtual_key: 0x41,
            flags: 0,
            sequence: 2,
        };
        let (event, _) = map_native_event(letter, tracker).unwrap();
        assert!(event.modifiers.shift());
    }

    #[test]
    fn maps_special_keys_and_lower_integrity_injection() {
        for (virtual_key, expected) in [
            (0x30, Some(0)),
            (0x20, None),
            (0x0d, None),
            (0x09, None),
            (0x08, None),
            (0x1b, None),
        ] {
            let (event, _) = map_native_event(
                NativeKeyboardEvent {
                    message: 0x0100,
                    virtual_key,
                    flags: 0x02,
                    sequence: u64::from(virtual_key),
                },
                ModifierTracker::default(),
            )
            .unwrap();
            if let Some(digit) = expected {
                assert_eq!(event.key.digit_value(), Some(digit));
            } else {
                assert!(event.key.letter_value().is_none());
            }
            assert_eq!(
                event.injection_origin,
                zonkey_types::InjectionOrigin::LowerIntegrityInjected
            );
        }
    }

    #[test]
    fn tracks_control_and_alt_and_keeps_sequences_non_zero() {
        let (control, tracker) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0100,
                virtual_key: 0x11,
                flags: 0,
                sequence: 1,
            },
            ModifierTracker::default(),
        )
        .unwrap();
        assert!(control.modifiers.control());
        let (alt, _) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0100,
                virtual_key: 0x12,
                flags: 0,
                sequence: 2,
            },
            tracker,
        )
        .unwrap();
        assert!(alt.modifiers.control() && alt.modifiers.alt());
        assert_ne!(control.sequence.get(), 0);
        assert_ne!(alt.sequence.get(), 0);
    }

    #[test]
    fn maps_left_right_modifier_variants() {
        for virtual_key in [0xa2, 0xa3, 0xa4, 0xa5, 0xa0, 0xa1, 0x5b, 0x5c] {
            for (message, sequence) in [(0x0100, 1), (0x0101, 2)] {
                let (event, _) = map_native_event(
                    NativeKeyboardEvent {
                        message,
                        virtual_key,
                        flags: 0,
                        sequence,
                    },
                    ModifierTracker::default(),
                )
                .unwrap();
                assert!(event.key.modifier_value().is_some());
            }
        }
    }

    #[test]
    fn left_control_is_carried_through_character_and_cleared_on_release() {
        let (control_down, tracker) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0100,
                virtual_key: 0xa2,
                flags: 0,
                sequence: 1,
            },
            ModifierTracker::default(),
        )
        .unwrap();
        assert!(control_down.modifiers.control());
        let (character_down, tracker) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0100,
                virtual_key: 0x43,
                flags: 0,
                sequence: 2,
            },
            tracker,
        )
        .unwrap();
        let (character_up, tracker) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0101,
                virtual_key: 0x43,
                flags: 0,
                sequence: 3,
            },
            tracker,
        )
        .unwrap();
        let (control_up, _) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0101,
                virtual_key: 0xa2,
                flags: 0,
                sequence: 4,
            },
            tracker,
        )
        .unwrap();
        assert!(character_down.modifiers.control());
        assert!(character_up.modifiers.control());
        assert!(!control_up.modifiers.control());
    }

    #[test]
    fn overlapping_left_and_right_modifiers_remain_active_until_both_release() {
        let (_, tracker) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0100,
                virtual_key: 0xa0,
                flags: 0,
                sequence: 1,
            },
            ModifierTracker::default(),
        )
        .unwrap();
        let (right_down, tracker) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0100,
                virtual_key: 0xa1,
                flags: 0,
                sequence: 2,
            },
            tracker,
        )
        .unwrap();
        let (_, tracker) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0101,
                virtual_key: 0xa0,
                flags: 0,
                sequence: 3,
            },
            tracker,
        )
        .unwrap();
        let (right_up, _) = map_native_event(
            NativeKeyboardEvent {
                message: 0x0101,
                virtual_key: 0xa1,
                flags: 0,
                sequence: 4,
            },
            tracker,
        )
        .unwrap();
        assert!(right_down.modifiers.shift());
        assert!(!right_up.modifiers.shift());
    }

    #[test]
    fn rejects_unknown_messages_and_zero_sequences() {
        assert!(
            map_native_event(
                NativeKeyboardEvent {
                    message: 0x1234,
                    virtual_key: 0x41,
                    flags: 0,
                    sequence: 1,
                },
                ModifierTracker::default(),
            )
            .is_none()
        );
        assert!(
            map_native_event(
                NativeKeyboardEvent {
                    message: 0x0100,
                    virtual_key: 0x41,
                    flags: 0,
                    sequence: 0,
                },
                ModifierTracker::default(),
            )
            .is_none()
        );
    }

    #[test]
    fn maps_complete_callback_metadata_for_common_keys() {
        for (message, virtual_key) in [
            (0x0100, 0x41),
            (0x0101, 0x41),
            (0x0100, 0x4b),
            (0x0100, 0x10),
            (0x0100, 0xa0),
            (0x0100, 0xa1),
        ] {
            assert!(
                map_native_event(
                    NativeKeyboardEvent {
                        message,
                        virtual_key,
                        flags: 0,
                        sequence: 1
                    },
                    ModifierTracker::default()
                )
                .is_some()
            );
        }
    }
}
