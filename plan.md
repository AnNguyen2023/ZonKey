# Zonkey — Technical Plan

> Mục tiêu: xây **Zonkey**, bộ gõ Telex Windows-first dành cho người dùng Việt làm kỹ thuật/IT. Zonkey phải giảm lỗi Telex làm biến dạng English/technical token, ví dụ `résume` → `resume`, đồng thời không làm hỏng tiếng Việt hợp lệ, code, CLI, URL hoặc password.

## 1. Bối cảnh Cay

Cay là dự án bộ gõ Telex native C++ đã phát triển từ Windows sang nhiều nền tảng. Những điểm cần xem là chuẩn hành vi (behavioral baseline), không phải mặc định để sao chép code:

- Core Telex được tách khỏi platform.
- Có Smart Bypass và Auto Restore: nhận diện token English, đặc biệt các pattern camelCase/snake_case, và khôi phục khi token kết thúc (ví dụ bấm Space).
- Windows dùng global keyboard hook kết hợp input injection.
- Có bản minimal/zero-CRT và bản full có tray, settings, excluded applications.
- Có adapter riêng cho macOS, Linux Fcitx5 và Android/JNI.

## 2. Inventory Cay cần audit

Khi clone Cay, lập `docs/cay-audit.md` theo bảng dưới. Không port code trước khi hoàn tất audit.

| Khu vực Cay | File/đường dẫn cần đọc | Mục đích audit |
|---|---|---|
| Telex core | `src/core/CayEngine.cpp`, `CayEngine.h` | State machine, thứ tự luật Telex, boundary, backspace, replay/revert, Auto Restore |
| Dữ liệu ngôn ngữ | `src/core/CayData.cpp`, `CayData.h`, `CayTypes.h` | Bảng nguyên âm/phụ âm, vần hợp lệ, scan và representation nội bộ |
| Windows hook | `src/platform/windows/shared/KeyboardHookManager.*` | Low-level hook, injected-event filtering, modifier, lifecycle và threading |
| Windows injection | `src/platform/windows/shared/InputInjector.*` | Backspace/replay/send Unicode, lỗi cursor, UAC/elevation, event loop |
| Minimal host | `src/platform/windows/minimal/main.cpp`, `no_crt.cpp` | Bootstrap, hotkey, config default, quyết định zero-CRT |
| Full Windows host | `src/platform/windows/full/main_full.cpp`, `ConfigManager.*` | Tray, settings, persistence, excluded app, lifecycle/UI |
| Build/release | `CMakeLists.txt`, `.github`, `scripts`, `INSTALL.md` | Target/compiler, packaging, signing, CI/CD |
| Other OS | `src/platform/macos`, `src/platform/fcitx5`, `src/platform/android` | Chỉ đọc để học ranh giới core/platform; không đưa vào v1 |
| License | `LICENSE` và các license trong `classic/` | Xác minh điều kiện fork/reuse trước khi copy bất cứ đoạn source nào |

### Câu hỏi audit bắt buộc

- Cay giữ `raw keystrokes`, text đã render, hay chỉ giữ buffer sau compose?
- Khi Auto Restore, engine xác định English bằng rule nào và recovery bằng raw replay hay Unicode decompose?
- Điều kiện chính xác để commit một token là gì: Space, Enter, Tab, punctuation, focus change hay cursor move?
- Backspace xử lý thế nào khi token vừa được auto-restored?
- Hook có bỏ qua event do chính injector tạo ra không? Dùng marker/flag nào?
- Có biện pháp chống re-entrancy, key stuck và race condition khi replay không?
- Excluded app được match bằng process path, executable name, window class hay title?
- Hành vi với browser, UWP, terminal, RDP, Office và app chạy elevated là gì?

## 3. Phạm vi Zonkey v1

### In scope

- Supported platform: Windows 11 x64 only (`x86_64-pc-windows-msvc`).
- Windows 32-bit, `i686-pc-windows-msvc`, 32-bit CI, and x86-specific
  compatibility workarounds are out of scope.
- Telex chuẩn và compatibility test với bộ case Cay/UniKey theo phạm vi được phép.
- English recovery theo token boundary.
- Technical dictionary, personal dictionary và never-transform rules.
- Chế độ quan sát (observe-only), dry-run diagnostics và undo correction.
- Per-app profiles: default, text-writing, technical-editor, terminal/secure.
- Tray UI tối giản, CLI quản lý config/dictionary.

### Out of scope v1

- macOS, Linux, Android.
- AI/LLM/cloud correction hoặc telemetry mặc định.
- Macro engine/phím tắt automation.
- Tự động sửa token confidence thấp.
- Cố gắng xử lý password/secure desktop/elevated windows không cùng integrity level.

## 4. Kiến trúc đề xuất

Zonkey nên dùng Rust workspace. Engine phải không phụ thuộc GUI hay Win32 để test được trên CI.

```text
zonkey/
├── Cargo.toml
├── crates/
│   ├── zonkey-types/       # Newtypes, event, token, decision, error
│   ├── zonkey-telex/       # Telex state machine, compose/reverse/replay plan
│   ├── zonkey-token/       # Raw/rendered buffer, boundaries, undo snapshot
│   ├── zonkey-detect/      # Classifier, lexical scoring, dictionary backend
│   ├── zonkey-policy/      # App profiles, rules, confidence threshold
│   ├── zonkey-config/      # Versioned config, migration, atomic persistence
│   ├── zonkey-win/         # Win32 hook, injector, foreground-process context
│   ├── zonkey-service/     # Runtime orchestration, event loop, logging
│   └── zonkey-cli/         # validate, diagnose, import/export dictionary
├── apps/
│   └── zonkey-settings/    # Tray/settings UI; không chứa Telex logic
├── assets/
│   └── dictionaries/
├── tests/
│   ├── corpus/
│   ├── golden/
│   └── windows-integration/
├── docs/
│   ├── architecture.md
│   ├── cay-audit.md
│   ├── decision-records/
│   └── threat-model.md
└── scripts/
```

