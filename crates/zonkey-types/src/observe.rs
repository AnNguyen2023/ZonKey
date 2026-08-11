//! Platform-neutral observed-input value types for the future M3A adapter.
//!
//! These types contain metadata only. They cannot observe input, queue events,
//! log, inject, suppress, replay, replace, or edit user text.

use super::InputContext;

const MAX_CONTEXT_LABEL_BYTES: usize = 64;

/// The kind of a key transition observed by a future adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    /// A normal key-down transition.
    KeyDown,
    /// A normal key-up transition.
    KeyUp,
    /// A system/modifier key-down transition.
    SystemKeyDown,
    /// A system/modifier key-up transition.
    SystemKeyUp,
}

/// A platform-neutral modifier identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierKey {
    /// Shift modifier.
    Shift,
    /// Control modifier.
    Control,
    /// Alt modifier.
    Alt,
    /// Meta/Windows/Command-style modifier.
    Meta,
}

/// Validation failure for an observed key constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedKeyError {
    /// The supplied character was not an ASCII alphabetic character.
    InvalidLetter,
    /// The supplied value was not a decimal digit in `0..=9`.
    InvalidDigit,
    /// The supplied character was not ASCII punctuation.
    InvalidPunctuation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObservedKeyKind {
    Letter(char),
    Digit(u8),
    Space,
    Enter,
    Tab,
    Backspace,
    Escape,
    Punctuation(char),
    Modifier(ModifierKey),
    Other,
}

/// A validated, platform-neutral key identity.
///
/// `Letter` and `Punctuation` values can only be created through validating
/// constructors. Native virtual-key numbers and arbitrary Unicode are not
/// represented. This type contains no key history or text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedKey(ObservedKeyKind);

impl ObservedKey {
    /// Constructs an ASCII alphabetic key.
    ///
    /// # Errors
    ///
    /// Returns [`ObservedKeyError::InvalidLetter`] for non-ASCII or
    /// non-alphabetic input.
    pub fn letter(value: char) -> Result<Self, ObservedKeyError> {
        value
            .is_ascii_alphabetic()
            .then_some(Self(ObservedKeyKind::Letter(value)))
            .ok_or(ObservedKeyError::InvalidLetter)
    }

    /// Constructs a decimal digit key from `0..=9`.
    ///
    /// # Errors
    ///
    /// Returns [`ObservedKeyError::InvalidDigit`] when `value` is greater
    /// than nine.
    pub fn digit(value: u8) -> Result<Self, ObservedKeyError> {
        (value <= 9)
            .then_some(Self(ObservedKeyKind::Digit(value)))
            .ok_or(ObservedKeyError::InvalidDigit)
    }

    /// Constructs the space key.
    #[must_use]
    pub const fn space() -> Self {
        Self(ObservedKeyKind::Space)
    }

    /// Constructs the enter key.
    #[must_use]
    pub const fn enter() -> Self {
        Self(ObservedKeyKind::Enter)
    }

    /// Constructs the tab key.
    #[must_use]
    pub const fn tab() -> Self {
        Self(ObservedKeyKind::Tab)
    }

    /// Constructs the backspace key.
    #[must_use]
    pub const fn backspace() -> Self {
        Self(ObservedKeyKind::Backspace)
    }

    /// Constructs the escape key.
    #[must_use]
    pub const fn escape() -> Self {
        Self(ObservedKeyKind::Escape)
    }

    /// Constructs an ASCII punctuation key after validation.
    ///
    /// # Errors
    ///
    /// Returns [`ObservedKeyError::InvalidPunctuation`] for non-ASCII or
    /// non-punctuation input.
    pub fn punctuation(value: char) -> Result<Self, ObservedKeyError> {
        value
            .is_ascii_punctuation()
            .then_some(Self(ObservedKeyKind::Punctuation(value)))
            .ok_or(ObservedKeyError::InvalidPunctuation)
    }

    /// Constructs a modifier key.
    #[must_use]
    pub const fn modifier(value: ModifierKey) -> Self {
        Self(ObservedKeyKind::Modifier(value))
    }

    /// Constructs an intentionally unclassified key.
    #[must_use]
    pub const fn other() -> Self {
        Self(ObservedKeyKind::Other)
    }

    /// Returns the letter when this is a letter key.
    #[must_use]
    pub const fn letter_value(self) -> Option<char> {
        match self.0 {
            ObservedKeyKind::Letter(value) => Some(value),
            _ => None,
        }
    }

    /// Returns the digit when this is a digit key.
    #[must_use]
    pub const fn digit_value(self) -> Option<u8> {
        match self.0 {
            ObservedKeyKind::Digit(value) => Some(value),
            _ => None,
        }
    }

