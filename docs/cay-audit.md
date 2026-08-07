# Audit Cay tại commit `c0ad81c79875be9856892b882333354fdbf44e52`

Audit ngày 2026-08-07 từ clone `https://github.com/tctvn/cay.git`, branch `main`.
Mọi path dưới đây tương đối với `references/cay/`. Clone chỉ đọc và không thuộc
production source của Zonkey.

## 1. Inventory source tree

`git ls-files` tại commit đã ghim cho thấy các nhóm sau:

- Root (6 file): `.cursorrules`, `.gitignore`, `CMakeLists.txt`, `INSTALL.md`,
  `LICENSE`, `README.md`; build dùng CMake/C++17.
- `.github/workflows/` (2): `nightly.yml`, `release.yml`.
- `scripts/` (10): install/uninstall cho Windows minimal/full/ARM/classic,
  macOS và Fcitx5, cộng `install-local-test.sh`.
- `src/core/` (5): `CayData.cpp`, `CayData.h`, `CayEngine.cpp`, `CayEngine.h`,
  `CayTypes.h`.
- `src/platform/windows/shared/` (4): `KeyboardHookManager.cpp/.h`,
  `InputInjector.cpp/.h`.
- `src/platform/windows/minimal/` (2): `main.cpp`, `no_crt.cpp`.
- `src/platform/windows/full/` (5): `main_full.cpp`, `ConfigManager.cpp/.h`,
  `cayy.rc`, `resource.h`.
- `src/platform/windows/classic/` (3 file trực tiếp): `gpl.txt`, `license.txt`, `readme.txt`,
  `byteio/` (2 file), `newkey/` (60 tracked file gồm C++/headers/resource và
  assets), `vnconv/` (27 tracked file gồm converter, miniz và build artifacts).
- `src/platform/macos/` (7): AppDelegate, event-tap hook, injector và plist.
- `src/platform/fcitx5/` (8): addon factory/engine, CMake, config templates,
  README và SVG.
- `src/platform/android/` (236 tracked file): Gradle wrapper/project, Kotlin IME,
  JNI/CMake bridge, resources; đáng chú ý upstream commit cả `.gradle/` cache và
  `app/.cxx/` build output cho bốn ABI.

Đây là inventory đầy đủ theo subtree và số file tracked. Audit hành vi tập trung
vào các file được yêu cầu; phần classic chỉ được kiểm tra provenance/license.

## 2. Core và platform architecture

`src/core/CayEngine.h/.cpp`, `CayData.h/.cpp`, `CayTypes.h` tạo core C++ không gọi
Win32. `TelexEngine` nhận `KeyEvent`, duy trì buffer cố định `MAX_BUFFER = 64` và
phát callback `InjectTextFunc(backspaceCount, newText, newTextLen)`. Platform map
native key events sang `KeyCode` rồi cung cấp callback injection.

Windows minimal/full cùng dùng `src/platform/windows/shared/`; macOS dùng
CGEvent tap + CGEventPost; Fcitx5 bọc core qua addon API; Android JNI build lại
`CayEngine.cpp`/`CayData.cpp` và Kotlin IME làm adapter. Kiến trúc vì vậy tái dùng
core, nhưng contract xóa dùng số `wchar_t`/ký tự hiển thị chứ không định nghĩa
grapheme cluster.

## 3. Lifecycle xử lý Telex

Theo `src/core/CayEngine.cpp`:

1. `OnKeyDown` reset khi navigation/control làm desync, xử lý Backspace và boundary.
2. `ReplayKey` ghi raw key vào `_buffer`, gọi `ShouldBypassWord`, rồi thử theo thứ
   tự double key, hook key, tone key; nếu không consume thì append raw character.
3. `ApplyDoubleKeys` backward-scan cho `aa/ee/oo/dd` (và mapping VNI/VIQR), có
   toggle/undo khi lặp modifier.
4. `ApplyHookKeys` backward-scan cho `w`, gồm các nhánh `ă/ơ/ư`, `ươ`, `ưa`, `ưu`
   và ngoại lệ `qu`/một số cụm `uo`.
5. `ApplyToneMarks` chọn vị trí qua `FindTonePosition`, đổi/toggle tone, xử lý `z`
   và auto-hook một số `uo`.
6. `UpdateScreen` tìm common prefix giữa `_lastOutput` và output mới rồi callback
   số backspace + suffix cần chèn.

`src/core/CayData.cpp` chứa bảng tone cho 12 nhóm nguyên âm, mapping
f/s/r/x/j/z, strip tone/accent, vowel check, hook rule và bảng initial/nucleus.
Ngoài bảng này, `CayEngine.cpp` còn có validator âm tiết riêng (`IsCompleteSyllable`).

## 4. Raw, rendered, replay, boundary, Space, punctuation và Backspace

- Raw được giữ ở `_buffer[i].raw`; rendered Unicode ở `_text`; `_lastOutput` là
  snapshot đã inject. Trường `_buffer[i].output` được khởi tạo bằng raw trong
  `ReplayKey`, nhưng source đã audit không cập nhật nó sau transform.