### Luồng event

```text
Win32 key event
  → Event filter (ignore injected / modifiers / excluded context)
  → RawTokenBuffer
  → Telex state machine
  → RenderedTokenBuffer + edit plan
  → Token boundary?
      ├─ no: inject normal Telex edit plan
      └─ yes: Technical/English classifier + policy
               ├─ RestoreEnglish(raw token)
               ├─ KeepVietnamese(rendered token)
               └─ Ambiguous(keep rendered token)
  → Injector executes one atomic edit plan
  → Undo snapshot / local diagnostic event
```

## 5. Data model quan trọng

Không được chỉ nhìn Unicode text đã render. Luôn giữ raw sequence độc lập với rendered output.

```rust
pub struct TokenState {
    pub raw_ascii: String,
    pub rendered: String,
    pub boundaries: Vec<Boundary>,
    pub started_at: std::time::Instant,
}

pub enum RecoveryDecision {
    KeepVietnamese,
    RestoreEnglish {
        text: String,
        confidence: f32,
        reason: DecisionReason,
    },
    Ambiguous,
}

pub struct EditPlan {
    pub delete_graphemes: usize,
    pub insert_text: String,
    pub replay_modifiers: bool,
}
```

`EditPlan` là ranh giới giữa engine và Win32. Core quyết định **cần xóa gì/chèn gì**, platform chỉ thực thi. Không để `SendInput` hoặc handle Win32 lọt vào core crate.

## 6. Nhận diện English/IT

### Quy tắc an toàn

Chỉ auto-restore khi tất cả điều kiện phù hợp:

1. Rendered token thực sự đã bị Telex biến đổi.
2. Raw token là ASCII và khớp exact với dictionary hoặc pattern mạnh.
3. Context cho phép auto-correct.
4. Confidence đạt ngưỡng profile, mặc định cao (>= 0.98).
5. Token không thuộc never-transform.

Nếu mơ hồ, giữ nguyên tiếng Việt. False positive nguy hiểm hơn false negative.

### Nguồn điểm (scoring)

| Tín hiệu | Ví dụ | Hành động |
|---|---|---|
| Exact IT/product dictionary | `powershell`, `fortigate`, `vcenter`, `openssl` | Cộng điểm mạnh |
| English dictionary | `resume`, `refresh`, `folder`, `driver` | Cộng điểm |
| Code pattern | `camelCase`, `snake_case`, `PascalCase`, `kebab-case` | Bypass/khôi phục mạnh |
| Network/CLI pattern | URL, FQDN, IP, path, `-Parameter`, `/switch` | Never-transform |
| Vietnamese syllable hợp lệ | `dùng`, `cài`, `được` | Trừ điểm restore |
| User override | allow/deny list | Ưu tiên tuyệt đối |

### Dictionary ban đầu

Tách file và version control:

```text
assets/dictionaries/
├── en-common.txt
├── it-common.txt
├── products.txt
├── patterns.toml
└── never-transform.txt
```

Các term IT ban đầu: `PowerShell`, `OpenSSL`, `IIS`, `ESXi`, `vCenter`, `VMware`, `pfSense`, `FortiGate`, `DNS`, `DHCP`, `VLAN`, `RDP`, `XRDP`, `PKI`, `FQDN`, `hostname`, `registry`, `certificate`, `endpoint`, `inventory`.

User dictionary phải nằm trong profile người dùng, không commit vào Git.

## 7. Context và security

### Profile mặc định

| Profile | Ví dụ | Chính sách |
|---|---|---|
| Writing | Word, Outlook, Teams, browser text area | Telex + recovery ngưỡng cao |
| Technical editor | VS Code, Notepad++, IDE | Telex; technical recovery bảo thủ |
| Terminal | PowerShell, Windows Terminal, CMD, SSH | Không auto-recovery; chỉ hotkey/manual |
| Secure | password, credential/UAC/secure desktop | Không hook/inject hoặc tắt hoàn toàn |
| Remote | RDP/VM console | Disabled mặc định; opt-in sau test |

### Bắt buộc

- Không log raw user text mặc định.
- Diagnostics chỉ ghi event metadata, decision reason và hash/length token; raw text cần explicit opt-in.
- Không gửi network request.
- Không tự chạy elevated; nêu rõ giới hạn khi foreground app có integrity level cao hơn.
- Filter event injected để tránh feedback loop.
- Bảo đảm injection theo một transaction và có timeout/failsafe.

## 8. Test strategy

### Unit tests

- Telex: dấu, `dd`, `aw`, `aa`, `ee`, `oo`, `ow`, `uw`, tone placement, undo by repeated key.
- Buffer: whitespace, punctuation, cursor movement, selection, backspace, focus loss.
- Recovery: `resume → résume → resume`, `pull → pũll → pull`, cases mơ hồ và Vietnamese hợp lệ.
- Pattern: URL, path, IP, FQDN, e-mail, semantic version, UUID, hash, command flag.
- Config: migration, malformed config, atomic save, default recovery.

### Golden corpus format

```json
{
  "name": "restore-resume-at-space",
  "keys": "resume ",
  "context": "writing",
  "expected": "resume ",
  "decision": "RestoreEnglish"
}
```

### Manual integration matrix

- Notepad, UWP Notepad, Word, Outlook, Teams.
- Chrome/Edge/Firefox input and contenteditable.
- VS Code, Windows Terminal, PowerShell, CMD.
- RDP session, VMware/ESXi remote console where available.
- Standard user and elevated foreground app.
- Vietnamese keyboard ANSI/ISO and external USB keyboard.

## 9. Delivery milestones

Status labels below describe the repository state at the time of this
documentation sync: DONE, IN PROGRESS, PLANNED, or DEFERRED. The roadmap and
design intent remain unchanged; completed milestones are called out so that
future Windows runtime work is not mistaken for an existing implementation.

### M0 — Audit & specification (1 tuần) — DONE

- Clone Cay as read-only upstream reference.
- Hoàn tất `docs/cay-audit.md`.
- Tạo 150–300 corpus case từ thực tế gõ Việt/English/IT.
- Viết architecture decision records: hook vs TSF, GUI, config format, license boundary.

