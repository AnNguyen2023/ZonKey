use zonkey_types::{Confidence, DecisionReason, InputContext, TokenBoundary};

use crate::{
    DictionaryProvider, classify_never_transform, is_vietnamese_candidate,
    scoring::exact_confidence,
};

/// Complete, caller-observed state needed for a deterministic classification.
#[derive(Clone, Copy, Debug)]
pub struct DetectionRequest<'a> {
    pub raw: &'a str,
    pub rendered: &'a str,
    pub boundary: &'a TokenBoundary,
    pub context: InputContext,
}

/// Lexical facts produced without applying recovery policy.
#[derive(Clone, Debug, PartialEq)]
pub enum LexicalEvidence {
    Candidate {
        text: String,
        confidence: Confidence,
        reason: DecisionReason,
    },
    NeverTransform,
    VietnameseCandidate,
    Unsupported,
    Unknown,
}

/// Exact-match classifier backed by an injected dictionary provider.
#[derive(Clone, Debug)]
pub struct Classifier<D> {
    dictionaries: D,
}

impl<D> Classifier<D> {
    pub const fn new(dictionaries: D) -> Self {
        Self { dictionaries }
    }
}

impl<D: DictionaryProvider> Classifier<D> {
    #[must_use]
    pub fn classify(&self, request: DetectionRequest<'_>) -> LexicalEvidence {
        if request.raw.is_empty()
            || !request.raw.is_ascii()
            || matches!(request.boundary, TokenBoundary::Unknown)
        {
            return LexicalEvidence::Unsupported;
        }
        if classify_never_transform(request.raw).is_some() {
            return LexicalEvidence::NeverTransform;
        }
        if request.raw != request.rendered
            && let Some(class) = self.dictionaries.classify(request.raw)
        {
            return LexicalEvidence::Candidate {
                text: request.raw.to_owned(),
                confidence: exact_confidence(class),
                reason: class.reason(),
            };
        }
        if is_vietnamese_candidate(request.rendered) {
            LexicalEvidence::VietnameseCandidate
        } else {
            LexicalEvidence::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DictionaryClass, InMemoryDictionaries};

    fn request<'a>(raw: &'a str, rendered: &'a str) -> DetectionRequest<'a> {
        DetectionRequest {
            raw,
            rendered,
            boundary: &TokenBoundary::Space,
            context: InputContext::Writing,
        }
    }

    #[test]
    fn exact_match_preserves_typed_casing() {
        let dictionaries =
            InMemoryDictionaries::with_entries(DictionaryClass::English, &["resume"]);
        let evidence = Classifier::new(dictionaries).classify(request("RESUME", "RÉSUME"));
        assert!(matches!(evidence, LexicalEvidence::Candidate { text, .. } if text == "RESUME"));
    }

    #[test]
    fn structured_tokens_take_precedence_over_dictionary_entries() {
        let dictionaries =
            InMemoryDictionaries::with_entries(DictionaryClass::Technical, &["server.local"]);
        assert_eq!(
            Classifier::new(dictionaries).classify(request("server.local", "server.local")),
            LexicalEvidence::NeverTransform
        );
    }

    #[test]
    fn unchanged_dictionary_word_is_not_a_recovery_candidate() {
        let dictionaries = InMemoryDictionaries::with_entries(DictionaryClass::English, &["pull"]);
        assert_eq!(
            Classifier::new(dictionaries).classify(request("pull", "pull")),
            LexicalEvidence::Unknown
        );
    }

    #[test]
    fn punctuation_boundary_allows_a_plain_dictionary_token() {
        let dictionaries =
            InMemoryDictionaries::with_entries(DictionaryClass::English, &["resume"]);
        let boundary = TokenBoundary::Punctuation('.');
        let evidence = Classifier::new(dictionaries).classify(DetectionRequest {
            raw: "resume",
            rendered: "réume",
            boundary: &boundary,
            context: InputContext::Writing,
        });
        assert!(matches!(evidence, LexicalEvidence::Candidate { text, .. } if text == "resume"));
    }

    #[test]
    fn injected_user_allow_list_has_certain_evidence() {
        let dictionaries =
            InMemoryDictionaries::with_entries(DictionaryClass::User, &["zonkeyterm"]);
        let evidence = Classifier::new(dictionaries).classify(request("zonkeyterm", "zonkéyterm"));
        assert!(matches!(
            evidence,
            LexicalEvidence::Candidate {
                confidence: Confidence::CERTAIN,
                reason: DecisionReason::UserAllowList,
                ..
            }
        ));
    }
}
