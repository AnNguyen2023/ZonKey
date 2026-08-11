# Zonkey architecture and implementation status


## M2 detection and policy boundary

M2 keeps lexical facts and recovery policy separate. Raw text, rendered text, boundary, and caller context enter zonkey-detect. LexicalEvidence then enters zonkey-policy, which returns a RecoveryDecision and optional EditPlan.

zonkey-detect does not authorize edits. zonkey-policy does not load dictionaries or recognize token syntax. zonkey-token remains the only crate that computes grapheme-safe edits. Context is supplied explicitly by a future adapter; M2 does not inspect applications, windows, processes, secure desktops, or remote sessions.

The dependency direction is acyclic: policy depends on detect, token, and types; detect depends only on types. Dictionary assets are embedded at compile time, so classification performs no filesystem or network access.

## M3A observe-only boundary

M3A remains a strict observe-only boundary. M3A-01 contracts and the M3A-02
mock pipeline are implemented, but the current codebase has no runtime text
editing capability and no crate acquires real keyboard input or executes
`EditPlan`s. The scope is governed by `docs/m3a-observe-only-scope.md` and ADR
0003. M3A-03 adapter-boundary decisions are recorded in
`docs/adr/0004-m3a03-observe-adapter-boundary.md`; this ADR does not add a
Windows runtime.

The proposed future data flow is deliberately non-implemented:

```text
future hook callback
  -> bounded loss-aware queue
  -> future service loop
  -> core detection/policy
  -> aggregate/redacted diagnostics
```

The callback must forward input immediately; observation must never suppress,
replay, inject, replace, or block user input. M3A adds no Windows dependency,
FFI, hook, GUI, service, persistence, or diagnostics writer.

### M3A-01 platform-neutral type boundary — DONE

M3A-01 adds only validated observed-input value types to `zonkey-types`.
They cannot observe input, start a thread, log text, or modify text. The types
carry no native handles, virtual-key numbers, timestamps, token text, process
identity, or window handles. `zonkey-win` is the only crate permitted to
depend on Windows APIs; its spike boundary hands events toward
`zonkey-service` and `zonkey-types`.

The contracts introduce no runtime behavior. Queueing, service lifecycle,
overflow, and shutdown semantics are implemented separately by M3A-02.

### M3A-02 bounded event pipeline — DONE

M3A-02 implements only an in-memory mock pipeline in `zonkey-service`: a
bounded FIFO queue and a synchronous service abstraction over mock event
sources/processors. It is deterministic, platform-neutral, and non-runtime.
The queue reports loss without reconstructing events; the service resets its
processor boundary before the first event after a loss episode and drains
accepted events on graceful stop. No component can observe real input or
change user text, and no `EditPlan` is executed.

The current synchronous service consumes a finite mock source into the bounded
queue before draining it, so a source larger than capacity can exercise queue
overflow in tests. This is still not a real producer/consumer runtime; no
Windows observer feeds it.

The queue, service lifecycle, and processor contracts are documented in
`docs/m3a-02-bounded-event-pipeline.md`. Windows mapping, a real hook,
foreground context, diagnostics writing, and text editing remain deferred.

### Current mock-only flow

```text
mock EventSource
    ↓
bounded ObserveQueue
    ↓
mock EventProcessor
    ↓
aggregate report
```

This flow is synchronous, deterministic, platform-neutral, and non-runtime.
It cannot observe real input or change user input. A future Windows runtime
must be documented separately and must not be described as implemented here.

### M3A-03 adapter/lifecycle contract — DONE

The platform-neutral adapter boundary is verified through public-API tests in
`zonkey-service`: validated events cross a mock adapter into the bounded
service, FIFO ownership and immutable processor input are preserved, overflow
and discontinuity are observable, and stop, exhaustion, and source-failure
terminal behavior are deterministic. No Windows observer is implemented.

M3A-04 is the completed Windows observe-source spike. ADR 0005 selected
`WH_KEYBOARD_LL`; the repository contains the Windows-only runnable observer,
bounded wake-message bridge, native-to-domain mapping tests, and owner manual
acceptance. Suppression, injection, replay, text editing, and foreground
inspection remain out of scope.

### M3A-04 implementation status — DONE

