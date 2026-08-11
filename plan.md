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

- Windows 11 x64 trước; Windows 10 là compatibility target.
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
3. **Platform:** Windows-only v1; không hứa cross-platform trong roadmap ban đầu.
4. **Default policy:** auto-recovery chỉ ở writing profile với confidence cao; terminal và remote tắt mặc định.
5. **Privacy:** local-first, offline-first, telemetry off.

## 12. Definition of Done v1

- Gõ Telex tiếng Việt ổn định trong test corpus.
- Tự khôi phục English/IT token chính xác ở writing profile và có undo ngay lập tức.
- Không tự đổi URL, path, IP, e-mail, code identifier hoặc password-like token.
- Có per-app policy, excluded apps, config backup và reset.
- Không key stuck, feedback loop hoặc mất text trong integration matrix cốt lõi.
- Build/release Windows reproducible, source license rõ ràng, binary được ký khi phát hành rộng.