    /// Returns the punctuation character when this is punctuation.
    #[must_use]
    pub const fn punctuation_value(self) -> Option<char> {
        match self.0 {
            ObservedKeyKind::Punctuation(value) => Some(value),
            _ => None,
        }
    }

    /// Returns the modifier when this is a modifier key.
    #[must_use]
    pub const fn modifier_value(self) -> Option<ModifierKey> {
        match self.0 {
            ObservedKeyKind::Modifier(value) => Some(value),
            _ => None,
        }
    }

    /// Returns whether this is the space key.
    #[must_use]
    pub const fn is_space(self) -> bool {
        matches!(self.0, ObservedKeyKind::Space)
    }

    /// Returns whether this is the Enter key.
    #[must_use]
    pub const fn is_enter(self) -> bool {
        matches!(self.0, ObservedKeyKind::Enter)
    }

    /// Returns whether this is the Tab key.
    #[must_use]
    pub const fn is_tab(self) -> bool {
        matches!(self.0, ObservedKeyKind::Tab)
    }

    /// Returns whether this is the Backspace key.
    #[must_use]
    pub const fn is_backspace(self) -> bool {
        matches!(self.0, ObservedKeyKind::Backspace)
    }

    /// Returns whether this is the Escape key.
    #[must_use]
    pub const fn is_escape(self) -> bool {
        matches!(self.0, ObservedKeyKind::Escape)
    }
}

/// Immutable state for the four supported modifier keys.
///
/// The fields are private so callers cannot construct an invalid bitmask or
/// attach native modifier values. The default has every modifier cleared.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifierState {
    shift: bool,
    control: bool,
    alt: bool,
    meta: bool,
}

impl ModifierState {
    /// Constructs a state with no active modifiers.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            shift: false,
            control: false,
            alt: false,
            meta: false,
        }
    }

    /// Returns a copy with Shift set to `active`.
    #[must_use]
    pub const fn with_shift(mut self, active: bool) -> Self {
        self.shift = active;
        self
    }

    /// Returns a copy with Control set to `active`.
    #[must_use]
    pub const fn with_control(mut self, active: bool) -> Self {
        self.control = active;
        self
    }

    /// Returns a copy with Alt set to `active`.
    #[must_use]
    pub const fn with_alt(mut self, active: bool) -> Self {
        self.alt = active;
        self
    }

    /// Returns a copy with Meta set to `active`.
    #[must_use]
    pub const fn with_meta(mut self, active: bool) -> Self {
        self.meta = active;
        self
    }

    /// Returns whether Shift is active.
    #[must_use]
    pub const fn shift(self) -> bool {
        self.shift
    }

    /// Returns whether Control is active.
    #[must_use]
    pub const fn control(self) -> bool {
        self.control
    }

    /// Returns whether Alt is active.
    #[must_use]
    pub const fn alt(self) -> bool {
        self.alt
    }

    /// Returns whether Meta is active.
    #[must_use]
    pub const fn meta(self) -> bool {
        self.meta
    }
}

/// Observation metadata about whether an event appears injected.
///
/// This is not a trust or security decision. A future platform adapter may
/// provide only best-effort metadata here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectionOrigin {
    /// The source is unknown.
    Unknown,
    /// The event appears physical or has no injection marker.
    PhysicalOrUnmarked,
    /// The event carries an injection marker.
    MarkedInjected,
    /// The event appears injected from a lower-integrity source.
    LowerIntegrityInjected,
}

/// A non-zero event sequence identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventSequence(u64);

impl EventSequence {
    /// Constructs a sequence identifier, rejecting zero.
    ///
    /// # Errors
    ///
    /// Returns [`EventSequenceError`] when `value` is zero.
    pub const fn new(value: u64) -> Result<Self, EventSequenceError> {
        if value == 0 {
            Err(EventSequenceError)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the validated non-zero value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Error returned when an event sequence is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventSequenceError;

/// A platform-neutral observed key event.
///
/// This value deliberately stores no token text, key history, timestamp,
/// title, path, PID, handle, native event object, or editing capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedInputEvent {
    /// Validated platform-neutral key identity.
    pub key: ObservedKey,
    /// Key transition kind.
    pub kind: KeyEventKind,
    /// Immutable modifier state.
    pub modifiers: ModifierState,
    /// Best-effort observation metadata about injection origin.
    pub injection_origin: InjectionOrigin,
    /// Non-zero event sequence.
    pub sequence: EventSequence,
}

/// Best-effort integrity relationship metadata for a future adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegrityRelation {
    /// Same integrity or relationship unavailable.
    SameOrUnknown,
    /// Observed target is lower integrity.
    Lower,
    /// Observed target is higher integrity.
    Higher,
    /// Integrity metadata cannot be obtained.
    Unavailable,
}

/// Sanitized foreground metadata for future policy context.
///
/// It contains no title, full path, command line, PID, HWND, native object, or
/// user text. Secure contexts are ineligible for token diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForegroundContextSnapshot {
    input_context: InputContext,
    executable_basename: Option<String>,
    window_class: Option<String>,
    integrity_relation: IntegrityRelation,
    secure_desktop: bool,
}