`zonkey-win` contains the Windows-only hook boundary, bounded non-blocking
bridge, modifier/injection mapping, and clean Ctrl+C shutdown path. The CLI
entry point is `cargo run -p zonkey-cli -- observe`. The callback only copies
minimal metadata, attempts the bounded handoff, chains `CallNextHookEx`, and
returns; the service processor prints sanitized aggregate event metadata after
drain. Manual acceptance covered modifier mapping and transparent focused-app
input: 49 callbacks/mappings/processed events with no drops. Raw Input remains
eligible for later production evaluation.

This is a validated spike, not a final production architecture. Diagnostics
are spike-only; mapping is intentionally incomplete and unsupported keys use
`ObservedKey::other`; there is no Unicode reconstruction, foreground/context
inspection, production IME behavior, injection, or editing. `WH_KEYBOARD_LL`
is not permanently selected for the final runtime.

The native boundary is isolated to `zonkey-win`; core crates remain
platform-neutral. Automated checks do not replace the owner manual test; that
M3A-04 acceptance has now passed.

### M3A-05 hardening review — DONE

ADR 0006 records decision class C: retain the validated `WH_KEYBOARD_LL`
spike temporarily and require a separate Raw Input comparison. Owner accepted
the comparison, which is now complete; the hook is selected for the next
production phase without permanently rejecting Raw Input.

The proposed production boundary permits minimal metadata capture, modifier
and injection-origin mapping, sequence assignment, bounded non-blocking
handoff, and deterministic shutdown. It forbids text reconstruction,
foreground/context inspection, persistence, telemetry, suppression, replay,
injection, editing, callback blocking, and raw key history. M3A-02 loss
semantics and separate bridge/service counters remain the baseline.

### M3A-06 Raw Input comparison spike — DONE

`zonkey-win` now contains a parallel Raw Input path. It registers keyboard
usage page `0x01`/usage `0x06` with `RIDEV_INPUTSINK` on a hidden message-only
window and deliberately omits `RIDEV_NOLEGACY`. The path copies bounded
`RAWKEYBOARD` metadata, maps through the same `ObservedInputEvent` contract,
and uses a separate capacity-256 newest-drop bridge. `observe-hook` preserves
the hook spike and `observe-raw` runs Raw Input.

Raw Input does not provide a direct `LLKHF_INJECTED` equivalent in this
boundary, so injection visibility is not claimed as equivalent. Owner
controlled comparison is complete; no decision-pipeline integration exists
yet.

Final owner evidence: Raw Input processed 36/36 events with zero mapping or
bridge drops; Hook received, accepted, and processed 96/96 events with zero
mapping, bridge, lock, or service drops. Both paths passed modifier, repeat,
shutdown, and transparency scenarios. The earlier 53-event loss was at the
platform-neutral `ObserveQueue` and was resolved by M3A-07.

M3A-06 exposed that the prior service loop filled its queue while polling a
live source and deferred processing until terminal shutdown. M3A-07 changes
the service loop to process one queued event after each source event, then
resume polling. Exhaustion and explicit stop still drain the remainder FIFO;
source failure still leaves pending events undrained. M3A-03's finite/mock
contract remains valid for its original scope.

### M3A-07 continuous service consumption — DONE

The deterministic single-thread interleaving rule prevents a live source from
filling capacity 256 solely because processing was deferred. The bounded queue
remains meaningful for bursts and preserves newest-drop and discontinuity
semantics. The 600-event sustained-source test processes all 600 events with
zero drops. No threads, async runtime, Windows mechanism selection, or
Telex/detection/policy integration is introduced.

### Next production observer path

For the next production phase, the selected observe-only path is:

```text
WH_KEYBOARD_LL
  → bounded native bridge
  → ObservedInputEvent
  → continuous ObserveService
  → future observe-only decision pipeline
```

This selection is phase-scoped, not permanent. Raw Input remains a validated
alternative for device identity and buffered high-rate input, and retains the
tradeoff of no direct injected-origin parity in the current boundary. Hook
callback timeout, message-loop dependency, and limited device identity remain
hardening/release risks. No input editing or injection exists.

### M3B-01 real-input diagnostic decision pipeline - IN PROGRESS

ADR 0008 records this intentionally narrow processor boundary.

