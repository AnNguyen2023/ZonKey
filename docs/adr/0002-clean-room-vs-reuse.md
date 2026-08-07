# ADR 0002: Clean-room thay vì reuse Cay

- Status: Accepted for M0/M1
- Date: 2026-08-07

## Context

Commit Cay được audit (`c0ad81c79875be9856892b882333354fdbf44e52`) có root
`LICENSE` là GPL-3.0. `src/platform/windows/classic` còn chứa nguồn UniKey với
notices GPL và copyright riêng. Zonkey hiện khai báo `MIT OR Apache-2.0`.

## Decision

Không sao chép, dịch, port hoặc liên kết source Cay vào Zonkey. Chỉ dùng hành vi
quan sát được và tài liệu audit làm đầu vào cho specification/test độc lập. Mọi
implementation phải được viết mới bằng Rust từ contract Zonkey.

## Consequences

Nếu sau này muốn reuse Cay, dự án phải quyết định chấp nhận GPL-3.0 cho covered
work, giữ notices, cung cấp Corresponding Source và đáp ứng toàn bộ nghĩa vụ khi
convey binary/source; đồng thời rà soát provenance/license của phần classic và
dependency. Việc đó cần review pháp lý riêng trước khi thay ADR này.

