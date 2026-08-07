use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use zonkey_telex::TelexEngine;
use zonkey_token::apply_edit_plan;
use zonkey_types::{EngineAction, EngineEvent};

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

fn parse_fixture(path: &Path, line_number: usize, line: &str) -> Result<Fixture, String> {
    let fixture: Fixture = serde_json::from_str(line).map_err(|error| {
        format!(
            "{}:{line_number}: invalid corpus JSON: {error}",
            path.display()
        )
    })?;
    validate_fixture(path, line_number, &fixture)?;
    Ok(fixture)
}

fn validate_fixture(path: &Path, line_number: usize, fixture: &Fixture) -> Result<(), String> {
    let location = format!("{}:{line_number}", path.display());
    if fixture.schema_version != 1 {
        return Err(format!(
            "{location}: unsupported schema_version {}; expected 1",
            fixture.schema_version
        ));
    }
    for (field, value) in [
        ("name", fixture.name.as_str()),
        ("context", fixture.context.as_str()),
        ("decision", fixture.decision.as_str()),
        ("status", fixture.status.as_str()),
        ("explanation", fixture.explanation.as_str()),
        ("reason", fixture.reason.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("{location}: field `{field}` must not be empty"));
        }
    }
    if !matches!(fixture.status.as_str(), "active" | "planned" | "ignored") {
        return Err(format!(
            "{location}: status `{}` must be active, planned, or ignored",
            fixture.status
        ));
    }
    if fixture.status == "ignored" && !fixture.explanation.contains("reason:") {
        return Err(format!(
            "{location}: ignored fixture explanation must contain `reason:`"
        ));
    }
    if let Some(plan) = &fixture.expected_edit_plan {
        let _ = (plan.delete_graphemes, plan.insert_text.as_str());
    }
    let _ = (
        fixture.telex_rendered_before_boundary.as_deref(),
        fixture.boundary.as_deref(),
    );
    Ok(())
}

fn load_corpus() -> Result<Vec<(PathBuf, usize, Fixture)>, String> {
    let directory = corpus_directory();
    let mut paths: Vec<PathBuf> = fs::read_dir(&directory)
        .map_err(|error| {
            format!(
                "cannot read corpus directory {}: {error}",
                directory.display()
            )
        })?
        .map(|entry| {
            entry
                .map(|value| value.path())
                .map_err(|error| format!("cannot read corpus entry: {error}"))
        })
        .collect::<Result<_, _>>()?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
    });
    paths.sort();

    let mut fixtures = Vec::new();
    for path in paths {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        for (index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            fixtures.push((
                path.clone(),
                index + 1,
                parse_fixture(&path, index + 1, line)?,
            ));
        }
    }
    Ok(fixtures)
}

fn run_fixture(fixture: &Fixture) -> Result<String, String> {
    let mut engine = TelexEngine::new();
    let mut visible = String::new();
    for character in fixture.raw_keys.chars() {
        let event = if character == '\u{8}' {
            EngineEvent::Backspace
        } else {
            EngineEvent::Character(character)
        };
        let action = engine
            .process(event)
            .map_err(|error| format!("{}: unsupported input: {error:?}", fixture.name))?;
        match action {
            EngineAction::Apply(plan) => apply_edit_plan(&mut visible, &plan),
            EngineAction::Commit(_) => visible.push(character),
            EngineAction::Noop | EngineAction::Reset => {}
        }
    }
    Ok(visible)
}

#[test]
fn corpus_schema_is_valid_and_active_cases_pass() {
    let fixtures = load_corpus().unwrap_or_else(|error| panic!("{error}"));
    assert!(fixtures.len() >= 150, "expected at least 150 corpus cases");

    let mut active = 0;
    for (path, line, fixture) in fixtures {
        if fixture.status != "active" || fixture.reason != "M1Engine" {
            continue;
        }
        active += 1;
        let actual = run_fixture(&fixture)
            .unwrap_or_else(|error| panic!("{}:{line}: {error}", path.display()));
        assert_eq!(
            actual,
            fixture.expected_text,
            "{}:{line}: active fixture `{}` failed",
            path.display(),
            fixture.name
        );
    }
    assert!(active >= 100, "expected substantial active M1 coverage");
}

#[test]
fn malformed_data_has_an_actionable_location() {
    let path = Path::new("tests/corpus/broken.jsonl");
    let error = parse_fixture(path, 7, "{not-json}").expect_err("data must be rejected");
    assert!(error.contains("tests/corpus/broken.jsonl:7"));
    assert!(error.contains("invalid corpus JSON"));
}