**Exit:** specs, test corpus và licensing decision được review.

### M1 — Pure Rust engine (2–3 tuần) — DONE

- `zonkey-types`, `zonkey-token`, `zonkey-telex`.
- Deterministic Telex behavior và golden tests.
- Chưa có hook/UI; this remains outside the completed milestone.

**Exit:** corpus Telex chạy green trên Windows CI.

### M2 — Recovery engine (2 tuần) — DONE

- `zonkey-detect`, dictionaries, policy engine.
- Detection, dictionaries, policy engine, and corpus coverage are implemented.
- Observe-only CLI replay remains PLANNED; the current CLI is not a runtime
  observer.
- Metrics local: false positive/false negative trong test corpus.

**Exit:** không auto-restore ca Vietnamese hợp lệ trong negative corpus quan trọng.

### M3A — Observe-only foundation

#### M3A-01 — Observed event contracts — DONE

- Validated, platform-neutral observed-input value types live in
  `zonkey-types`.
- These contracts do not observe input, expose native values, or modify text.

#### M3A-02 — Bounded observe-only mock pipeline — DONE

- `zonkey-service` provides the deterministic bounded queue and synchronous
  mock `EventSource`/processor pipeline.
- Overflow is loss-aware and aggregate-only; accepted events are drained
  FIFO. No runtime input observation or text editing exists.

#### M3A-03 — Observe adapter boundary contract — DONE

- See `docs/adr/0004-m3a03-observe-adapter-boundary.md`. The platform-neutral
  contract and its public-API evidence are complete; this does not imply a
  Windows observer or input editing.

#### M3A-04 — Windows observe-source spike — DONE

- ADR 0005 selects `WH_KEYBOARD_LL` for the first spike. The Windows-only
  runnable observer, bounded wake-message bridge, native mapping tests, and
  owner manual acceptance are complete.
- Manual evidence: `Shift+A` and `Ctrl+C` reached the service with matching
  modifier metadata; 49 callbacks, mappings, accepted and processed events;
  zero mapping rejects, bridge drops, lock drops, or service drops. The
  focused application continued receiving input normally.
- No suppression, injection, replay, text editing, or foreground inspection is
  implied.

#### M3A-05 — Windows observer hardening review — DONE

- ADR 0006 proposes retaining the validated `WH_KEYBOARD_LL` spike
  temporarily while requiring a bounded Raw Input comparison before
  production release or decision-pipeline integration.
- The production native boundary, ownership, loss, diagnostics, privacy, and
  preconditions are documented; no implementation changes are included.
- Owner accepted Decision C and the M3A-06 comparison is complete. The next
  production phase selects `WH_KEYBOARD_LL`; Telex/detection/policy
  integration remains a separate observe-only milestone.

#### M3A-06 — Raw Input comparison spike — DONE

- `zonkey-win` now has a parallel hidden-window Raw Input observer using
  `RIDEV_INPUTSINK` without `RIDEV_NOLEGACY`.
- `observe-hook` preserves the M3A-04 hook path; `observe-raw` runs the Raw
  Input comparison path. Both use the existing observe-only service contract.
- Automated mapping and bounded-handoff tests pass. Owner controlled
  comparison passed for both paths; `WH_KEYBOARD_LL` is selected for the next
  production phase because direct injected-origin visibility is available.
- Raw Input remains a validated fallback for device identity and buffered
  high-rate input; it is not permanently rejected.
- Owner evidence records Raw Input 36/36 processed with no drops and Hook
  96/96 received, accepted, and processed with no drops. A sustained hook run
  previously exposed 53 downstream drops; M3A-07 resolved that bottleneck.
- No Telex/detection/policy integration, injection, editing, suppression, or
  foreground inspection is in scope.

#### M3A-07 — Continuous observe-service consumption — DONE

- `ObserveService` now processes one queued event after each source event and
  drains the remainder only at terminal exhaustion/stop.
- This deterministic single-thread fairness rule prevents a live source from
  filling capacity 256 solely because processing was deferred until shutdown.
- Source failure remains terminal without draining pending events; exhaustion
  and explicit stop retain FIFO drain semantics.
- Automated sustained-source evidence passes; Windows mechanism selection and
  Telex/detection/policy integration remain out of scope.

### M3 — Windows prototype (2–3 tuần) — PLANNED

- `zonkey-win` với low-level hook và safe event filtering.
- Notepad/browser test; edit plan + undo.
- Observe-only và manual enable flag.

#### M3B-01 - Real-input diagnostic decision pipeline - IN PROGRESS

- `diagnose` feeds the existing Windows observe source into a stateful,
  platform-neutral Telex/detection/policy diagnostic processor.
- Only semantic key-down events in the bounded A-Z/digit/basic-boundary scope
  mutate token state. Key-up, shortcuts, injected events, unsupported keys,
  and discontinuities are ignored or reset conservatively.
- Diagnostics are sanitized and observation-only; no edit plan is executed,
  no user text is changed, and `--show-token` is an explicit temporary debug
  flag. Native capture remains unaware of dictionaries and policy.

This milestone remains in progress pending owner Windows diagnostic evidence.

#### M3B-02 - Diagnostic decision-quality review - DONE

- Existing active/planned Telex and M2 JSONL fixtures were reviewed through
  the existing deterministic corpus harness; no detector, dictionary, Telex,
  or policy behavior was changed.
- Active M2 evidence covers exact English/technical/product recovery,
  Vietnamese negatives, never-transform structures, restricted contexts, and
  expected ambiguity. `hello` is an active expected ambiguity.
- Owner accepted the unchanged-token recovery invariant: `raw == rendered`
  yields no recovery evidence from dictionary membership alone. `hello`
  remains expected ambiguity; `pull` remains conservative; `abc123` now has
  active ambiguous coverage.

#### M3B-03 - Owner-approved conservative decision fixtures - DONE

