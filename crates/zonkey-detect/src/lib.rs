//! Deterministic, platform-independent lexical recovery evidence.

mod classifier;
mod dictionary;
mod patterns;
mod scoring;
mod validation;

pub use classifier::{Classifier, DetectionRequest, LexicalEvidence};
pub use dictionary::{
    BuiltInDictionaries, DictionaryClass, DictionaryProvider, InMemoryDictionaries,
};
pub use patterns::{NeverTransformKind, classify_never_transform};
pub use validation::is_vietnamese_candidate;
