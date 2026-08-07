# Corpus schema v1

Mỗi dòng `.jsonl` là một JSON object UTF-8 độc lập. Topic files dùng prefix số để
có thứ tự review ổn định. `initial.jsonl` giữ nguyên expectations M0 nhưng đã được
migrate tên field; case M2 chưa chạy được có status `planned`, không bị thay đổi
expected output.

Field bắt buộc:

- `schema_version`: hiện phải bằng `1`.
- `name`: định danh ổn định, không rỗng.
- `raw_keys`: input; `\b` biểu diễn Backspace trong corpus runner.
- `context`: writing, technical-editor, terminal, remote hoặc secure.
- `expected_text`: visible text cuối cùng.
- `decision`: outcome/specification của case.
- `status`: `active`, `planned` hoặc `ignored`.
- `explanation`: lý do case tồn tại; ignored phải chứa `reason:`.

Field tùy chọn:

- `telex_rendered_before_boundary`
- `expected_edit_plan`: `{ "delete_graphemes": N, "insert_text": "..." }`
- `boundary`

`active` phải pass M1 ngay. `planned` là expectation hợp lệ nhưng thuộc milestone
sau. `ignored` chỉ dùng khi có lý do executable rõ ràng. Parser báo path + line
cho JSON/schema lỗi và từ chối unknown fields.

## M2 metadata

Every fixture requires reason. M1 engine fixtures use M1Engine; M2 fixtures use a typed recovery reason name. M2 active fixtures also require telex_rendered_before_boundary. Exact dictionary cases verify it through the real M1 compose function and require it to differ from raw_keys. The policy corpus applies optional edits only to an in-memory string.