- Added active `abc123` ambiguous coverage and retained the existing `pull`
  unchanged-dictionary regression and `hello` ambiguous fixture.
- No detector, dictionary, Telex, policy, threshold, or runtime behavior was
  changed. The milestone records product expectations without broad tuning.

#### M3B-04 - Conservative structured-token corpus coverage - DONE

- Reviewed active never-transform coverage for URLs, domains, Windows and
  Unix paths, IP literals, command options, identifiers, versions, UUIDs,
  hashes, and secret-like tokens.
- Coverage is sufficient for the existing detector semantics; no fixture gap
  justified adding another active case. Active corpus remains 356 and the 69
  planned structured-token fixtures remain inactive.
- No detector, policy, Telex, dictionary, Windows, or runtime behavior changed.

#### M3B-05 - Transformed English recovery coverage - DONE

- Reviewing one category only: exact dictionary-backed English/technical/
  product recovery after a real Telex mutation (`raw != rendered`).
- Existing active evidence covers 43 English, 12 technical, and 8 product
  cases with no active mismatches. Vietnamese and restricted-context cases
  remain negative controls.
- Planned historical recovery fixtures remain inactive. No detector, policy,
  Telex, dictionary, threshold, Windows, or runtime behavior changed.
- Planned recovery-labelled records, including `pull`, are not treated as
  activation evidence until individually audited.

#### M3B-06 - Planned transformed-recovery fixture audit - DONE

- Audit all 14 planned transformed records as valid transformation,
  unchanged-token, already-covered, outdated expectation, or unclear.
- Do not activate or modify fixtures until owner review; no detector or policy
  tuning is in scope.
- See `docs/m3b-06-planned-recovery-audit.md` for the complete row-by-row
  evidence table and readiness decision.
- Owner decided to activate none of the 14 records; the set is historical/
  stale backlog pending corpus hygiene.

#### M3B-07 - Planned corpus hygiene - DONE

- Documentation/corpus-maintenance decision only: determine whether obsolete
  or redundant planned records should be removed, archived, or rewritten when
  they still represent approved product intent.
- Do not activate redundant records and do not change detector, policy, or
  Telex semantics.
- Owner-approved disposition removed all 14 audited records from the planned
  corpus: active remains 356 and planned is now 55. See
  `docs/m3b-07-planned-corpus-hygiene.md`.

#### M3B-08 - Transformed ambiguity review - DONE

- Review-only inventory of genuinely transformed tokens that remain ambiguous.
- Current inventory has five technical-editor cases; all are expected
  conservative outcomes, and zero eligible writing-context cases were found.
- No detector, policy, Telex, dictionary, or fixture changes are justified.
- See `docs/m3b-08-transformed-ambiguity-review.md`.

#### M3C-01 - Restore simulation boundary - DONE

- Convert an approved `RestoreCandidate` into a deterministic, inspectable
  platform-neutral `RestorePlan`; never execute it or modify user text.
- Keep logical Unicode-scalar span counts and bounded latest-plan lifetime.
- No Windows editing, foreground inspection, clipboard, replay, suppression,
  or other execution mechanism; any execution requires a separate approved
  milestone. See ADR 0009.
- Owner manual validation: `resume` produced `replace_len=5` and
  `replacement_len=6`; `dungf` reported Keep with no plan; `hello` reported
  Ambiguous with no plan. Runtime counters were received=16, accepted=16,
  dropped=0, processed=16, discontinuities=0, source_failures=0,
  unsupported_events=0.

#### M3C-04 - Restore-plan handoff contract - DONE

- Capture an eligible current plan as an immutable, platform-neutral
  simulation snapshot; stale snapshots remain historical only after service
  invalidation or replacement.
- No generation, revalidation, round-trip execution, Windows consumer, or
  OS/editor inspection is introduced. See ADR 0012.
- Deterministic tests prove capture-time eligibility, stale-after-invalidation
  snapshot semantics, and replacement by a newer candidate; no manual CLI
  validation is required.

#### M3C-05 - Handoff revalidation model - DONE

- Compare captured handoffs with current service state using a deterministic
  service-local generation; identical-content stale snapshots are rejected.
- Current/Stale is simulation metadata only. No OS/editor verification,
  execution, or persistence is introduced. See ADR 0013.
- Generation is service-local `u64`, starts at 1, increments only for new
  RestoreCandidate plans, and uses checked allocation that fails closed
rather than wrapping or reusing identity.

#### M3C-06 - Internal execution-gate contract design - DONE

- Compose current plan, eligibility, handoff, and revalidation into a
  fail-closed service-side gate that stops before external validation.
- PASS never means executable or safe-to-edit; foreground/editor evidence,
  secure state, operation units, and observation races remain unknown. See
  ADR 0014.

- Owner accepted the service-side composition of plan eligibility, handoff,
  revalidation, generation, span, and simulation-only evidence.
- `PassedForExternalValidation` stops before external validation and never
  authorizes mutation; external requirements and observation races remain
  unknown. Deterministic service tests cover pass and fail-closed rejection.

#### M3C-07 - External-validation requirements specification - DONE

- Documentation-only definition of required external evidence, unknown/veto
  semantics, freshness, operation units, and race control.
- Specifies WHAT a future adapter must prove, not HOW; no OS/editor
  acquisition or mutation is implemented. Owner accepted the exact-match,
  fail-closed, unresolved-operation-unit, logical-freshness, and race
  requirements. M3C-09 is proposed for design-only controlled-test-surface
  design. See
  ADR 0015.

#### M3C-08 - External evidence acquisition architecture review - DONE

- Design-only comparison of UI Automation/accessibility, TSF, application-
  specific adapters, and a controlled synthetic test surface.
- Reviews exact text/caret binding, identity, composition, secure/session
  evidence, operation units, freshness, atomicity, privilege, and compatibility.
- Recommendation C is a controlled test-surface design before OS acquisition;
  it is test infrastructure, not a production adapter. No Windows API,
  inspection, mutation, or dependency is selected; atomicity and production
  evidence remain unresolved. See ADR 0016.

