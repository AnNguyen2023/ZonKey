# ADR 0001: Windows-first

- Status: Accepted
- Date: 2026-08-07

## Decision

Zonkey v1 targets Windows 11 x64 only, using the Rust target
`x86_64-pc-windows-msvc`. Windows 32-bit, `i686-pc-windows-msvc`, and
cross-platform adapters are out of scope.

## Consequences

CI chính chạy trên Windows. Prototype input về sau nằm hoàn toàn trong
`zonkey-win`; thiết kế hook + injection phải được đánh giá lại với TSF trước bản
production.
