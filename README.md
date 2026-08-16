# Zonkey

> Zonkey — bộ gõ Telex thông minh cho người làm IT.

Zonkey là dự án Windows-first, offline/local-first. Mục tiêu là khôi phục an toàn
các token tiếng Anh và kỹ thuật bị Telex làm biến dạng, đồng thời bảo vệ tiếng
Việt hợp lệ, mã nguồn, lệnh, URL và dữ liệu nhạy cảm.

Nguyên tắc mặc định là thận trọng: tránh thay thế tự động sai quan trọng hơn việc
bỏ sót một trường hợp có thể sửa. Mọi quyết định tự động trong tương lai phải dựa
trên raw input tách biệt với Unicode đã render và trả về một `EditPlan` trừu tượng.

Trạng thái hiện tại: **M1 core engine in progress; not an end-user IME.** Workspace
có token handling grapheme-safe và nền tảng Telex thuần Rust; chưa có hook bàn
phím, injection, giao diện tray, installer hay English/IT auto-recovery.

Tài liệu chính:

- [`plan.md`](plan.md): kế hoạch triển khai có thẩm quyền.
- [`docs/architecture.md`](docs/architecture.md): ranh giới crate và luồng dự kiến.
- [`docs/cay-audit.md`](docs/cay-audit.md): audit tham chiếu Cay đã ghim commit.
- [`docs/threat-model.md`](docs/threat-model.md): mô hình đe dọa ban đầu.
- [`docs/m1-core-design.md`](docs/m1-core-design.md): contract state machine M1.

Kiểm tra workspace:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## M2 status

The platform-independent M2 detection and policy core is implemented. It uses embedded, version-controlled exact dictionaries, deterministic technical-token protection, and explicit caller-provided context. Recovery returns a typed decision plus an optional abstract edit plan.

This is still not an end-user IME. M2 has no hooks, injection, process inspection, GUI, tray, installer, elevation, network calls, or platform runtime behavior.

## M3D-17 spike

`hosts/vscode-spike/` holds a VS Code cooperating-host adapter spike: contract
mapping, one-transaction compare-and-replace, and idempotent request ids,
validated on a dummy extension harness only. Real VS Code applies fail closed
at `CompositionUnknown` because VS Code cannot prove IME composition state. No
production wiring. See `docs/adr/0022-m3d17-vscode-host-adapter-spike.md`.