- `FallbackToRaw` dựng lại chuỗi trực tiếp từ `_buffer[].raw`; recovery không
  decompose Unicode để tạo output cuối.
- Enter, Tab và Space chạy macro/Auto Restore, gọi `CommitWord` (save state rồi
  reset). Space không bị suppress nên ứng dụng đích tự nhận Space.
- Punctuation/digit printable không phải boundary commit: code chạy macro check
  nếu có token, sau đó fall through và đưa ký tự vào buffer để giữ mapping
  Backspace. Vì vậy token có thể tiếp tục qua punctuation.
- Escape/navigation/Delete và modifier hệ thống reset state. Mouse button down
  reset engine qua hook Windows.
- Backspace khi buffer đang hoạt động bị suppress. Engine dry-run/replay raw keys,
  tìm key tạo hoặc sửa visual character cuối, loại các key đó, replay phần còn lại
  rồi diff màn hình. Đây là dependency heuristic, không phải grapheme algorithm.
- Backspace ngay sau một committed boundary khôi phục saved state và để OS tự xóa
  boundary character; `_canRestore` sau đó bị tắt. Với Auto Restore, state được
  save lại bởi `CommitWord` sau `FallbackToRaw`.

## 5. Smart Bypass và Auto Restore

`ShouldBypassWord` trong `src/core/CayEngine.cpp` chạy trên tối đa 15 raw key đã
lowercase. Nó bypass khi gặp một số dấu hiệu không giống một âm tiết Việt: initial
không hợp lệ (`w/f/j/z` đầu từ, `q` không đi với `u`, `p` không đi với `h`, cluster
phụ âm lạ), nhiều âm tiết kiểu vowel-consonant-vowel, validator âm tiết thất bại,
hoặc tone huyền/hỏi/ngã với final `c/ch/p/t`. Nếu rendered text đã có Vietnamese
mark, phần lớn rule bypass phía sau không chạy.

Auto Restore chỉ chạy ở Enter/Tab/Space và chỉ khi rendered token có Vietnamese
mark. Engine strip tone (nhưng giữ hình dạng accent cho validator), lọc ký tự,
yêu cầu `!IsCompleteSyllable`, rồi gọi `IsLikelyEnglishWord`. Nếu đạt, nó thay
rendered text bằng toàn bộ raw buffer qua `FallbackToRaw`.

Không có confidence score, dictionary tiếng Anh/IT, user dictionary hay context
profile trong core đã audit.

## 6. English/technical-token detection

`IsLikelyEnglishWord` là heuristic: dài ít nhất 2; chỉ lowercase `a-z`; có ít nhất
một vowel; không quá bốn consonant liên tiếp; loại double `q/h/j/k/x/v/w/y`.
Smart Bypass có structural rules nêu trên. README quảng bá camelCase/snake_case,
nhưng source core không có parser được đặt tên cho camelCase, snake_case, URL,
path, IP, FQDN, e-mail hay PowerShell parameter. Một số identifier có thể được
bypass gián tiếp bởi uppercase/raw structure hoặc punctuation, nhưng không có
never-transform contract đã xác minh.

## 7. Windows hook, injection và loop prevention

`src/platform/windows/shared/KeyboardHookManager.cpp` cài `WH_KEYBOARD_LL` và
`WH_MOUSE_LL` toàn desktop thread (`threadId = 0`) bằng singleton static. Keyboard
hook tự map A-Z theo Shift XOR Caps Lock và digit không Shift; các layout/ký tự
khác chỉ có `character = 0`. Bitmask pressed-state được cập nhật nhưng source đã
audit không dùng `TestKeyBit` để debounce.

`InputInjector::ReplaceText` tạo một batch `INPUT`: khi cần xóa, chèn dummy `~`,
xóa dummy + old text bằng VK_BACK, rồi chèn UTF-16 bằng `KEYEVENTF_UNICODE`.
Mỗi synthetic event có `dwExtraInfo = 0x1234`; hook bỏ qua marker này. Minimal và
full host còn kiểm tra marker lần nữa. Batch gửi bằng một `SendInput` call.

Rủi ro đã xác minh:

- Comment nói `MAX_INPUTS 321`, nhưng implementation dùng `INPUT inputs[256]` và
  silently dừng vòng xóa/chèn khi hết chỗ.
- Không kiểm tra return value của `SendInput`, không retry/rollback/timeout.
- Marker 0x1234 cố định và dễ collision; source không kiểm tra `LLKHF_INJECTED`.
- Không có explicit re-entrancy lock; loop prevention phụ thuộc marker.
- Không có kiểm chứng foreground/caret vẫn giữ nguyên trong lúc batch chạy.
- Cách đếm UTF-16/backspace không bảo đảm grapheme-aware cho emoji/combining text.

## 8. Windows edge cases

- Chromium/Excel: injector có workaround dummy printable `~` để phá autocomplete
  trước replace; comment nêu Chrome/Excel. Không có automated test hoặc kết quả
  API để chứng minh độ đúng.
- UWP, Office nói chung, terminal và RDP: không có nhánh nhận diện/adapter/policy
  riêng trong các file Windows đã audit.
