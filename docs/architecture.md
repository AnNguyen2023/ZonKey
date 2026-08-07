# Kiến trúc M1


## M2 detection and policy boundary

M2 keeps lexical facts and recovery policy separate. Raw text, rendered text, boundary, and caller context enter zonkey-detect. LexicalEvidence then enters zonkey-policy, which returns a RecoveryDecision and optional EditPlan.

zonkey-detect does not authorize edits. zonkey-policy does not load dictionaries or recognize token syntax. zonkey-token remains the only crate that computes grapheme-safe edits. Context is supplied explicitly by a future adapter; M2 does not inspect applications, windows, processes, secure desktops, or remote sessions.

The dependency direction is acyclic: policy depends on detect, token, and types; detect depends only on types. Dictionary assets are embedded at compile time, so classification performs no filesystem or network access.

## Future M3A observe-only boundary

M3A is a reviewed design milestone, not a runtime implementation. The current
codebase has no text-edit capability beyond returning abstract plans, and no
crate acquires keyboard input or executes those plans. Platform-neutral event
contracts will be introduced only in M3A-01 after the scope in
`docs/m3a-observe-only-scope.md` and ADR 0003 are approved.

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
zonkey-service → zonkey-policy → zonkey-detect → zonkey-types
   ↑
zonkey-cli

zonkey-win → zonkey-types       (placeholder M0, không đổi trong M1)
```

- `zonkey-types`: event, boundary, action, unsupported result và `EditPlan`.
- `zonkey-token`: raw/rendered lifecycle và mọi phép tính grapheme.
- `zonkey-telex`: state machine deterministic; chỉ phụ thuộc token/types.
- `zonkey-detect` và `zonkey-policy`: vẫn là placeholder; M1 không recovery.
- `zonkey-win`: vẫn chỉ validate placeholder plan; không hook/injection/dependency mới.

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
