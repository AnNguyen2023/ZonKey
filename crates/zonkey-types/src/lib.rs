//! Shared, platform-independent domain types.

mod observe;

pub use observe::{
    ContextSnapshotError, EventSequence, EventSequenceError, ForegroundContextSnapshot,
    InjectionOrigin, IntegrityRelation, KeyEventKind, ModifierKey, ModifierState,
    ObservedInputEvent, ObservedKey, ObservedKeyError, ObserverError, ObserverStatus,
};

/// An explicit boundary that ends or invalidates the current token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenBoundary {
    Space,
    Enter,
    Tab,
    Punctuation(char),
    CursorMove,
    FocusLoss,
    Unknown,
}

impl TokenBoundary {
    /// Classifies characters that have defined M1 boundary semantics.
    #[must_use]
    pub fn from_char(character: char) -> Option<Self> {
        match character {
            ' ' => Some(Self::Space),
            '\n' | '\r' => Some(Self::Enter),
            '\t' => Some(Self::Tab),
            value if value.is_ascii_punctuation() => Some(Self::Punctuation(value)),
            _ => None,
        }
    }
}

/// Caller-supplied context for a platform-independent recovery decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputContext {
    Writing,
    TechnicalEditor,
    Terminal,
    Secure,
    Remote,
    Unknown,
}

/// Why the recovery classifier reached a decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecisionReason {
    ExactEnglishDictionary,
    ExactTechnicalDictionary,
    ExactProductDictionary,
    UserAllowList,
    NeverTransformPattern,
    VietnameseCandidate,
    ContextBlocked,
    InsufficientConfidence,
    UnsupportedInput,
}

/// A finite confidence constrained to the inclusive range `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Confidence(f32);

impl Confidence {
    pub const ZERO: Self = Self(0.0);
    pub const ENGLISH_EXACT: Self = Self(0.98);
    pub const TECHNICAL_EXACT: Self = Self(0.99);
    pub const CERTAIN: Self = Self(1.0);

    /// Validates and constructs a confidence.
    ///
    /// # Errors
    ///
    /// Returns [`ConfidenceError`] for NaN, infinity, or values outside
    /// `0.0..=1.0`.
    pub fn new(value: f32) -> Result<Self, ConfidenceError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ConfidenceError)
        }
    }

    /// Returns the validated scalar value.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// Error returned when confidence is non-finite or outside its valid range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfidenceError;

/// Conservative recovery outcome. This type never executes an edit.
#[derive(Clone, Debug, PartialEq)]
pub enum RecoveryDecision {
    KeepVietnamese {
        reason: DecisionReason,
    },
    RestoreEnglish {
        text: String,
        confidence: Confidence,
        reason: DecisionReason,
    },
    Ambiguous {
        confidence: Confidence,
        reason: DecisionReason,
    },
}

/// An abstract text edit produced by core logic and executed by a platform adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditPlan {
    /// Number of user-perceived grapheme clusters to delete before the caret.
    pub delete_graphemes: usize,
    /// Unicode text to insert after deletion.
    pub insert_text: String,
}

/// A platform-neutral result from one engine transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineAction {
    Noop,
    Apply(EditPlan),
    Commit(TokenBoundary),
    Reset,
}

/// Input accepted by the M1 engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineEvent {
    Character(char),
    Backspace,
    Boundary(TokenBoundary),
}

/// A behavior that the intentionally narrow M1 core cannot process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsupportedBehavior {
    NonAsciiInput(char),
}

#[cfg(test)]
mod tests {
    use super::{Confidence, EditPlan, TokenBoundary};

    #[test]
    fn empty_edit_plan_is_a_no_op() {
        assert_eq!(EditPlan::default().delete_graphemes, 0);
        assert!(EditPlan::default().insert_text.is_empty());
    }

    #[test]
    fn classifies_character_boundaries() {
        assert_eq!(TokenBoundary::from_char(' '), Some(TokenBoundary::Space));
        assert_eq!(TokenBoundary::from_char('\n'), Some(TokenBoundary::Enter));
        assert_eq!(TokenBoundary::from_char('\t'), Some(TokenBoundary::Tab));
        assert_eq!(
            TokenBoundary::from_char('.'),
            Some(TokenBoundary::Punctuation('.'))
        );
        assert_eq!(TokenBoundary::from_char('a'), None);
    }

    #[test]
    fn confidence_rejects_non_finite_and_out_of_range_values() {
        assert!(Confidence::new(f32::NAN).is_err());
        assert!(Confidence::new(f32::INFINITY).is_err());
        assert!(Confidence::new(-0.01).is_err());
        assert!(Confidence::new(1.01).is_err());
        assert_eq!(Confidence::new(0.98).map(Confidence::get), Ok(0.98));
    }
}
