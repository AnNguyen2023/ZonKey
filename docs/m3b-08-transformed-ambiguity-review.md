# M3B-08 transformed ambiguity review

Status: DONE.

## Inventory

The corpus contains five active cases where `raw != rendered` and the
fixture decision is `Ambiguous`/`InsufficientConfidence`:

| Raw | Rendered | Detector evidence | Policy result | Corpus status | Likely intent | Classification |
| --- | --- | --- | --- | --- | --- | --- |
| `resume` | `réume` | exact English candidate | `Ambiguous` / insufficient confidence | active, technical-editor | preserve a common English token in a technical editor | EXPECTED_AMBIGUOUS |
| `server` | `sẻvẻ` | exact English candidate | `Ambiguous` / insufficient confidence | active, technical-editor | preserve a technical token without technical-grade evidence | EXPECTED_AMBIGUOUS |
| `browser` | `brớẻ` | exact English candidate | `Ambiguous` / insufficient confidence | active, technical-editor | preserve a technical token without technical-grade evidence | EXPECTED_AMBIGUOUS |
| `folder` | `foldẻ` | exact English candidate | `Ambiguous` / insufficient confidence | active, technical-editor | preserve a common technical token conservatively | EXPECTED_AMBIGUOUS |
| `request` | `requét` | exact English candidate | `Ambiguous` / insufficient confidence | active, technical-editor | preserve a common technical token conservatively | EXPECTED_AMBIGUOUS |

The raw inventory count is **5**. All five are explicitly technical-editor
context cases. The policy requires exact technical/product evidence in that
context, so these are context-controlled conservative outcomes, not eligible
writing-context ambiguity cases. After excluding context-blocked cases as
required by this review, the eligible transformed-ambiguous inventory is **0**.

## Selected category

No eligible category exists. The only observed group is
**technical-editor common-English mutations**, and it is intentionally
excluded rather than treated as a detector or policy gap. No category with
three eligible writing-context cases is present.

Root-cause totals for the observed five cases:

```text
EXPECTED_AMBIGUOUS = 5
DICTIONARY_GAP = 0
DETECTOR_GAP = 0
POLICY_GAP = 0
SPEC_UNCLEAR = 0
ALREADY_COVERED_ELSEWHERE = 0
```

## Representative trace

```text
raw      = resume
rendered = réume
detector = exact English candidate
policy   = Ambiguous (technical-editor requires technical-grade evidence)
```

The same policy path explains the other four cases. There is no evidence for
lowering the technical-editor threshold or broadening dictionary evidence.

## Safety controls

Writing-context recovery fixtures remain protected by exact transformed
English evidence. Vietnamese candidates, structured never-transform tokens,
and restricted-context vetoes remain separate controls and must not be
weakened by any future review. The accepted invariant also remains in force:

```text
raw == rendered -> dictionary membership alone does not create recovery evidence
```

Conversely, `raw != rendered` plus lexical evidence does not automatically
imply restoration; context and policy vetoes still apply.

## Decision

No behavior change, fixture activation, dictionary addition, detector change,
or policy tuning is justified. There is no evidence-backed narrow improvement
for a follow-up milestone from this inventory. M3B-08 is **DONE**.
