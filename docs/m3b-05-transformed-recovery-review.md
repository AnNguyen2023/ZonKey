# M3B-05 transformed English recovery coverage review

Status: IN REVIEW. This review changes no detector, dictionary, Telex, or
policy behavior.

## Selected category

**Exact dictionary-backed recovery after Telex mutation** was selected because
it has 63 active fixtures spanning English, technical, and product terms, with
clear expected `RestoreEnglish` outcomes. It directly exercises the accepted
invariant that recovery requires `raw != rendered` plus lexical evidence.

## Inventory

The corpus has 143 cases where `raw_keys` differs from the recorded Telex
rendering: 129 active and 14 planned. The selected category contains 63 active
exact dictionary cases:

| Evidence class | Active | Planned transformed recovery |
| --- | ---: | ---: |
| Exact English dictionary | 43 | 0 |
| Exact technical dictionary | 12 | 0 |
| Exact product dictionary | 8 | 0 |

Three planned records in `initial.jsonl` are labelled as recovery examples but
remain unactivated. Their values must be audited individually before use:

| Planned name | Recorded raw | Recorded rendered | Current interpretation |
| --- | --- | --- | --- |
| `restore-resume-at-space` | `resume ` | `résume` | Records a mutation, but includes the boundary in `raw_keys`; planned-only audit item |
| `restore-pull-at-space` | `pull ` | `pũll` | Current Telex evidence does not establish this mutation; planned fixture requiring audit |
| `restore-refresh-at-space` | `refresh ` | `rếh` | Records a mutation, but includes the boundary in `raw_keys`; planned-only audit item |

In particular, `pull` is not evidence of valid transformed recovery merely
because the planned record has a recovery label. The accepted invariant
`raw == rendered` -> no recovery evidence remains unchanged.

## Review matrix

| Raw | Telex rendered | Detector evidence | Policy | Expected | Status |
| --- | --- | --- | --- | --- | --- |
| `resume` | `réume` | ExactEnglishDictionary | Writing permits candidate | RestoreEnglish | PASS |
| `config` | `cònfig` | ExactTechnicalDictionary | Writing permits candidate | RestoreEnglish | PASS |
| `firefox` | `fỉèõ` | ExactProductDictionary | Writing permits candidate | RestoreEnglish | PASS |
| `dungf` | `dùng` | VietnameseCandidate | KeepVietnamese | KeepVietnamese | PASS negative control |
| `resume` in terminal/secure | `réume` | exact English evidence | ContextBlocked veto | KeepVietnamese | PASS negative control |

All active selected-category cases pass the existing policy corpus harness;
there are no active mismatches.

## Representative recovery trace

```text
raw = resume
  -> Telex rendered = réume
  -> detector = Candidate(ExactEnglishDictionary, ENGLISH_EXACT)
  -> SafePolicy(Writing) = RestoreEnglish
```

Restoration is permitted by the combination of an observed mutation, exact
dictionary evidence, writing context, and the absence of a never-transform
match. Dictionary membership alone is not sufficient for unchanged input.

## Planned-only findings

`restore-resume-at-space`, `restore-pull-at-space`, and
`restore-refresh-at-space` remain historical/planned M1 expectations. The
current M2 active corpus already covers transformed recovery through the real
classifier and policy, so activation is not required to prove current support.

## Safety observations

The Vietnamese `dungf` family remains a keep outcome despite mutation. The
same transformed English term is blocked in terminal, secure, remote, and
unknown contexts by policy. These controls limit false-positive risk for a
future recovery expansion.

## Recommendation

No activation is performed in M3B-05. The next milestone is
**M3B-06 — planned transformed-recovery fixture audit**, which will classify
all 14 planned transformed cases individually before any fixture change.
