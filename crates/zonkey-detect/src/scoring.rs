use zonkey_types::Confidence;

use crate::DictionaryClass;

pub(crate) const fn exact_confidence(class: DictionaryClass) -> Confidence {
    match class {
        DictionaryClass::English => Confidence::ENGLISH_EXACT,
        DictionaryClass::Technical | DictionaryClass::Product => Confidence::TECHNICAL_EXACT,
        DictionaryClass::User => Confidence::CERTAIN,
    }
}
