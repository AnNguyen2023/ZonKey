//! Conservative policy for platform-independent recovery decisions.

use zonkey_detect::LexicalEvidence;
use zonkey_token::edit_plan_between;
use zonkey_types::{Confidence, DecisionReason, EditPlan, InputContext, RecoveryDecision};

/// Minimum confidence for ordinary writing contexts.
pub const DEFAULT_AUTO_RECOVERY_THRESHOLD: Confidence = Confidence::ENGLISH_EXACT;
/// Technical editors require an exact technical-grade signal.
pub const TECHNICAL_EDITOR_THRESHOLD: Confidence = Confidence::TECHNICAL_EXACT;

/// A decision and an optional abstract edit. No platform action is performed.
#[derive(Clone, Debug, PartialEq)]
pub struct PolicyOutcome {
    pub decision: RecoveryDecision,
    pub edit_plan: Option<EditPlan>,
}

/// Default-deny recovery policy.
#[derive(Clone, Copy, Debug, Default)]
pub struct SafePolicy;

impl SafePolicy {
    #[must_use]
    pub fn decide(
        self,
        evidence: LexicalEvidence,
        context: InputContext,
        rendered: &str,
    ) -> PolicyOutcome {
        if matches!(
            context,
            InputContext::Terminal
                | InputContext::Secure
                | InputContext::Remote
                | InputContext::Unknown
        ) {
            return keep(DecisionReason::ContextBlocked);
        }

        match evidence {
            LexicalEvidence::Candidate {
                text,
                confidence,
                reason,
            } if permits_candidate(context, confidence, &reason) => {
                let edit_plan = edit_plan_between(rendered, &text);
                PolicyOutcome {
                    decision: RecoveryDecision::RestoreEnglish {
                        text,
                        confidence,
                        reason,
                    },
                    edit_plan: Some(edit_plan),
                }
            }
            LexicalEvidence::Candidate { confidence, .. } => PolicyOutcome {
                decision: RecoveryDecision::Ambiguous {
                    confidence,
                    reason: DecisionReason::InsufficientConfidence,
                },
                edit_plan: None,
            },
            LexicalEvidence::NeverTransform => keep(DecisionReason::NeverTransformPattern),
            LexicalEvidence::VietnameseCandidate => keep(DecisionReason::VietnameseCandidate),
            LexicalEvidence::Unsupported => keep(DecisionReason::UnsupportedInput),
            LexicalEvidence::Unknown => PolicyOutcome {
                decision: RecoveryDecision::Ambiguous {
                    confidence: Confidence::ZERO,
                    reason: DecisionReason::InsufficientConfidence,
                },
                edit_plan: None,
            },
        }
    }
}

fn permits_candidate(
    context: InputContext,
    confidence: Confidence,
    reason: &DecisionReason,
) -> bool {
    match context {
        InputContext::Writing => confidence >= DEFAULT_AUTO_RECOVERY_THRESHOLD,
        InputContext::TechnicalEditor => {
            confidence >= TECHNICAL_EDITOR_THRESHOLD
                && matches!(
                    reason,
                    DecisionReason::ExactTechnicalDictionary
                        | DecisionReason::ExactProductDictionary
                        | DecisionReason::UserAllowList
                )
        }
        InputContext::Terminal
        | InputContext::Secure
        | InputContext::Remote
        | InputContext::Unknown => false,
    }
}

fn keep(reason: DecisionReason) -> PolicyOutcome {
    PolicyOutcome {
        decision: RecoveryDecision::KeepVietnamese { reason },
        edit_plan: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_allows_exact_english_recovery_and_returns_an_edit() {
        let evidence = LexicalEvidence::Candidate {
            text: "resume".into(),
            confidence: Confidence::ENGLISH_EXACT,
            reason: DecisionReason::ExactEnglishDictionary,
        };
        let outcome = SafePolicy.decide(evidence, InputContext::Writing, "résume");
        assert!(matches!(
            outcome.decision,
            RecoveryDecision::RestoreEnglish { .. }
        ));
        assert!(outcome.edit_plan.is_some());
    }

    #[test]
    fn terminal_context_vetoes_even_certain_candidates() {
        let evidence = LexicalEvidence::Candidate {
            text: "PowerShell".into(),
            confidence: Confidence::CERTAIN,
            reason: DecisionReason::ExactProductDictionary,
        };
        let outcome = SafePolicy.decide(evidence, InputContext::Terminal, "PơwerShell");
        assert_eq!(outcome, keep(DecisionReason::ContextBlocked));
    }

    #[test]
    fn technical_editor_rejects_common_english_but_accepts_technical_terms() {
        let english = LexicalEvidence::Candidate {
            text: "resume".into(),
            confidence: Confidence::ENGLISH_EXACT,
            reason: DecisionReason::ExactEnglishDictionary,
        };
        assert!(matches!(
            SafePolicy
                .decide(english, InputContext::TechnicalEditor, "résume")
                .decision,
            RecoveryDecision::Ambiguous { .. }
        ));

        let technical = LexicalEvidence::Candidate {
            text: "hostname".into(),
            confidence: Confidence::TECHNICAL_EXACT,
            reason: DecisionReason::ExactTechnicalDictionary,
        };
        assert!(matches!(
            SafePolicy
                .decide(technical, InputContext::TechnicalEditor, "hóstname")
                .decision,
            RecoveryDecision::RestoreEnglish { .. }
        ));
    }
}
