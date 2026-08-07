# Threat model ban đầu


## M2 controls

- Exact dictionary matches are considered only when Telex actually changed the token; approximate and statistical matching are out of scope.
- Structured technical tokens are protected before dictionary lookup.
- Terminal, secure, remote, and unknown contexts veto automatic recovery.
- Technical editors require technical, product, or explicit user evidence at the higher threshold.
- Unknown lexical evidence becomes Ambiguous and never silently edits text.
- Core results are data only. M2 cannot execute edits and does not collect or transmit input.
- Dictionary poisoning could turn an unsafe token into exact evidence. Built-in
  lists are small, reviewed, version controlled, and backed by real-composition
  corpus cases; future user dictionaries must be validated by their adapter.
- Unsafe automatic replacement is contained by pattern-first blocking, explicit
  context vetoes, constrained confidence, and an abstract non-executing result.
## Tài sản cần bảo vệ

- Nội dung người dùng, đặc biệt mật khẩu, token truy cập, lệnh và mã nguồn.
- Tính toàn vẹn của văn bản và trạng thái phím/modifier.
- Quyền người dùng và ranh giới integrity level của Windows.
- Cấu hình/dictionary cục bộ.

## Mối đe dọa chính

- Hook vô tình biến thành keylogger qua log, crash dump hoặc telemetry.
- Event được inject quay lại hook, gây vòng lặp hoặc nhân đôi nội dung.
- Xóa sai số lượng Unicode do đếm byte/code unit thay vì grapheme.
- Race giữa xóa/chèn và ứng dụng đích, autocomplete hoặc focus/caret thay đổi.
- Tự sửa sai token Việt, code, URL, đường dẫn, lệnh hoặc trường mật khẩu.
- Injection thất bại một phần làm mất chữ hoặc kẹt modifier.
- Tiến trình thấp quyền cố thao tác cửa sổ elevated/secure desktop.
- Cấu hình hỏng hoặc ghi không atomic làm thay đổi policy an toàn.

## Kiểm soát bắt buộc trước prototype Windows

- Secure/terminal/remote deny-by-default và per-app policy.
- Marker cho synthetic events, guard re-entrancy, timeout và fail-closed.
- Transaction có kiểm tra kết quả từng batch; luôn phát đủ key-up.
- Grapheme-aware edit planning và corpus Unicode.
- Không lưu raw input nếu chưa opt-in rõ ràng; diagnostics dùng reason/hash/length.
- Không tự elevate; phát hiện integrity mismatch và không hook/inject.
- Config versioned, validated, atomic-save và có backup.

M0 chưa có hook, injector, logging hay network code nên chưa tạo bề mặt runtime đó.