impl ForegroundContextSnapshot {
    /// Constructs a safe unknown snapshot with no identifying metadata.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            input_context: InputContext::Unknown,
            executable_basename: None,
            window_class: None,
            integrity_relation: IntegrityRelation::SameOrUnknown,
            secure_desktop: false,
        }
    }

    /// Constructs a snapshot from bounded sanitized labels.
    ///
    /// Labels accept only ASCII letters, digits, `.`, `_`, and `-`, and are
    /// limited to 64 bytes. Full paths, titles, command lines, and user text
    /// are rejected by this shape.
    ///
    /// # Errors
    ///
    /// Returns [`ContextSnapshotError::InvalidLabel`] when either optional
    /// label is empty, too long, or contains a disallowed character.
    pub fn new(
        input_context: InputContext,
        executable_basename: Option<&str>,
        window_class: Option<&str>,
        integrity_relation: IntegrityRelation,
        secure_desktop: bool,
    ) -> Result<Self, ContextSnapshotError> {
        Ok(Self {
            input_context,
            executable_basename: sanitize_label(executable_basename)?,
            window_class: sanitize_label(window_class)?,
            integrity_relation,
            secure_desktop,
        })
    }

    /// Returns the selected input context.
    #[must_use]
    pub const fn input_context(&self) -> InputContext {
        self.input_context
    }

    /// Returns the sanitized executable basename, if present.
    #[must_use]
    pub fn executable_basename(&self) -> Option<&str> {
        self.executable_basename.as_deref()
    }

    /// Returns the bounded sanitized window-class label, if present.
    #[must_use]
    pub fn window_class(&self) -> Option<&str> {
        self.window_class.as_deref()
    }

    /// Returns the future best-effort integrity relationship.
    #[must_use]
    pub const fn integrity_relation(&self) -> IntegrityRelation {
        self.integrity_relation
    }

    /// Returns whether this snapshot represents a secure desktop.
    #[must_use]
    pub const fn secure_desktop(&self) -> bool {
        self.secure_desktop
    }

    /// Returns whether token diagnostics are eligible for this snapshot.
    #[must_use]
    pub const fn diagnostics_eligible(&self) -> bool {
        !self.secure_desktop && !matches!(self.input_context, InputContext::Secure)
    }
}

fn sanitize_label(value: Option<&str>) -> Result<Option<String>, ContextSnapshotError> {
    let Some(value) = value else { return Ok(None) };
    if value.is_empty()
        || value.len() > MAX_CONTEXT_LABEL_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ContextSnapshotError::InvalidLabel);
    }
    Ok(Some(value.to_owned()))
}

/// Validation failure for foreground context labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextSnapshotError {
    /// A label was empty, too long, or contained disallowed characters.
    InvalidLabel,
}

/// Lifecycle state of a future observer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObserverStatus {
    /// Observer startup is in progress.
    Starting,
    /// Observer is active.
    Running,
    /// Observer is draining and stopping.
    Stopping,
    /// Observer has stopped.
    Stopped,
    /// Observer failed and cannot continue without review.
    Failed,
}