#### M3C-09 - Controlled external-validation test-surface design - DONE

- Design-only contract for a ZonKey-owned deterministic surface: exact text,
  abstract TextUnit coordinates, caret/selection, logical identity,
  composition, secure/session state, checked revision snapshots, and race
  injection.
- Defines a test-only compare-and-replace oracle and fail-closed rejection
  vocabulary. It does not prove production safety and adds no implementation,
  OS inspection, mutation, or dependency. Owner accepted the design; M3C-10
  completed the test-only platform-neutral implementation. See ADR 0017.

#### M3C-10 - Controlled validation surface implementation - DONE

- Implement the approved surface as `#[cfg(test)]` infrastructure in
  `zonkey-service`, with deterministic state, snapshots, race injection, and
  compare-and-replace tests.
- Keep TextUnit distinct from byte/scalar assumptions and preserve production
  non-equivalence. No OS/editor acquisition or mutation is included.
- M3C controlled-model phase is COMPLETE; production acquisition and execution
  are not approved or implemented.

#### M3D-01 - Production acquisition readiness review - DONE

- Readiness-only selection of at most one target class and acquisition family:
  standard Win32 edit controls plus the UI Automation/accessibility provider
  family, excluding secure/password fields.
- Defines exact evidence to observe, unknown/fail-closed states, freshness and
  race limits, privilege/compatibility boundaries, and explicit stop conditions.
  No OS API, inspection, mutation, or dependency is implemented. Owner
  approved readiness only; production editing remains out of scope. See ADR
  0018.

#### M3D-02 - Narrow Win32 edit-control acquisition spike - BLOCKED_BY_UIA_ENVIRONMENT

- Read-only UIA probe limited to a standard Win32 `Edit` control, excluding
  secure/password fields and rejecting unknown, denied, stale, contradictory,
  or unsupported evidence.
- Sanitized evidence only: provider/class identity, empty selection, and exact
  candidate text immediately before the caret. UIA/COM remains isolated in
  `zonkey-win`; no mutation, editing, or production execution is included. See
  ADR 0019. Windows-only test fixtures create and destroy owned controls for
  validation; they do not modify external applications.
- Standard Edit identity resolves, but usable UIA TextPattern is unavailable in
  the current environment; UIA validation is not claimed.

#### M3D-04 - Narrow standard-EDIT native read-only fallback - DONE

- Diagnostic-only `zonkey-win` path reads standard Win32 `Edit` text and
  empty-caret selection with bounded native messaging, rejecting secure,
  failed, timed-out, and contradictory states; cross-process selection reads
  are bounded.
- A fixed two-sample comparison rejects observed identity, style, text, or
  selection changes. This is partial staleness detection only; reads remain
  non-atomic and imply no execution or mutation safety. See ADRs 0020 and
  0021.
- Overall narrow acquisition validation is via the native fallback only.

#### M3D-05 - Coherence/freshness boundary - DONE

- Outcome: `PARTIAL_STALENESS_DETECTION_ONLY`.
- No atomic snapshot, future freshness guarantee, or validation-to-mutation
  atomicity is claimed.

#### M3D-17 - VS Code host adapter spike - DONE

- Smallest VS Code extension-side adapter mapping the cooperating-host
  contract onto VS Code APIs for one local ordinary
  `TextDocument`/`TextEditor` with a single caret, under
  `hosts/vscode-spike`: evidence snapshot (protocol id, host/session identity,
  document URI + open-instance epoch, editor identity, document version,
  UTF-16 range + exact expected text + replacement, empty selection/caret,
  capability flags) and one `TextEditor.edit` compare-and-replace with an
  immediate pre-edit re-read and post-commit verification (exact next version
  and exact resulting text).
- Fail-closed rejection mirrors the service vocabulary plus
  `EditTransactionRefused`; lost or ambiguous outcomes are `Indeterminate`
  and are never auto-retried; request ids replay recorded results, conflicting
  reuse rejects, and session restart clears the ledger.
- VS Code exposes no IME composition state: the real binding reports
  composition `Unknown`, so real-VS-Code applies fail closed
  (`CompositionUnknown`); no composition proof is claimed and the `Applied`
  path is dummy-harness-proven only. No auto-restore wiring, hooks, injection,
  clipboard, SendInput, multi-host, or generic editor support. See ADR 0022.

#### M3D-18 - Composition evidence feasibility review - DONE

- Design-only review with one narrow real-VS-Code capability probe. Verdict:
  `NO_TRUSTWORTHY_COMPOSITION_BINDING`.
- VS Code exposes no composition API, and document-change heuristics do not
  measure composition state, so they do not qualify as evidence. TSF is
  in-process COM: no supported mutation-free cross-process observer exists,
  and external reads require running code inside the host process. A
  sanitized read-only UIA probe measured that the real VS Code 1.133
  Chromium provider does expose `TextEditPattern` (root web area plus Edit
  elements) and that `GetActiveComposition` returns a measurable
  no-composition state (`S_OK` with a null range) when no IME is active.
- The verdict stays fail-closed because the TextEdit signal carries no
  element identity that binds to the targeted VS Code editor/document of the
  host contract, an active-composition usable range is unproven (producing
  one would require injecting real IME input), and read-to-snapshot
  coherence cannot be closed from outside the host transaction.
  `CompositionUnknown` stays fail-closed; no TSF, IPC, transport, mutation,
  or API selection is added. See ADR 0023.

#### M3D-19 - Bounded request ledger and transport boundary - IN PROGRESS

- Hardens cooperating-host protocol plumbing without enabling real mutation:
  a platform-neutral `zonkey-service::transport` boundary (length-prefixed
  UTF-8 frames capped at 64 KiB with fail-closed malformed/oversized
  handling, a `zonkey.host-transport/1` hello that binds exactly one session
  id as the only trusted identity, and timeout/connection-loss mapping to a
  recorded ambiguous outcome that is never retried) plus bounded ledgers on
  both sides of the contract.