The current diagnostic path is deliberately separate from native capture:

```text
zonkey-win observed events
  -> bounded ObserveService
  -> DiagnosticDecisionProcessor (zonkey-service)
  -> TelexEngine -> classifier -> SafePolicy
  -> sanitized decision category
```

`cargo run -p zonkey-cli -- diagnose` uses the existing hook source and this
processor. The processor consumes only validated `ObservedInputEvent` values;
key-up, shortcut, injected-origin, unsupported, and discontinuity cases do not
mutate a token across unsafe boundaries. It observes abstract policy outcomes
only and never executes an `EditPlan` or changes user text. Output is
token-length/decision metadata by default; `--show-token` is a temporary
foreground debug option with no persistence.

This is diagnostic evidence work, not a production IME runtime. Native code
remains unaware of Telex, dictionaries, detection, and policy. Foreground
context, Unicode reconstruction, persistence, injection, suppression, replay,
and text editing remain out of scope.

### M3B-02 decision-quality review - DONE

The review reuses the existing JSONL Telex and policy corpus harness rather
than changing runtime behavior. It covers 355 active fixtures and keeps 69
planned fixtures inactive. Active M2 evidence includes exact dictionary
recovery, Vietnamese negatives, never-transform structures, restricted
contexts, and explicit ambiguity. The live `hello` ambiguity is supported by
an active fixture. `pull` follows the recovery invariant that `raw == rendered`
provides no recovery evidence; `abc123` has active conservative coverage.

No detector threshold, dictionary entry, Telex rule, policy rule, or editing
capability is changed by M3B-02/M3B-03. Recovery candidate evidence requires
an observed transformation or mutation signal; dictionary membership alone is
insufficient for an unchanged token.

### M3B-04 conservative structured-token corpus coverage - DONE

Active never-transform fixtures already cover the high-risk structured-token
shapes represented by the current detector: URLs, domains, Windows and Unix
paths, command options, identifiers, IP literals, versions, UUIDs, hashes, and
secret-like values. The active corpus remains at 356 fixtures and the 69
planned structured-token cases remain inactive. No additional fixture was
necessary and no runtime or decision behavior changed.

### M3B-05 transformed English recovery coverage - DONE

This review isolates exact dictionary-backed recovery after Telex mutation.
The active corpus contains 43 English, 12 technical, and 8 product examples
with `raw != rendered`, all passing the existing classifier/policy harness.
Vietnamese and restricted-context cases remain negative controls. Planned
historical recovery-labelled records are not activation evidence; `pull` is
specifically a planned fixture requiring audit because current Telex evidence
does not show a mutation. No detector, policy, Telex, dictionary, threshold,
or runtime behavior changes in this review.

M3B-06 audited all 14 planned transformed records individually. None is ready
for activation: 13 duplicate active coverage and `pull` conflicts with the
unchanged-token recovery invariant. The owner decision is to activate none;
the planned set was historical/stale backlog and M3B-07 removed it from the
executable planned corpus. Active remains 356, planned is now 55. No runtime
behavior changes.

### M3B-08 transformed ambiguity review - DONE

The review inventory found five transformed `Ambiguous` cases, all in the
technical-editor context. They are expected conservative outcomes because
that policy requires technical-grade evidence; zero eligible writing-context
ambiguity cases were found. No behavior, corpus, detector, policy, or Telex
change is justified.

### M3C-01 restore simulation boundary - DONE

`zonkey-service` now exposes a bounded `RestorePlan` only when the existing
policy returns `RestoreCandidate`. It records logical Unicode-scalar lengths,
replacement data, and evidence while permanently remaining simulation-only;
Keep, Ambiguous, Unsupported, empty, injected, and discontinuity paths do not
produce an executable plan. The CLI may show sanitized plan metadata, with
token values limited to the existing `--show-token` diagnostic mode. No
Windows editing or foreground inspection is part of this milestone.
Owner manual validation confirmed `resume` simulation with
`replace_len=5`/`replacement_len=6`, while `dungf` produced Keep/no plan and
`hello` produced Ambiguous/no plan. No text modification occurred.

### M3C-02 restore-plan lifecycle and validation - DONE

