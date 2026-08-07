# M1 core design

## State machine

`TelexEngine` là value do caller sở hữu, không có mutable global. Trạng thái duy
nhất là `TokenState { raw_ascii, rendered }`. Mỗi `EngineEvent` tạo đúng một
`EngineAction`: `Noop`, `Apply(EditPlan)`, `Commit(TokenBoundary)` hoặc `Reset`.

M1 recompute rendered output từ toàn bộ raw token sau mỗi character/backspace.
Cách này ưu tiên tính deterministic và khả năng kiểm chứng hơn incremental cache;
optimization chỉ được thêm khi golden corpus chứng minh hành vi không đổi.

## Invariants raw/rendered

1. `raw_ascii` chỉ chứa ASCII đã được caller nhập và không được suy ra từ Unicode.
2. `rendered` luôn là output deterministic của raw token theo rule M1.
3. Backspace xóa một raw key rồi recompose; không đoán ngược từ rendered text.
4. Commit/reset xóa đồng thời raw và rendered state.
5. Direct non-ASCII input là typed unsupported; Unicode do Telex tạo là hợp lệ.

## Grapheme safety contract

Mọi visible deletion dùng extended grapheme cluster từ `unicode-segmentation`.
`edit_plan_between` tìm common grapheme prefix, xóa suffix bằng số grapheme và
chèn suffix Unicode mới. `delete_last_graphemes` rebuild string từ grapheme slices,
không xóa bằng byte offset hoặc UTF-16 code unit. Tests bao phủ ASCII, tiếng Việt
precomposed, combining mark, emoji/ZWJ và mixed Unicode.

## Boundary semantics

- Space, Enter, Tab và ASCII punctuation: commit token; boundary tự nó không nằm
  trong `EditPlan` và platform/corpus host chuyển tiếp boundary character.
- CursorMove, FocusLoss và Unknown: reset vì caret/context không còn đáng tin.
- Backspace là event riêng, không phải token boundary.

Punctuation được chọn là boundary rõ ràng cho Zonkey M1, khác hành vi ngầm nối
buffer đã ghi nhận trong audit tham chiếu. Lựa chọn này cần được corpus review.

## Telex behavior hiện có

- Shape: `dd`, `aa`, `aw`, `ee`, `oo`, `ow`, `uw`.
- Tone precomposed: `s`, `f`, `r`, `x`, `j` cho 12 vowel shapes.
- Undo typed rõ ràng cho repeated shape/tone key.
- Tone placement M1 chọn vowel cuối; corpus active chỉ chứa trường hợp mà rule này
  đã được chấp nhận. Đây chưa phải engine Telex hoàn chỉnh.

## Deferred intentionally

M2: English/IT dictionaries, scoring, auto-recovery, never-transform pattern,
context policy và ambiguity handling.

M3: Windows event acquisition/execution, hook hoặc TSF evaluation, injection,
foreground/app/security context, tray/UI và installer.

Các gap Telex còn lại trước khi tuyên bố M1 hoàn chỉnh: tone placement cho nucleus
nhiều vowel, `qu`/`gi`, free-order modifiers, uppercase/case preservation, `z`,
selection/caret snapshot và review punctuation semantics.