- Both ledgers use fixed capacity with deterministic FIFO eviction (oldest
  inserted id first, lookups never refresh order); exact duplicates replay
  recorded results without re-execution, conflicting reuse rejects, session
  restart invalidates history, and every outcome kind including
  `Indeterminate` is replayed verbatim. Evicted duplicates re-validate
  against live host state before anything executes.
- The Windows 11 x64 destination transport is a localhost named pipe; that OS
  binding is deliberately not implemented. Real VS Code applies remain
  fail-closed at `CompositionUnknown`; no composition, mutation, or
  auto-restore policy changed. See ADR 0024.

#### M3D-20 - Windows named-pipe transport spike - IN PROGRESS

- Narrow Windows 11 x64 named-pipe binding of the M3D-19 transport contract,
  isolated in `zonkey-win::pipe_transport`: duplex byte-mode pipe, bounded
  64 KiB frames, `HELLO`/`WELCOME` handshake binding one server-issued
  session id, protocol/session mismatch rejection before execution,
  malformed/oversized frames failing closed by disconnection, a bounded
  client read timeout via `CancelSynchronousIo`, and clean listener/session
  teardown. Duplicate request ids replay recorded outcomes over the pipe;
  a disconnect after a request is `Indeterminate` and is resolved by replay,
  never by automatic retry.
- Security is stated exactly: default process DACL (creating user, admins,
  local system), no explicit per-user ACL, no impersonation, no trust in
  PID/HWND/pipe name; the session id is a correlation token, not
  cryptographic authentication, and no stronger claim is made.
- Ten real-pipe tests cover hello, roundtrip with a dummy host, duplicate
  replay, protocol and session mismatch, malformed and oversized frames,
  disconnect ambiguity plus replay, server restart invalidation, and the
  bounded timeout. No VS Code `Applied`, no composition change, no mutation,
  no auto-restore wiring. See ADR 0025.

#### M3D-21 - Real VS Code transport integration, query/reject only - IN PROGRESS

- End-to-end connection between the real VS Code extension host and the real
  Windows named-pipe transport: a TypeScript pipe client in the spike
  speaking the exact M3D-20 wire protocol, a query-only composition gate in
  `zonkey-service::transport` (Unknown/Active fail closed; even Inactive
  returns `rejected:ExecutionNotImplemented`), and
  `zonkey-cli serve-host-validation` exposing the endpoint.
- The real-VS-Code integration test runs real processes and asserts:
  activation, snapshot with `composition: Unknown`, the session-bound
  handshake, the request crossing the pipe, the recorded duplicate replay,
  same-tick disconnect resolving through replay (never automatic retry), and
  an untouched document. Measured facts: the ext host needs the
  `\\?\pipe\` device-path form, and the spike pipe serves one connection at
  a time.
- No `TextEditor.edit` is enabled, composition policy is unchanged, and no
  mutation path exists in the chain. Security claims stay exactly as ADR
  0025. See ADR 0026.

#### M3D-22 - Real decision handoff into transport request - IN PROGRESS

- `zonkey-service::transport::build_host_request` maps a submitted
  `RestorePlanHandoff` through revalidation and the internal execution gate
  into a `HandoffRequest` (rendered token, replacement, scalar units,
  generation; deterministic `handoff-<generation>` request id). Every other
  outcome — no current plan, stale handoff, generation mismatch, malformed
  span, gate failure — rejects before any transport involvement. No
  host-native range is derived from service scalar lengths; the UTF-16 range
  stays owned by the host snapshot/adapter.
- The pipe binding gains a read-only, session-checked `HANDOFF` query with a
  `HandoffProvider`; `zonkey-cli serve-host-validation --handoff-token resume`
  drives the real decision pipeline (scripted platform-neutral events, not
  runtime input) and serves the validated handoff. The VS Code side queries
  the handoff, merges it with its real snapshot, and sends the request under
  the handoff's request id: the real E2E stays `rejected:CompositionUnknown`,
  duplicates replay, and the document is untouched.
- No mutation, no Applied path, no composition bypass. See ADR 0027.

#### M3D-23 - Live observer to handoff to transport E2E - DONE

- `SharedDecisionState`/`SharedDecisionProcessor` in
  `zonkey-service::transport` share the one real decision processor between
  the unchanged `WH_KEYBOARD_LL` observer loop and the pipe endpoint's
  handoff provider; no second input pipeline exists. Wiring tests re-verify
  injected-event exclusion, shortcut isolation, discontinuity resets,
  Keep/Ambiguous producing no request, only-current-eligible handoff, and
  deterministic `handoff-<generation>` identity. The host keeps owning the
  UTF-16 range.
- `zonkey-cli handoff-live` runs the live endpoint. The owner's first
  manual run exposed a self-invalidating 3-terminal race (the keystrokes
  needed to launch E2E were observed by the global hook and invalidated the
  handoff); the fixed single-command harness `npm run smoke:m3d23`
  pre-starts everything, polls for the live handoff, and on the first
  `HANDOFF_OBSERVED` closes its poll connection and runs the real VS Code
  validation automatically, ending at `rejected:CompositionUnknown` with an
  untouched document. A labeled `--scripted-tooling-check` proves harness
  mechanics only.
- Owner live smoke PASSED (2026-08-16): real physical keyboard, live
  `RestoreCandidate` (`handoff:handoff-1|réume|resume|5|6|1`), real pipe +
  real VS Code, `M3D23_LIVE_SMOKE_OK` exit 0, no SendInput or scripted feed
  as evidence, document unchanged. See ADR 0028.

#### M3D-24 - Production boundary / remaining blockers review - DONE

- Design-only classification of remaining blockers before any real `Applied`
  path, based on the validated live end-to-end chain. Two blockers are
  MUST_SOLVE_BEFORE_APPLIED: composition evidence (ADR 0023 conditions) and
  the crash/Indeterminate recovery policy; mutation eligibility itself must
  be designed and approved first. Pipe ACL, ledger bounds, host/session
  identity, transport lifecycle, packaging, and diagnostics/privacy are
  HARDEN_BEFORE_RELEASE with fail-closed defaults today.
- Verdict: `READY_FOR_CONTROLLED_MUTATION_DESIGN` — the design may proceed
  while remaining unimplementable until the composition source exists and
  the recovery policy is approved. No mutation is designed or implemented in
  M3D-24. See ADR 0029.

#### M3D-25 - Controlled mutation design - IN PROGRESS

- Design-only contract for any future cooperating-host `Applied`: a
  fixed-order eligibility checklist (protocol/session/host/document/editor
  identity, revalidated current handoff and internal gate, local ordinary
  file document, one empty caret, host-owned UTF-16 range with exact
  expected text and matching version, `KnownNonSecure`, `SupportedLocal`,
  proven-Inactive composition with `CAP_COMPOSITION_PROOF`, valid
  idempotent request id, well-formed replacement) where any unknown or
  mismatch rejects; a one-transaction compare-and-replace with immediate
  pre-transaction re-read and post-commit verification (`Applied` only when
  transaction accepted, range/text matched at transaction time, exact
  replacement committed, and verification consistent); and an
  `Indeterminate` recovery policy (never auto-retry, ledger replay for
  duplicates, unproven outcomes stay Indeterminate, an explicit
  owner-acknowledged reconciliation readback before any new request for the
  same logical target, no best-guess retry).
- Real VS Code composition remains `Unknown`, so real Applied stays
  disabled (`Rejected(CompositionUnknown)`); the proven-Inactive
  precondition is defined, not invented. No mutation is implemented. See
  ADR 0030.

#### M3D-26 - Controlled mutation contract harness - IN PROGRESS

- Test-only implementation of the ADR 0030 contract on a deterministic
  dummy cooperating host (`#[cfg(test)]` in `zonkey-service`): fixed-order
  eligibility validator, one atomic host-owned compare-and-replace with the
  in-transaction re-read as the only authorization window, post-commit
  verification (exactly `revision + 1` and the exact intended text), the
  reused bounded idempotency ledger, and the Indeterminate workflow that
  blocks the logical target until a reconciliation readback plus explicit
  owner acknowledgement.