The latest optional plan is the complete lifecycle model: semantic token
input, Backspace, boundaries, Escape, unsupported input, and discontinuity
invalidate it; a later RestoreCandidate replaces it. Key-up, modifier-only,
shortcut, and injected transitions do not create semantic token intent and
preserve an existing plan. Reading the plan is immutable and non-consuming.
No generation or one-shot simulation API is needed at this boundary.
Owner manual validation confirmed `resume` produced a current plan,
`hello` cleared it with Ambiguous/no plan, and `config` replaced it with a
new RestoreCandidate plan. Runtime counters were received=56, accepted=56,
dropped=0, processed=56, discontinuities=0, source_failures=0,
unsupported_events=0. No text modification occurred.

### M3C-03 restore-plan precondition model - DONE

`zonkey-service` provides fail-closed `PlanEligibility` validation for the
current in-memory plan. It checks presence, simulation-only execution state,
and internal Unicode-scalar span consistency without rerunning policy. The
result means only eligible for future execution consideration; foreground
contents, caret, selection, focus, window/process, IME, secure-field, and race
conditions remain unknown and out of scope.
Owner manual validation confirmed `resume` as
`eligibility=simulation-current` with `replace_len=5` and
`replacement_len=6`; `hello` and `dungf` both reported
`eligibility=no-plan`. Runtime counters were received=40, accepted=40,
dropped=0, processed=40, discontinuities=0, source_failures=0,
unsupported_events=0. No text modification occurred.

Zonkey là Rust workspace Windows-first nhưng phần lõi M1 hoàn toàn độc lập nền
tảng. M1 chỉ tạo quyết định và `EditPlan`; nó không phải IME cho người dùng cuối.

## Ranh giới và hướng dependency

```text
zonkey-types
   ↑       ↑
zonkey-token ──→ unicode-segmentation
   ↑
zonkey-telex
   ↑
zonkey-service → zonkey-token / zonkey-telex / zonkey-detect / zonkey-policy
             → zonkey-types
   ↑
zonkey-cli

zonkey-win → zonkey-service    (observe spike orchestration)
          → zonkey-types       (Windows boundary; native code cfg(windows))
```

- `zonkey-types`: event, boundary, action, unsupported result và `EditPlan`.
- `zonkey-token`: raw/rendered lifecycle và mọi phép tính grapheme.
- `zonkey-telex`: state machine deterministic; chỉ phụ thuộc token/types.
- `zonkey-detect` và `zonkey-policy`: M2 implementation đã hoàn tất; vẫn
  platform-neutral và không thực thi edit.
- `zonkey-win`: Windows observe-only spike boundary; native code is
  `cfg(windows)`-gated and does not inject or suppress input.
- `zonkey-cli`: có command `observe` cho M3A-04 manual spike; chưa là
  production IME hoặc replay runtime.

Core không phụ thuộc Win32, GUI, clock, locale, network hoặc filesystem. Serde và
serde_json chỉ là dev-dependency của test runner corpus.

## Event flow chính xác trong M1

```text
EngineEvent owned by caller
  ├─ Character(ASCII letter)
  │    → append TokenState.raw_ascii
  │    → deterministically compose entire raw token
  │    → TokenState.replace_rendered
  │    → grapheme diff(old rendered, new rendered)
  │    → EngineAction::Apply(EditPlan)
  ├─ Backspace
  │    → pop one raw ASCII key
  │    → deterministic recompose + grapheme diff
  │    → Apply(EditPlan), or Noop when empty
  └─ Boundary
       ├─ Space/Enter/Tab/Punctuation → clear token + Commit(boundary)
       └─ CursorMove/FocusLoss/Unknown → clear token + Reset
```

`EditPlan` chỉ mô tả số visible grapheme cần xóa và Unicode cần chèn. Nó không có
khả năng thực thi input và không giữ handle/API nền tảng. Corpus runner chỉ áp plan
vào một `String` in-memory để kiểm thử contract.

## Nguyên tắc an toàn

- `raw_ascii` và `rendered` là hai nguồn trạng thái độc lập.
- Không byte-index khi xóa/thay text; extended grapheme cluster là đơn vị visible.
- Non-ASCII direct input trả `UnsupportedBehavior` typed trong M1.
- English/IT recovery, context policy và never-transform detection để M2.
- Hook, injection, app context và UI để M3 trở đi.
