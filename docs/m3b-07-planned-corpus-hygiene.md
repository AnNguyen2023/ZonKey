# M3B-07 planned corpus hygiene

Status: DONE.

M3B-06 audited 14 planned records. Owner-approved hygiene disposition removes
all 14 from `tests/corpus/initial.jsonl`; none is activated, archived, or
rewritten. The audit evidence remains in
`docs/m3b-06-planned-recovery-audit.md`.

## Disposition table

| ID | M3B-06 classification | Disposition | Reason |
| --- | --- | --- | --- |
| restore-resume-at-space | ALREADY_COVERED | REMOVE | Active exact recovery fixture already covers it |
| restore-pull-at-space | UNCHANGED_TOKEN | REMOVE | Historical `pũll` conflicts with current `pull` invariant |
| restore-refresh-at-space | ALREADY_COVERED | REMOVE | Active exact recovery coverage exists |
| keep-vietnamese-cai-dat | ALREADY_COVERED | REMOVE | Active Vietnamese behavior is covered |
| never-transform-url | ALREADY_COVERED | REMOVE | Active URL protection exists |
| never-transform-windows-path | ALREADY_COVERED | REMOVE | Active Windows-path protection exists |
| never-transform-ipv4 | ALREADY_COVERED | REMOVE | Active IP protection exists |
| never-transform-fqdn | ALREADY_COVERED | REMOVE | Active domain protection exists |
| never-transform-email | ALREADY_COVERED | REMOVE | Active e-mail protection exists |
| never-transform-powershell-parameter | ALREADY_COVERED | REMOVE | Active command-option protection exists |
| never-transform-camel-case | ALREADY_COVERED | REMOVE | Active identifier protection exists |
| never-transform-pascal-case | ALREADY_COVERED | REMOVE | Active identifier protection exists |
| never-transform-snake-case | ALREADY_COVERED | REMOVE | Active identifier protection exists |
| never-transform-kebab-case | ALREADY_COVERED | REMOVE | Active identifier protection exists |

## Counts and semantics

```text
active before = 356
active after  = 356
planned before = 69
planned after  = 55
REMOVE = 14
ARCHIVE = 0
REWRITE = 0
```

The recovery and never-transform semantics are unchanged. In particular,
`pull` is preserved only as documented historical audit evidence; its stale
planned rendered value `pũll` is not current product intent.

No detector, policy, Telex, dictionary, threshold, Windows, or diagnostic
runtime behavior changed.