- Synthetic composition evidence exists only in the harness: `Inactive`
  requires `CAP_COMPOSITION_PROOF`, and an unproven Inactive still fails
  closed as `CompositionUnknown`. Twenty deterministic tests cover the
  happy path, every major rejection, duplicate/replay semantics, lost and
  ambiguous outcomes, all three reconciliation verdicts, the blocked-target
  lifecycle, revision overflow, and no-partial-mutation. The real VS Code
  binding is untouched and still returns `CompositionUnknown`; no mutation
  path exists in production. See ADR 0031.

#### M3D-27 - Production hardening priority review - DONE

- Design-only classification of the remaining areas: composition evidence
  and recovery-policy approval/tooling are the two BLOCKS_REAL_APPLIED
  items; pipe ACL/authentication, per-boot pipe identity, ledger
  sizing/persistence, host/session identity hardening, and
  startup/packaging/VSIX gate the release; reconnect/multi-client,
  diagnostics/privacy, and reconciliation UX are SHOULD_HARDEN; nothing is
  currently DEFER. Real Applied stays disabled and `CompositionUnknown`
  stays fail-closed.
- Next milestone chosen (exactly one): M3D-28 production reconciliation and
  recovery tooling — the only Applied-blocking item fully within our
  control, converting the ADR 0030 recovery workflow into transport-grade
  reconcile/readback and owner-ack tooling without any mutation. See ADR
  0032.

#### M3D-28 - Production reconciliation and recovery tooling - DONE

- `RecoveryRegistry` in `zonkey-service::transport`: bounded (FIFO
  eviction), in-session blocked-target registry implementing the ADR 0030
  recovery lifecycle — Indeterminate blocks the logical target (URI +
  rendered token); only an explicit reconciliation readback
  (`AppliedAcknowledged`/`NotApplied`/`ConflictHumanReview`, idempotent)
  plus an explicit operator acknowledgement unblocks it; ack-before-
  reconcile and wrong sessions are rejected; persistence is deliberately
  absent (restart empties state; durable recovery stays release-gated).
- The pipe protocol reuses the session-bound framing with one `RECOVERY`
  command family (LIST/BLOCK/RECONCILE/ACK); `zonkey-cli recovery` exposes
  the operator surface; both endpoints carry the shared registry. Verified
  by registry and real-pipe tests plus a real-VS-Code E2E
  (`npm run test:recovery`) ending in an untouched document. No mutation,
  no Applied path, `CompositionUnknown` unchanged. See ADR 0033.

#### M3D-29 - Named-pipe ACL / peer-truth hardening - DONE

- Every created pipe instance carries an explicit DACL granting the
  creating user's SID `GENERIC_ALL` and nothing else (`SetEntriesInAclW`);
  the default process DACL is never used, so other interactive users,
  `Everyone`, administrators, and `LOCAL SYSTEM` are denied by omission.
  Admins can still take ownership and rewrite the DACL — documented
  residual, not a defect. `FILE_FLAG_FIRST_PIPE_INSTANCE` fails creation
  closed on name squatting; if the DACL, nonce, or peer inputs cannot be
  built, no listener is created at all.
- Peer truth for the single-user architecture = current-user SID, enforced
  twice: OS DACL at connect, plus fail-closed server-side verification
  (identification-level impersonation via `SECURITY_IDENTIFICATION` SQOS,
  token user SID compare with `EqualSid`, always `RevertToSelf` before
  serving). PID/HWND/pipe name are never trusted identity; no
  cryptographic authentication is claimed.
- Per-lifecycle identity: `generate_pipe_name` emits
  `\\.\pipe\zonkey-<prefix>-<128-bit BCrypt nonce>`; session ids embed a
  128-bit nonce. Restart ⇒ new identity; stale pipe names fail closed and
  stale sessions are rejected before execution.
