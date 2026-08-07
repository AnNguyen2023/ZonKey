use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use zonkey_detect::{BuiltInDictionaries, Classifier, DetectionRequest};
use zonkey_policy::SafePolicy;
use zonkey_telex::compose;
use zonkey_token::apply_edit_plan;
use zonkey_types::{DecisionReason, InputContext, RecoveryDecision, TokenBoundary};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: u32,
    name: String,
    raw_keys: String,
    context: String,
    expected_text: String,
    decision: String,
    status: String,
    explanation: String,
    reason: String,
    #[serde(default)]
    telex_rendered_before_boundary: Option<String>,
    #[serde(default)]
    expected_edit_plan: Option<ExpectedEditPlan>,
    #[serde(default)]
    boundary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedEditPlan {
    delete_graphemes: usize,
    insert_text: String,
}

fn corpus_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn load_corpus() -> Vec<(PathBuf, usize, Fixture)> {
    let directory = corpus_directory();
    let mut paths: Vec<_> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| {
            entry
                .expect("corpus directory entry must be readable")
                .path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect();
    paths.sort();

    let mut fixtures = Vec::new();
    for path in paths {
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        for (index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let fixture = serde_json::from_str(line).unwrap_or_else(|error| {
                panic!(
                    "{}:{}: invalid corpus JSON: {error}",
                    path.display(),
                    index + 1
                )
            });
            fixtures.push((path.clone(), index + 1, fixture));
        }
    }
    fixtures
}

fn context(value: &str) -> InputContext {
    match value {
        "writing" => InputContext::Writing,
        "technical-editor" => InputContext::TechnicalEditor,
        "terminal" => InputContext::Terminal,
        "secure" => InputContext::Secure,
        "remote" => InputContext::Remote,
        "unknown" => InputContext::Unknown,
        other => panic!("unsupported corpus context: {other}"),
    }
}

fn boundary(value: Option<&str>) -> TokenBoundary {
    match value.unwrap_or("Space") {
        "Space" => TokenBoundary::Space,
        "Enter" => TokenBoundary::Enter,
        "Tab" => TokenBoundary::Tab,
        "Punctuation(.)" => TokenBoundary::Punctuation('.'),
        other => panic!("unsupported M2 corpus boundary: {other}"),
    }
}

fn reason(value: &str) -> DecisionReason {
    match value {
        "ExactEnglishDictionary" => DecisionReason::ExactEnglishDictionary,
        "ExactTechnicalDictionary" => DecisionReason::ExactTechnicalDictionary,
        "ExactProductDictionary" => DecisionReason::ExactProductDictionary,
        "UserAllowList" => DecisionReason::UserAllowList,
        "NeverTransformPattern" => DecisionReason::NeverTransformPattern,
        "VietnameseCandidate" => DecisionReason::VietnameseCandidate,
        "ContextBlocked" => DecisionReason::ContextBlocked,
        "InsufficientConfidence" => DecisionReason::InsufficientConfidence,
        "UnsupportedInput" => DecisionReason::UnsupportedInput,
        other => panic!("unsupported M2 corpus reason: {other}"),
    }
}

fn decision_parts(decision: &RecoveryDecision) -> (&'static str, &DecisionReason) {
    match decision {
        RecoveryDecision::KeepVietnamese { reason } => ("KeepVietnamese", reason),
        RecoveryDecision::RestoreEnglish { reason, .. } => ("RestoreEnglish", reason),
        RecoveryDecision::Ambiguous { reason, .. } => ("Ambiguous", reason),
    }
}

#[test]
#[allow(clippy::too_many_lines)] // Keeping the end-to-end corpus assertions together improves failure locality.
fn active_m2_corpus_passes_real_classifier_and_policy() {
    let classifier = Classifier::new(BuiltInDictionaries);
    let mut active = 0;
    let mut exact = 0;
    let mut vietnamese = 0;
    let mut never = 0;
    let mut context_cases = 0;
    let mut ambiguous = 0;

    for (path, line, fixture) in load_corpus() {
        assert_eq!(fixture.schema_version, 1, "{}:{line}", path.display());
        assert!(!fixture.name.trim().is_empty(), "{}:{line}", path.display());
        assert!(
            !fixture.explanation.trim().is_empty(),
            "{}:{line}",
            path.display()
        );
        if fixture.status != "active" || fixture.reason == "M1Engine" {
            continue;
        }
        active += 1;
        let rendered = fixture
            .telex_rendered_before_boundary
            .as_deref()
            .unwrap_or_else(|| panic!("{}:{line}: M2 fixture needs rendered text", path.display()));
        if fixture.reason.starts_with("Exact") {
            exact += 1;
            assert_eq!(
                compose(&fixture.raw_keys).as_deref(),
                Ok(rendered),
                "{}:{line}: rendered form must come from M1",
                path.display()
            );
            assert_ne!(
                rendered,
                fixture.raw_keys,
                "{}:{line}: recovery requires a real mutation",
                path.display()
            );
        }
        match fixture.reason.as_str() {
            "VietnameseCandidate" => vietnamese += 1,
            "NeverTransformPattern" => never += 1,
            "ContextBlocked" => context_cases += 1,
            "InsufficientConfidence" => ambiguous += 1,
            _ => {}
        }

        let token_boundary = boundary(fixture.boundary.as_deref());
        let input_context = context(&fixture.context);
        let evidence = classifier.classify(DetectionRequest {
            raw: &fixture.raw_keys,
            rendered,
            boundary: &token_boundary,
            context: input_context,
        });
        let outcome = SafePolicy.decide(evidence, input_context, rendered);
        let (actual_decision, actual_reason) = decision_parts(&outcome.decision);
        assert_eq!(
            actual_decision,
            fixture.decision,
            "{}:{line}: fixture `{}`",
            path.display(),
            fixture.name
        );
        assert_eq!(
            actual_reason,
            &reason(&fixture.reason),
            "{}:{line}: fixture `{}`",
            path.display(),
            fixture.name
        );

        let mut visible = rendered.to_owned();
        if let Some(plan) = &outcome.edit_plan {
            apply_edit_plan(&mut visible, plan);
        }
        assert_eq!(
            visible,
            fixture.expected_text,
            "{}:{line}: fixture `{}`",
            path.display(),
            fixture.name
        );
        if let Some(expected) = fixture.expected_edit_plan {
            let actual = outcome.edit_plan.expect("fixture requires an edit plan");
            assert_eq!(
                actual.delete_graphemes,
                expected.delete_graphemes,
                "{}:{line}",
                path.display()
            );
            assert_eq!(
                actual.insert_text,
                expected.insert_text,
                "{}:{line}",
                path.display()
            );
        }
    }

    assert!(active >= 180, "expected at least 180 active M2 fixtures");
    assert!(exact >= 50, "expected at least 50 exact dictionary cases");
    assert!(
        vietnamese >= 40,
        "expected at least 40 added Vietnamese negatives"
    );
    assert!(never >= 50, "expected at least 50 never-transform cases");
    assert!(
        context_cases >= 20,
        "expected at least 20 blocked context cases"
    );
    assert!(ambiguous >= 20, "expected at least 20 ambiguous cases");
}