- Elevated app/secure desktop: không có integrity-level check hay auto-elevation.
  Source không xử lý rõ thất bại do UIPI và cũng không kiểm tra `SendInput` result.
- Full host exclude theo executable basename (case-insensitive), lấy bằng
  `QueryFullProcessImageNameW`; không match full path, class hay title. Nếu không
  mở được process thì không đổi trạng thái.
- Foreground change dùng `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, ...,
  WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)` và reset khi bật/tắt theo list.
- Ctrl/Alt/Win reset engine; mouse click reset. Không có handler riêng cho focus
  loss ngoài foreground event của bản full; bản minimal không có foreground hook.

## 9. Build, release, packaging và CI

`CMakeLists.txt` yêu cầu CMake 3.20/C++17. Windows tạo `cay` minimal, `cayy` full
và `cay-classic`; minimal/full dùng MSVC flags tối ưu size, static runtime và các
Win32 libraries. Minimal khai báo no-CRT startup và `no_crt.cpp` cấp new/delete,
memset/memcpy; tuy vậy CMake vẫn link `libcmt`, `libvcruntime`, `libucrt`. macOS
tạo universal app (deployment 11.0); Fcitx5 là option; Android có Gradle/JNI.

`.github/workflows/release.yml` chạy khi push tag `v*` hoặc `cay`, build Windows
x64/ARM64, macOS universal và Android debug APK rồi upload GitHub Release.
`.github/workflows/nightly.yml` chạy mỗi push `main`, build Windows classic và
Fcitx5, force-update tag/release `nightly`. Không thấy test job, static analysis,
artifact signing, checksum/SBOM hay release provenance trong hai workflow.

Windows install scripts tải binary release vào `%LOCALAPPDATA%\CayIME` và tạo
HKCU Run startup; uninstall dừng process, xóa install directory và Run entries.
Các lệnh README/INSTALL dùng `irm ... | iex` hoặc curl/wget pipe shell. Không có
checksum/signature verification trong các path script đã audit.

## 10. License và reuse recommendation

Root `LICENSE` là nguyên văn GNU GPL version 3; README cũng ghi GPL-3.0. Thư mục
`src/platform/windows/classic` có file GPL và nhiều header ghi UniKey copyright
1998-2002 Pham Kim Long, GPL version 2 or later. Đây không phải kết luận pháp lý,
nhưng copy/port/link source vào một workspace dự kiến MIT/Apache sẽ tạo nghĩa vụ
copyleft không tương thích với ý định license hiện tại.

Khuyến nghị: **clean-room behavioral reimplementation**. Chỉ dùng facts hành vi
trong tài liệu/test do Zonkey tự viết; không copy expression, bảng, comment hoặc
code Cay. Nếu chọn reuse sau này, tối thiểu phải cấp phép toàn covered work tương
thích GPL-3.0 khi phân phối, giữ notices, đánh dấu sửa đổi, cung cấp license và
Corresponding Source/build scripts; provenance classic/dependency cần review riêng.

## 11. Behavioral test checklist cho Zonkey

Zonkey phải reproduce, improve hoặc intentionally reject từng nhóm:

- Telex cơ bản: `aa/ee/oo/dd`, `aw/ow/uw`, f/s/r/x/j/z, uppercase và toggle lặp.
- Tone placement: một/hai/ba vowel, final consonant, `qu`, `gi`, `iê/uô/ươ/uyê`.
- `uo/ươ/uơ/ưa/ưu` và các ngoại lệ được ghi nhận từ Cay.
- Raw/rendered divergence và exact recovery từ raw, không decompose rendered.
- Space/Enter/Tab boundary; quyết định rõ punctuation có commit hay không (Zonkey
  nên commit theo token rules rõ ràng thay vì ngầm nối buffer như Cay).
- Backspace sau compose, sau modifier, sau restore và ngay sau boundary.
- Navigation, selection, mouse, focus/caret/process change làm invalid state.
- Buffer overflow và token dài không mất/phá text.
- English positives `resume`, `pull`, `refresh`; Vietnamese negatives `dùng`,
  `cài đặt`; ambiguity luôn fail-safe.
- camelCase, PascalCase, snake_case, kebab-case được bypass theo contract.
- URL, Windows path, IP, FQDN, e-mail, semantic version, UUID/hash và PowerShell
  parameter luôn never-transform.
- Unicode grapheme: combining marks, surrogate pairs, emoji/ZWJ; delete theo
  grapheme thay vì UTF-16 unit.
- Synthetic-event marker, partial `SendInput`, zero return, cap batch, focus race,
  re-entrancy và modifier key-up failsafe.
- Notepad/UWP, Chromium contenteditable, Office, VS Code, terminal, RDP và elevated
  foreground; unsupported contexts phải disable/fail-closed.
- Per-app matching cần identity/path rõ ràng, không chỉ basename dễ collision.
- Config malformed/migration/atomic save; privacy diagnostics không chứa raw text.
- Build/release phải có fmt, Clippy, tests; về sau thêm signing, checksums/SBOM và
  installer verification trước public release.