- Evidence-based robustness fix: `open_raw_handle` returned
  `ConnectTimeout` after a single 50 ms `WaitNamedPipeW` miss on
  `ERROR_PIPE_BUSY`, ignoring the caller's deadline (reproduced under
  parallel load in three tests); it now honors the full deadline. No test
  timeout was loosened.
- Verified by DACL inspection on a live pipe (current-user-only allow
  ACEs), nonce/name uniqueness, restart-identity death, and the unchanged
  protocol/session/frame/ledger/disconnect/timeout suite under repeated
  full-workspace runs. No mutation, no Applied, `CompositionUnknown`
  unchanged. See ADR 0034.

#### M3D-30 - Production ledger sizing / persistence review - DONE

- Design-only review closing the ADR 0032 BLOCKS_RELEASE ledger item; no
  implementation, real Applied still disabled, `CompositionUnknown`
  fail-closed.
- Capacities: ledger 256 (hard bound ≈ 32 MiB, typical ≈ 1 MiB), recovery
  registry 128; new fail-closed eviction rules — ledger eviction of an
  Ambiguous entry escalates to a blocked target, and the registry never
  evicts unresolved blocks (full-and-unresolved ⇒ refuse new blocks).
- Must survive restart: unresolved blocked targets with reconcile/ack
  state only. Definite outcomes and sessions never persist (compare-and-
  replace backstop; M3D-29 per-lifecycle sessions). Durable record = URI +
  salted SHA-256 of expected/replacement + range + state; document text
  never persists in plaintext.
- Restart semantics: unresolved blocks ⇒ recovery-required mode (blocked
  targets reject until reconcile + owner ack); persisted entries rebind to
  the current session on first command; corrupt/torn state file fails
  closed with a typed error, never defaults to "no blocks".
- Storage model chosen: compact durable state file (≤ 256 KiB hard cap,
  versioned + CRC32, temp-write → `FlushFileBuffers` →
  `MoveFileExW(WRITE_THROUGH|REPLACE_EXISTING)`), single writer,
  current-user-only DACL reusing the M3D-29 guard. Append-only journal and
  no-persistence models evaluated and rejected for the release posture.
- Verdict: **DURABLE_RECOVERY_MODEL_READY** — implementation is a separate
  owner-approved milestone. See ADR 0035.

#### M3C-02 - Restore-plan lifecycle and validation - DONE

- Validate bounded current-plan ownership and deterministic invalidation.
- Semantic input, Backspace, new decisions, Escape, unsupported input, and
  discontinuity invalidate or replace stale plans; non-semantic transitions
  remain read-only and do not create plans.
- No plan identity, simulated consumption, execution, or OS mutation is
  introduced. See ADR 0010.
- Owner manual validation: `resume` created a plan, `hello` cleared it with
  Ambiguous/no plan, and `config` deterministically replaced it. Runtime
  counters were received=56, accepted=56, dropped=0, processed=56,
  discontinuities=0, source_failures=0, unsupported_events=0.

#### M3C-03 - Restore-plan precondition model - DONE

- Add fail-closed, read-only eligibility metadata for a current simulation
  plan; this never means executable or safe to edit.
- Validate only plan presence, simulation-only state, and internal scalar
  span consistency. Do not rerun policy or claim foreground/editor state is
  known. See ADR 0011.
- Owner manual validation confirmed `resume` eligibility, and no-plan
  results for `hello` (Ambiguous) and `dungf` (Keep). Runtime counters were
  received=40, accepted=40, dropped=0, processed=40, discontinuities=0,
  source_failures=0, unsupported_events=0.

The M3A-04 observe-source spike is present, but production Windows runtime
observation, input modification, and injection remain out of scope.

**Exit:** không key stuck, không loop injected event, không mất text ở test matrix cơ bản.

### M4 — Usable desktop build (2 tuần) — PLANNED

- Tray, config, per-app profile, dictionary management CLI.
- Installer/portable package, logging opt-in, crash reporting local.
- Signing plan và release CI.

**Exit:** daily-driver pilot trên 1–3 máy trong 2 tuần.

### M5 — Hardening (liên tục) — DEFERRED

- Regression corpus từ các correction bị undo.
- RDP, UWP, Office, Chromium, terminal compatibility.
- Performance, startup, memory, recovery/rollback tests.

## 10. Công nghệ đề xuất

- Rust stable, edition hiện hành tại thời điểm khởi tạo.
- `windows` crate cho Win32 bindings; cô lập trong `zonkey-win`.
- `serde` + TOML/JSON versioned config.
- Unicode grapheme-aware xử lý text; không dùng index byte để xóa text.
- `tracing` cho diagnostics có redaction.
- UI: native tray/settings tối giản; chọn framework sau M3. Không dùng Tauri/Electron trong input hot path.
- CI: GitHub Actions, `cargo fmt --check`, Clippy, unit tests, corpus tests, Windows build matrix.

## 11. Quyết định cần chốt sớm

1. **License:** chỉ reuse Cay sau khi đọc LICENSE và xác nhận toàn bộ dependency/license compatible; nếu không, implement clean-room theo behavioral specification.
2. **Input technology:** v1 hook + injection để prototype, nhưng đánh giá TSF trước bản production. TSF phức tạp hơn nhưng có thể tương thích text services tốt hơn.
3. **Platform:** Windows 11 x64 only for v1; Rust target is
   `x86_64-pc-windows-msvc`. Windows 32-bit and cross-platform targets are not
   supported.
4. **Default policy:** auto-recovery chỉ ở writing profile với confidence cao; terminal và remote tắt mặc định.
5. **Privacy:** local-first, offline-first, telemetry off.

## 12. Definition of Done v1

- Gõ Telex tiếng Việt ổn định trong test corpus.
- Tự khôi phục English/IT token chính xác ở writing profile và có undo ngay lập tức.
- Không tự đổi URL, path, IP, e-mail, code identifier hoặc password-like token.
- Có per-app policy, excluded apps, config backup và reset.
- Không key stuck, feedback loop hoặc mất text trong integration matrix cốt lõi.
- Build/release Windows reproducible, source license rõ ràng, binary được ký khi phát hành rộng.
