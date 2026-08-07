use std::collections::BTreeSet;

use zonkey_types::DecisionReason;

/// Dictionary category used as policy-neutral lexical evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DictionaryClass {
    English,
    Technical,
    Product,
    User,
}

impl DictionaryClass {
    pub(crate) const fn reason(self) -> DecisionReason {
        match self {
            Self::English => DecisionReason::ExactEnglishDictionary,
            Self::Technical => DecisionReason::ExactTechnicalDictionary,
            Self::Product => DecisionReason::ExactProductDictionary,
            Self::User => DecisionReason::UserAllowList,
        }
    }
}

/// Read-only exact-match dictionary abstraction.
pub trait DictionaryProvider {
    fn classify(&self, token: &str) -> Option<DictionaryClass>;
}

/// Version-controlled dictionaries embedded in the binary.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltInDictionaries;

impl DictionaryProvider for BuiltInDictionaries {
    fn classify(&self, token: &str) -> Option<DictionaryClass> {
        let token = token.to_ascii_lowercase();
        if contains(
            include_str!("../../../assets/dictionaries/products.txt"),
            &token,
        ) {
            Some(DictionaryClass::Product)
        } else if contains(
            include_str!("../../../assets/dictionaries/it-common.txt"),
            &token,
        ) {
            Some(DictionaryClass::Technical)
        } else if contains(
            include_str!("../../../assets/dictionaries/en-common.txt"),
            &token,
        ) {
            Some(DictionaryClass::English)
        } else {
            None
        }
    }
}

fn contains(contents: &str, token: &str) -> bool {
    contents.lines().any(|line| {
        let entry = line.trim();
        !entry.is_empty() && !entry.starts_with('#') && entry.eq_ignore_ascii_case(token)
    })
}

/// Mutable in-memory provider intended for configuration adapters and tests.
#[derive(Clone, Debug, Default)]
pub struct InMemoryDictionaries {
    english: BTreeSet<String>,
    technical: BTreeSet<String>,
    products: BTreeSet<String>,
    user: BTreeSet<String>,
}

impl InMemoryDictionaries {
    #[must_use]
    pub fn with_entries(class: DictionaryClass, entries: &[&str]) -> Self {
        let mut dictionaries = Self::default();
        let target = dictionaries.set_mut(class);
        target.extend(entries.iter().map(|entry| entry.to_ascii_lowercase()));
        dictionaries
    }

    pub fn insert(&mut self, class: DictionaryClass, entry: &str) {
        self.set_mut(class).insert(entry.to_ascii_lowercase());
    }

    fn set_mut(&mut self, class: DictionaryClass) -> &mut BTreeSet<String> {
        match class {
            DictionaryClass::English => &mut self.english,
            DictionaryClass::Technical => &mut self.technical,
            DictionaryClass::Product => &mut self.products,
            DictionaryClass::User => &mut self.user,
        }
    }
}

impl DictionaryProvider for InMemoryDictionaries {
    fn classify(&self, token: &str) -> Option<DictionaryClass> {
        let token = token.to_ascii_lowercase();
        [
            (DictionaryClass::User, &self.user),
            (DictionaryClass::Product, &self.products),
            (DictionaryClass::Technical, &self.technical),
            (DictionaryClass::English, &self.english),
        ]
        .into_iter()
        .find_map(|(class, entries)| entries.contains(&token).then_some(class))
    }
}
