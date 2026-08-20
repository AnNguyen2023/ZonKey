# M3D-40 internal beta pilot results

Use one sanitized record per real pilot machine. Leave unknown fields as
`NOT_RUN`; never invent a PASS result. Do not record raw document text,
document paths/URIs, pipe/session IDs, token contents, secrets, or recovery
hashes.

## Pilot record

```text
machine_id=<non-sensitive alias>
windows_version=<Windows 11 x64 version>
vscode_version=<desktop version>
beta_version=0.0.1
beta_commit=c7cd538166e2f2c57eb5db5763673fd3181b0018
install=<PASS|FAIL|NOT_RUN>
startup_discovery=<PASS|FAIL|NOT_RUN>
run_beta_smoke=<PASS|FAIL|NOT_RUN>
reconnect=<PASS|FAIL|NOT_RUN>
restart_stale_identity=<PASS|FAIL|NOT_RUN>
recovery_reconciliation=<PASS|FAIL|NOT_RUN>
uninstall_cleanup=<PASS|FAIL|NOT_RUN>
document_unchanged=<true|false|not_checked>
result_enums=<bounded enums only>
transport_retries=<bounded integer>
stop_condition_hit=<true|false>
notes=<sanitized operational note>
```

## Stop-the-line

Stop immediately and report a blocker for unexpected document mutation,
Applied/mutation reachability, recovery-state loss, cross-user/session access,
stale identity acceptance, privacy leakage, or repeated transport instability.

Real Applied remains disabled and `CompositionUnknown` remains fail-closed.

## Pilot machine #1 — real evidence — 2026-08-19

```text
machine_id=pilot-01-local
windows_version=Windows 11 x64 build 10.0.26200
vscode_version=1.133.0
beta_version=0.0.1
beta_commit=c7cd538166e2f2c57eb5db5763673fd3181b0018
install=PASS
startup_discovery=PASS
run_beta_smoke=PASS
reconnect=PASS
restart_stale_identity=PASS
recovery_reconciliation=PASS
uninstall_cleanup=PASS
document_unchanged=true
result_enums=Rejected(CompositionUnknown), CurrentHandoffUnavailable, ConflictHumanReview, NotApplied, AckBeforeReconcile
transport_retries=NOT_RECORDED
stop_condition_hit=false
notes=owner physical smoke PASS; reconnect/restart/recovery used approved isolated validations; isolated VS Code profile cleanup PASS; primary dev profile was not modified
pilot_status=INTERNAL_BETA_PILOT_HEALTHY
```

The recovery validation required a test-harness-only `ELECTRON_RUN_AS_NODE`
environment guard. The runtime and recovery semantics were unchanged.

## Diagnostic hold — 2026-08-19

```text
pilot_status=PILOT_SMOKE_FAILED_PENDING_DIAGNOSIS
run_beta_smoke=FAIL
failed_marker=UNAVAILABLE_NOT_PRESERVED
failure_class=OTHER_TYPED_FAILURE
last_successful_marker=UNKNOWN
evidence=owner reported failure; the visible PowerShell runner had no sanitized transcript; endpoint was no longer running after the attempt
root_cause_hypothesis=evidence-capture gap; runtime failure stage is not established
next_action=rerun smoke only with sanitized marker capture; do not run reconnect/restart/recovery
```

## Machine #2 cross-machine pilot — 2026-08-20 (M3D-41)

```text
machine=SECOND_INTERNAL_TEST_MACHINE (sanitized)
windows=Windows 11 x64 class; PowerShell execution policy PASS; Node/npm PASS
package_checksum_gate=PASS
prior_failures=BETA_TEST_TOOLING_DEPENDENCY (bootstrap-order: missing .vscode-test VS Code archive; path correction applied by owner)
run_beta_smoke=PASS (owner physical keyboard only)
markers=M3D37_NO_CURRENT_HANDOFF_OBSERVED, M3D37_LIVE_HANDOFF_OBSERVED, M3D37_PACKAGED_COMMAND_OK, M3D37_DOCUMENT_UNCHANGED_OK, M3D37_ONE_WINDOW_SMOKE_OK, PILOT_SMOKE_OK, ZONKEY_BETA_SMOKE_OK
exit_code=0
document_unchanged=true
result_enum=Rejected(CompositionUnknown)
applied_or_mutation=NONE
conclusion=prior machine #2 failures were tooling/bootstrap/setup dependencies, not runtime defects
m3d41_verdict=MULTI_MACHINE_BETA_HEALTHY
```

Runtime, beta package, and checksums were unchanged throughout. TB-001
(self-contained clean-machine runner bootstrap + typed bootstrap failure
markers + `ZONKEY_M3D37_BOOTSTRAP_ONLY` verification mode) is implemented
in `hosts/vscode-spike/scripts/run-m3d37-smoke.mjs` after this evidence
was recorded.
