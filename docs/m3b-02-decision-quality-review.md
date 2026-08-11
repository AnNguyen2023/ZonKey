# M3B-02 diagnostic decision-quality review

Status: DONE. No detector, dictionary, Telex, or policy behavior was
changed for this review.

## Corpus inventory

The existing JSONL corpus contains 424 fixtures:

| Status | Count | Meaning |
| --- | ---: | --- |
| active | 355 | Current executable expectations |
| planned | 69 | Explicit future expectations; not activated |

The existing Telex and policy corpus tests were reused as the deterministic
review harness. Active M1/Telex coverage is 151 cases. Active M2 decision
coverage is 204 cases:

| Category/reason | Count | Decision |
| --- | ---: | --- |
| Exact English dictionary | 43 | RestoreEnglish |
| Exact technical dictionary | 12 | RestoreEnglish |
| Exact product dictionary | 8 | RestoreEnglish |
| Vietnamese candidate | 40 | KeepVietnamese |
| Never-transform pattern | 56 | KeepVietnamese |
| Context blocked | 20 | KeepVietnamese |
| Insufficient confidence | 25 | Ambiguous |

The 69 planned fixtures contain 35 future never-transform cases, 28 future
English recovery cases, and 6 future Vietnamese keep cases. They remain
planning evidence rather than active acceptance criteria.

## Decision-quality matrix

| Input/category | Fixture/source | Telex render | Detection | Policy | Status |
| --- | --- | --- | --- | --- | --- |
| `resume` | active `09-m2-exact-dictionary`, `12-m2-context-policy` | mutated (`réume`) | exact English dictionary | RestoreEnglish in Writing; blocked/ambiguous in restricted contexts | PASS |
| `dungf` | active M1/Telex and M2 Vietnamese cases | `dùng` | VietnameseCandidate | KeepVietnamese | PASS |
| `hello` | active `13-m2-ambiguous` | unchanged | Unknown | Ambiguous | EXPECTED_AMBIGUOUS |
| `pull` | owner-approved recovery-invariant review; existing unchanged-dictionary test | unchanged under current Telex input | Unknown (dictionary membership is insufficient) | Ambiguous | PASS / conservative invariant |
| `abc123` | active `13-m2-ambiguous` | unchanged | Unknown | Ambiguous | PASS / conservative mixed alphanumeric fixture |
| URLs, paths, domains, options, identifiers, secrets | active `11-m2-never-transform` | unchanged | NeverTransformPattern | KeepVietnamese | PASS |
| Technical/product terms | active `09-m2-exact-dictionary` | mutated | exact technical/product dictionary | RestoreEnglish | PASS |
| Terminal/secure/remote contexts | active `12-m2-context-policy` | mutated | exact evidence | ContextBlocked policy veto | PASS |

## Live examples and layer attribution

- `resume -> RestoreCandidate`: Telex creates a changed rendered form;
  dictionary detection supplies exact English evidence; Writing `SafePolicy`
  permits the candidate.
- `dungf -> Keep`: Telex creates `dùng`; detector classifies it as a
  Vietnamese candidate; policy keeps it.
- `hello -> Ambiguous`: Telex leaves the token unchanged; detection produces
  no lexical candidate; policy returns `InsufficientConfidence`. This matches
  the active `m2-ambiguous-hello` fixture.
- `pull -> Ambiguous`: current live result follows the explicit recovery
  invariant: when `raw == rendered`, dictionary membership alone does not
  produce recovery evidence. The unchanged-dictionary regression test remains
  authoritative.
- `abc123 -> Ambiguous`: unchanged/unknown under current rules and now covered
  by an active conservative mixed-alphanumeric fixture.

## Safety review

Changing `Ambiguous` to `RestoreEnglish` for unknown tokens would increase the
risk of corrupting valid Vietnamese, identifiers, or secrets. Existing
never-transform and context-blocked cases demonstrate the conservative policy
boundary. No false-positive mitigation should be weakened in this milestone.

No false-negative candidate is proven by this review. `pull` is intentionally
conservative when Telex produced no mutation, and `abc123` is explicitly
covered as ambiguous.

## Recommendations by layer

- Telex: no change supported by this evidence.
- Dictionary/corpus: retain planned historical recovery fixtures as inactive;
  keep the active `abc123` conservative fixture and the unchanged-dictionary
  regression for `pull`.
- Detector: no proven gap in active fixtures.
- Policy: no threshold change justified.
- Tests: retain the existing corpus harness and add only approved expectations
  in a follow-up.

## Next smallest milestone

The owner-approved M3B-03 fixture work preserves the unchanged-token recovery
invariant and adds active conservative coverage for `abc123`. No runtime
behavior or thresholds change.
