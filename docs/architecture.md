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
identity, or window handles. `zonkey-win` remains the only future crate that
may depend on Windows APIs, with the intended direction `zonkey-win ->
zonkey-types`.

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

The next roadmap boundary is M3A-04, a separately approved Windows
observe-source spike. Its scope is limited to mapping a platform event source
to existing validated contracts; suppression, injection, replay, text editing,
and foreground inspection remain out of scope.

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
- `zonkey-detect` và `zonkey-policy`: M2 implementation đã hoàn tất; vẫn
  platform-neutral và không thực thi edit.
- `zonkey-win`: vẫn chỉ validate placeholder plan; chưa có hook/injection.
- `zonkey-cli`: vẫn là stub, chưa phải runtime observer hay replay command.

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
