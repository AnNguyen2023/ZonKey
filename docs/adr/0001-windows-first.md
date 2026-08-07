# ADR 0001: Windows-first

- Status: Accepted
- Date: 2026-08-07

## Decision

Zonkey v1 nhắm Windows 11 x64 trước và Windows 10 là compatibility target. Core
vẫn độc lập nền tảng, nhưng không cam kết adapter macOS, Linux hay Android trong v1.

## Consequences

CI chính chạy trên Windows. Prototype input về sau nằm hoàn toàn trong
`zonkey-win`; thiết kế hook + injection phải được đánh giá lại với TSF trước bản
production.

