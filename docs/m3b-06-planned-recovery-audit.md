# M3B-06 planned transformed-recovery fixture audit

Status: DONE. No record was activated, edited, deleted, or rewritten during
M3B-06. M3B-07 later removed all 14 audited records from the
executable planned corpus without changing their audit evidence. The current
recovery invariant remains:

```text
raw == rendered -> dictionary membership alone yields no recovery evidence
```

The 14 records below are all from `tests/corpus/initial.jsonl`. Analysis
normalizes the complete `raw_keys` sequence into `raw_token` plus its trailing
Space boundary; fixture contents are unchanged.

| ID | raw_keys | raw_token | boundary | current rendered | planned expected | current result | classification | evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| restore-resume-at-space | `resume ` | `resume` | Space | `résume` | RestoreEnglish | RestoreEnglish | ALREADY_COVERED | active `m2-exact-resume` |
| restore-pull-at-space | `pull ` | `pull` | Space | `pull` | RestoreEnglish | Ambiguous | UNCHANGED_TOKEN | current Telex leaves token unchanged; dictionary evidence is insufficient |
| restore-refresh-at-space | `refresh ` | `refresh` | Space | `rếh` | RestoreEnglish | RestoreEnglish | ALREADY_COVERED | active exact English recovery coverage |
| keep-vietnamese-cai-dat | `caif ddawtj ` | `caif ddawtj` | Space | `cài đặt` | KeepVietnamese | KeepVietnamese | ALREADY_COVERED | active Vietnamese-candidate coverage and existing M1 semantics |
| never-transform-url | `https://example.com/resume?q=pull ` | `https://example.com/resume?q=pull` | Space | unchanged | NeverTransform | KeepVietnamese | ALREADY_COVERED | active URL/never-transform fixtures |
| never-transform-windows-path | `C:\Users\dev\refresh.txt ` | `C:\Users\dev\refresh.txt` | Space | unchanged | NeverTransform | KeepVietnamese | ALREADY_COVERED | active Windows-path fixtures |
| never-transform-ipv4 | `192.168.1.10 ` | `192.168.1.10` | Space | unchanged | NeverTransform | KeepVietnamese | ALREADY_COVERED | active IP fixtures |
| never-transform-fqdn | `api.dev.example.com ` | `api.dev.example.com` | Space | unchanged | NeverTransform | KeepVietnamese | ALREADY_COVERED | active domain fixtures |
| never-transform-email | `dev.resume@example.com ` | `dev.resume@example.com` | Space | unchanged | KeepVietnamese | KeepVietnamese | ALREADY_COVERED | active e-mail/never-transform fixtures |
| never-transform-powershell-parameter | `-RefreshToken ` | `-RefreshToken` | Space | unchanged | NeverTransform | KeepVietnamese | ALREADY_COVERED | active command-option fixtures |
| never-transform-camel-case | `refreshToken ` | `refreshToken` | Space | unchanged | NeverTransform | KeepVietnamese | ALREADY_COVERED | active identifier fixtures |
| never-transform-pascal-case | `RefreshToken ` | `RefreshToken` | Space | unchanged | NeverTransform | KeepVietnamese | ALREADY_COVERED | active identifier fixtures |
| never-transform-snake-case | `refresh_token ` | `refresh_token` | Space | unchanged | NeverTransform | KeepVietnamese | ALREADY_COVERED | active identifier fixtures |
| never-transform-kebab-case | `refresh-token ` | `refresh-token` | Space | unchanged | KeepVietnamese | KeepVietnamese | ALREADY_COVERED | active identifier fixtures |

For structured records, “unchanged” is the current Telex result; the planned
rendered value is also unchanged after removing the boundary. Their
`NeverTransformPattern` evidence protects them before dictionary recovery.

## Special review

- `resume`: `raw_token=resume`, `rendered_token=résume`, mutation is real,
  exact English evidence exists, and policy restores it. It is already covered
  by active M2 fixtures.
- `pull`: `raw_token=pull`, `rendered_token=pull`, no mutation, detector is
  `Unknown`, policy is `Ambiguous`; this planned expectation is outdated or
  stale and must not be activated.
- `refresh`: `raw_token=refresh`, `rendered_token=rếh`, mutation and exact
  English evidence exist; current policy restores it and active coverage is
  already present.

## Aggregate classification

```text
VALID_TRANSFORMED_RECOVERY = 0
UNCHANGED_TOKEN             = 1
ALREADY_COVERED             = 13
OUTDATED_EXPECTATION        = 0
SPEC_UNCLEAR                = 0

READY                       = 0
NOT_READY                   = 14
OWNER_DECISION_REQUIRED     = 0
```

No record is READY: the genuine recovery cases are duplicates of active
coverage, while `pull` conflicts with the accepted unchanged-token invariant.
The owner decision is to activate none of the 14 records. The planned set is
historical/stale backlog pending a separate corpus-hygiene decision.

## Recommended next step

**M3B-07 — planned corpus hygiene** is the next documentation/corpus-
maintenance milestone. It will decide whether stale records are removed,
archived, or rewritten only when they still represent approved product intent.
No activation is implied, and no fixture changes are made in M3B-06.