/// Redacted categories of observer failure.
///
/// Native OS error strings are intentionally not stored in this public type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObserverError {
    /// The requested observation capability is unavailable.
    NotSupported,
    /// Observer startup failed.
    StartupFailed,
    /// The event source closed.
    EventSourceClosed,
    /// A bounded queue lost capacity.
    QueueOverflow,
    /// An observed event failed neutral validation.
    InvalidEvent,
    /// Observer shutdown failed.
    ShutdownFailed,
    /// An intentionally redacted internal failure.
    Internal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_event_kinds_are_exhaustive_and_copyable() {
        let kinds = [
            KeyEventKind::KeyDown,
            KeyEventKind::KeyUp,
            KeyEventKind::SystemKeyDown,
            KeyEventKind::SystemKeyUp,
        ];
        assert_eq!(kinds.len(), 4);
        assert_eq!(kinds[0], KeyEventKind::KeyDown);
    }

    #[test]
    fn observed_key_validates_letter_punctuation_and_digits() {
        assert_eq!(ObservedKey::letter('A').unwrap().letter_value(), Some('A'));
        assert_eq!(ObservedKey::letter('z').unwrap().letter_value(), Some('z'));
        assert_eq!(
            ObservedKey::letter('é'),
            Err(ObservedKeyError::InvalidLetter)
        );
        assert_eq!(
            ObservedKey::letter('1'),
            Err(ObservedKeyError::InvalidLetter)
        );
        assert_eq!(
            ObservedKey::punctuation('?').unwrap().punctuation_value(),
            Some('?')
        );
        assert_eq!(
            ObservedKey::punctuation('é'),
            Err(ObservedKeyError::InvalidPunctuation)
        );
        assert_eq!(
            ObservedKey::punctuation('a'),
            Err(ObservedKeyError::InvalidPunctuation)
        );
        assert_eq!(ObservedKey::digit(0).unwrap().digit_value(), Some(0));
        assert_eq!(ObservedKey::digit(9).unwrap().digit_value(), Some(9));
        assert_eq!(ObservedKey::digit(10), Err(ObservedKeyError::InvalidDigit));
    }

    #[test]
    fn observed_key_covers_non_character_categories() {
        assert_eq!(ObservedKey::space(), ObservedKey::space());
        assert_eq!(ObservedKey::enter(), ObservedKey::enter());
        assert_eq!(ObservedKey::tab(), ObservedKey::tab());
        assert_eq!(ObservedKey::backspace(), ObservedKey::backspace());
        assert_eq!(ObservedKey::escape(), ObservedKey::escape());
        assert!(ObservedKey::space().is_space());
        assert!(ObservedKey::enter().is_enter());
        assert!(ObservedKey::tab().is_tab());
        assert!(ObservedKey::backspace().is_backspace());
        assert!(ObservedKey::escape().is_escape());
        assert_eq!(
            ObservedKey::modifier(ModifierKey::Shift).modifier_value(),
            Some(ModifierKey::Shift)
        );
        assert_eq!(ObservedKey::other(), ObservedKey::other());
    }

    #[test]
    fn modifier_state_defaults_to_clear_and_supports_combinations() {
        assert_eq!(ModifierState::default(), ModifierState::new());
        let state = ModifierState::new()
            .with_shift(true)
            .with_control(true)
            .with_alt(true)
            .with_meta(true);
        assert!(state.shift() && state.control() && state.alt() && state.meta());
    }

    #[test]
    fn sequence_rejects_zero() {
        assert_eq!(EventSequence::new(0), Err(EventSequenceError));
        assert_eq!(EventSequence::new(1).unwrap().get(), 1);
    }

    #[test]
    fn event_shape_contains_only_neutral_metadata() {
        let event = ObservedInputEvent {
            key: ObservedKey::letter('a').unwrap(),
            kind: KeyEventKind::KeyDown,
            modifiers: ModifierState::new(),
            injection_origin: InjectionOrigin::PhysicalOrUnmarked,
            sequence: EventSequence::new(1).unwrap(),
        };
        assert_eq!(event.key.letter_value(), Some('a'));
    }

    #[test]
    fn unknown_context_is_safe_and_secure_context_is_not_diagnostic_eligible() {
        let unknown = ForegroundContextSnapshot::unknown();
        assert_eq!(unknown.input_context(), InputContext::Unknown);
        assert!(unknown.diagnostics_eligible());
        let secure = ForegroundContextSnapshot::new(
            InputContext::Secure,
            None,
            None,
            IntegrityRelation::Unavailable,
            false,
        )
        .unwrap();
        assert!(!secure.diagnostics_eligible());
        let desktop = ForegroundContextSnapshot::new(
            InputContext::Writing,
            Some("editor.exe"),
            Some("TextWindow"),
            IntegrityRelation::SameOrUnknown,
            true,
        )
        .unwrap();
        assert!(!desktop.diagnostics_eligible());
    }

    #[test]
    fn context_labels_are_bounded_and_sanitized() {
        assert!(
            ForegroundContextSnapshot::new(
                InputContext::Writing,
                Some("C:\\editor.exe"),
                None,
                IntegrityRelation::SameOrUnknown,
                false,
            )
            .is_err()
        );
        assert!(
            ForegroundContextSnapshot::new(
                InputContext::Writing,
                Some("editor.exe"),
                Some("class name"),
                IntegrityRelation::SameOrUnknown,
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn observer_errors_carry_no_native_payload() {
        let errors = [
            ObserverError::NotSupported,
            ObserverError::StartupFailed,
            ObserverError::EventSourceClosed,
            ObserverError::QueueOverflow,
            ObserverError::InvalidEvent,
            ObserverError::ShutdownFailed,
            ObserverError::Internal,
        ];
        assert_eq!(errors.len(), 7);
        assert_eq!(ObserverError::Internal, ObserverError::Internal);
    }
}
