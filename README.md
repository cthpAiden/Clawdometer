# Clawdometer

Unofficial Windows desktop HUD for Claude Code usage limits.

*Tiếng Việt bên dưới — [xem bản tiếng Việt](#clawdometer-tiếng-việt).*

> **Unofficial.** Not affiliated with or endorsed by Anthropic.

## What it does

The HUD polls Anthropic's usage endpoint every 60 seconds, so the 5-hour and
7-day rate-limit percentages stay fresh even when no Claude Code session is
running. It shows them in a small always-on-top HUD and a system-tray tooltip
(`5h X% · 7d Y%`).

Additionally, Claude Code sends usage data (percentages, reset times, model,
context window) to your statusline command on every API response. Clawdometer
installs itself as that statusline command and records the latest snapshot to
`~/.clawdometer/state.json`. Whichever source is newer wins — in the HUD and
in `clawdometer status`.

The HUD header shows a countdown to the 5-hour window reset (limits are
account-wide, so a model name would add nothing). The footer shows data age
and turns red with a hint if polling has been failing for over 10 minutes —
the hint distinguishes a network problem ("offline, retrying") from an
expired sign-in ("sign-in expired, open Claude Code").

If you already had a statusline configured, Clawdometer preserves it and
chains it: your original statusline still renders its output (with a 2-second
timeout), and `uninstall` restores it exactly.

## Security: how the app handles your credential

Clawdometer never asks for or stores its own credential. Its live poller
reuses the OAuth access token that Claude Code already keeps in
`~/.claude/.credentials.json`:

- Every 60 seconds it reads that file and shells out to Windows' bundled
  `C:\Windows\System32\curl.exe` (absolute path — immune to PATH planting,
  pinned to HTTPS + TLS 1.2) to make a single read-only
  `GET https://api.anthropic.com/api/oauth/usage` with an
  `Authorization: Bearer` header. The token is passed to curl **over stdin,
  never on the command line**, so it is not visible in the process list.
- If the stored token has expired (or the endpoint rejects it), it makes one
  `POST https://api.anthropic.com/v1/oauth/token` (refresh-token grant using
  Claude Code's public PKCE client id — not a secret) and writes the rotated
  tokens back to `~/.claude/.credentials.json` atomically, so Claude Code's
  own session keeps working. The write is compare-and-swap guarded: if Claude
  Code rotated the tokens itself in the meantime, Clawdometer discards its
  copy instead of clobbering the newer one.
- On repeated failure the poller backs off exponentially (up to 30 minutes
  between attempts) instead of hammering the auth endpoint.

Those are the only two network requests in the entire application. The
statusline hook and CLI are compiled under a `cargo-deny` ban on all HTTP/TLS
crates and are provably network-free, and there is **no telemetry of any
kind**. The token is never logged, never written anywhere except back to
Claude Code's own credentials file, and never sent anywhere except
`api.anthropic.com`. It is never exposed to the HUD webview: the UI receives
only usage percentages and reset times over one-way events, runs under a
strict CSP, has no invokable backend commands, and no filesystem or shell
capabilities.

**Don't trust — verify.** Everything above is checkable in this repository:

- The *only* network code in the entire codebase is
  [`app/src-tauri/src/usage_poller.rs`](app/src-tauri/src/usage_poller.rs)
  (~400 lines including tests). Read it.
- `cargo deny check bans` proves no HTTP/TLS crate is compiled into any
  binary (config in [`deny.toml`](deny.toml)).
- The webview's entire permission set is
  [`app/src-tauri/capabilities/default.json`](app/src-tauri/capabilities/default.json)
  — events and window-dragging, nothing else.
- Release binaries are **not code-signed**. If you don't want to trust a
  downloaded binary, build from source (below) — it takes one command.
  Only download releases from this repository's official Releases page;
  a binary from anywhere else could be a tampered copy.

**Writes:** only `~/.clawdometer/`, the `statusLine` key of
`~/.claude/settings.json` (during `install`/`uninstall`), and the credentials
write-back described above. Exception: the tray's "Start with Windows" toggle
writes the standard HKCU Run registry key, only when you click it.

**Two things worth knowing:**

- `clawdometer install` saves a full backup of your `settings.json` to
  `~/.clawdometer/backups/` before touching it. If your settings contain
  secrets (an `env` block with API keys, an `apiKeyHelper` command), those
  are in the backups too — delete `~/.clawdometer/backups/` when you no
  longer need them, or use `uninstall --purge`.
- Avoid running `install`/`uninstall` while a Claude Code session is actively
  changing settings — both edit `settings.json`, and the last writer wins.

## Requirements

- Windows 10 1803+ (needs the bundled `curl.exe`) or Windows 11.
- Microsoft Edge WebView2 runtime — preinstalled on Windows 11 and updated
  Windows 10. If missing, the installer downloads it, which needs internet.
- Claude Code installed and signed in (the HUD reads its OAuth token from
  `~/.claude/.credentials.json` — or `%CLAUDE_CONFIG_DIR%\.credentials.json`
  if you've set that variable; using Claude Code refreshes it).

## Install

### From GitHub Releases

1. Download the installer (`Clawdometer_<version>_x64-setup.exe`) from the
   [latest release](../../releases/latest).
2. Run it. **Windows SmartScreen will warn you** ("Windows protected your
   PC") because the binary is not code-signed — code-signing certificates
   cost money this hobby project doesn't have. Click *More info* → *Run
   anyway*, but only if you downloaded it from this repository's Releases
   page. If that trust step bothers you (it should!), build from source
   instead — see below.
3. Launch **Clawdometer** from the Start menu. A tray icon appears and the
   HUD window shows up; within a minute it displays live percentages — no
   other setup needed, as long as Claude Code is installed and signed in.

For the optional statusline integration you also need the CLI
(`clawdometer.exe`): download it from the same release (if attached) or
build it from source, then see "Getting started" below.

### From source

Requires Rust (the MSVC toolchain is pinned via `rust-toolchain.toml`) and
[tauri-cli](https://tauri.app):

```
cargo build --release -p clawdometer-cli   # -> target/release/clawdometer.exe (CLI)
cd app/src-tauri && cargo tauri build      # -> HUD app + NSIS installer
```

## Getting started

1. **Run the HUD** (`Clawdometer.exe`). A tray icon appears and the HUD
   window shows up. Within a minute it displays live percentages — no CLI
   step required. Launching it a second time just brings the existing HUD to
   the front (single instance).
2. **Optional — statusline integration:** run `clawdometer install` in a
   terminal. This sets Clawdometer as your Claude Code statusline command, so
   every Claude Code response also updates the HUD instantly and your
   statusline shows `[Model] 5h X% · 7d Y%`.

## HUD usage

- **Move it:** drag the card anywhere; the position is saved once the drag
  settles and remembered across restarts (and sanity-checked against your
  current monitors, so an unplugged display can't strand it off-screen).
- **Tray icon, left-click:** show/hide the HUD.
- **Tray icon, right-click:** menu with *Show/Hide*, *Compact size*,
  *Opacity*, *Start with Windows* (check mark reflects the actual HKCU Run
  key state), and *Quit*.
- **Compact size:** shrinks the card to roughly half width (bars and
  percentages only — no footer or reset times). Also toggled by
  double-clicking the card. Remembered across restarts.
- **Opacity:** 100/85/70/55% — makes the always-on-top card less visually
  blocking. Also available by right-clicking the card. Remembered across
  restarts.
- **Footer:** data age ("as of 1m ago"). If it turns red, the poll is
  failing and the message names the fix: not signed in ("open Claude Code
  and sign in"), an expired sign-in, a missing `curl.exe`, or the network
  ("offline, retrying"). On a first run the same message appears immediately
  in place of "waiting for usage data" (the tray tooltip shows it too), so a
  machine without Claude Code isn't left staring at a blank card.

## CLI

```
clawdometer install      # backs up settings.json, sets/wraps statusLine
clawdometer status       # print the current merged snapshot + capture time
clawdometer uninstall    # restores the original statusLine (or removes the key)
clawdometer uninstall --purge   # also deletes ~/.clawdometer/
```

- `install` writes a timestamped backup of your `settings.json` to
  `~/.clawdometer/backups/` before touching anything, and never overwrites
  an existing backup.
- `install` is idempotent; re-running after moving the binary updates the
  stale path in place.
- If you edited `statusLine` yourself after installing, `uninstall` refuses
  to touch it and tells you where your original is preserved.
- `uninstall --purge`: quit the HUD first — a running HUD's poller recreates
  `~/.clawdometer/` within a minute.
- `--settings <path>` (for `install`/`uninstall`) targets a non-default
  settings.json — mainly for testing.
- **Complete removal:** run `clawdometer uninstall --purge`, then uninstall
  the HUD app from Windows *Apps & features* — its uninstaller also removes
  the "Start with Windows" autostart entry. If you run a portable or
  from-source build (no installer), toggle *Start with Windows* off in the
  tray menu **before** deleting the binary; otherwise the HKCU Run registry
  value it created is left pointing at a deleted exe.

## Files

Everything lives in `~/.clawdometer/`:

| File | Purpose |
|------|---------|
| `state.json` | last statusline snapshot (written by the hook) |
| `live.json` | last poller snapshot (written by the HUD every 60s) |
| `poll_error.json` | why the last poll failed (`auth`/`network`); deleted on success |
| `wrapped.json` | your original statusline command, chained + restored on uninstall |
| `ui.json` | HUD window position, opacity, compact mode |
| `backups/` | timestamped copies of settings.json taken before each install (may contain secrets from your settings — see Security) |

## Building from source

See "Install → From source" above. Additionally:

```
cargo test --workspace     # full test suite
cargo deny check bans      # verify the no-network-crates invariant
```

## Notes

- Percentages have 1% granularity — the same as `/usage` inside Claude Code.
- The HUD footer shows how old the data is ("as of Xm ago"). With live
  polling working it should never say more than a minute.
- Live polling uses Windows' bundled `curl.exe`, which ignores the system
  (WinHTTP/IE) proxy. Behind a corporate proxy, set the `HTTPS_PROXY`
  environment variable (user level) so polls go through it; statusline-hook
  data is unaffected.

## License

MIT

---

# Clawdometer (Tiếng Việt)

HUD không chính thức cho Windows, hiển thị giới hạn sử dụng của Claude Code.

> **Không chính thức.** Không liên kết với và không được Anthropic bảo trợ.

## Ứng dụng làm gì

HUD truy vấn endpoint usage của Anthropic mỗi 60 giây, nên phần trăm giới
hạn 5 giờ và 7 ngày luôn được cập nhật kể cả khi không có phiên Claude Code
nào đang chạy. Số liệu hiển thị trong một cửa sổ HUD nhỏ luôn nổi trên cùng
và trong tooltip ở khay hệ thống (`5h X% · 7d Y%`).

Ngoài ra, Claude Code gửi dữ liệu sử dụng (phần trăm, thời điểm reset, model,
context window) tới lệnh statusline của bạn sau mỗi phản hồi API. Clawdometer
tự cài mình làm lệnh statusline đó và ghi ảnh chụp mới nhất vào
`~/.clawdometer/state.json`. Nguồn nào mới hơn sẽ thắng — cả trong HUD lẫn
trong `clawdometer status`.

Phần đầu HUD hiển thị đếm ngược tới lúc reset cửa sổ 5 giờ. Phần chân hiển
thị tuổi của dữ liệu và chuyển sang màu đỏ kèm gợi ý nếu việc truy vấn thất
bại quá 10 phút — gợi ý phân biệt lỗi mạng ("offline, retrying") với phiên
đăng nhập hết hạn ("sign-in expired, open Claude Code").

Nếu bạn đã có statusline cấu hình sẵn, Clawdometer sẽ giữ nguyên và nối
chuỗi nó: statusline gốc vẫn hiển thị output của mình (với timeout 2 giây),
và `uninstall` khôi phục lại chính xác.

## Bảo mật: ứng dụng xử lý thông tin đăng nhập của bạn thế nào

Clawdometer không bao giờ yêu cầu hay tự lưu trữ thông tin đăng nhập riêng.
Bộ poller tái sử dụng OAuth access token mà Claude Code đã lưu sẵn trong
`~/.claude/.credentials.json`:

- Mỗi 60 giây, nó đọc file đó rồi gọi `C:\Windows\System32\curl.exe` có sẵn
  của Windows (đường dẫn tuyệt đối — miễn nhiễm với chiêu cài binary giả qua
  PATH, ghim HTTPS + TLS 1.2) để thực hiện đúng một request chỉ-đọc
  `GET https://api.anthropic.com/api/oauth/usage` với header
  `Authorization: Bearer`. Token được truyền cho curl **qua stdin, không bao
  giờ qua dòng lệnh**, nên không hiện trong danh sách tiến trình.
- Nếu token đã hết hạn (hoặc bị endpoint từ chối), nó thực hiện một request
  `POST https://api.anthropic.com/v1/oauth/token` (refresh-token grant, dùng
  PKCE client id công khai của Claude Code — không phải bí mật) và ghi token
  mới trở lại `~/.claude/.credentials.json` một cách nguyên tử, để phiên của
  chính Claude Code vẫn hoạt động. Thao tác ghi có kiểm tra compare-and-swap:
  nếu Claude Code đã tự xoay vòng token trong lúc đó, Clawdometer bỏ bản của
  mình thay vì ghi đè lên bản mới hơn.
- Khi thất bại liên tiếp, poller giãn thời gian thử lại theo cấp số nhân
  (tối đa 30 phút giữa các lần) thay vì dồn dập gọi endpoint xác thực.

Đó là hai request mạng duy nhất trong toàn bộ ứng dụng. Hook statusline và
CLI được biên dịch với lệnh cấm (qua `cargo-deny`) mọi crate HTTP/TLS nên
chắc chắn không có khả năng truy cập mạng, và **hoàn toàn không có telemetry
dưới bất kỳ hình thức nào**. Token không bao giờ bị ghi log, không bao giờ
được ghi ra nơi nào khác ngoài chính file credentials của Claude Code, và
không bao giờ được gửi tới đâu khác ngoài `api.anthropic.com`. Token cũng
không bao giờ lộ ra webview của HUD: giao diện chỉ nhận phần trăm sử dụng và
thời điểm reset qua sự kiện một chiều, chạy dưới CSP nghiêm ngặt, không có
lệnh backend nào gọi được từ giao diện, và không có quyền truy cập file hay
shell.

**Đừng tin — hãy kiểm chứng.** Mọi điều ở trên đều kiểm tra được ngay trong
repository này:

- Đoạn code mạng *duy nhất* trong toàn bộ codebase là
  [`app/src-tauri/src/usage_poller.rs`](app/src-tauri/src/usage_poller.rs)
  (~400 dòng tính cả test). Hãy đọc nó.
- `cargo deny check bans` chứng minh không crate HTTP/TLS nào được biên dịch
  vào bất kỳ binary nào (cấu hình trong [`deny.toml`](deny.toml)).
- Toàn bộ quyền của webview nằm trong
  [`app/src-tauri/capabilities/default.json`](app/src-tauri/capabilities/default.json)
  — chỉ sự kiện và kéo cửa sổ, không gì khác.
- Binary phát hành **không được ký số (code-signed)**. Nếu bạn không muốn
  tin một binary tải về, hãy tự biên dịch từ mã nguồn (bên dưới) — chỉ một
  lệnh. Chỉ tải release từ trang Releases chính thức của repository này;
  binary từ bất kỳ nơi nào khác có thể là bản đã bị can thiệp.

**Ghi dữ liệu:** chỉ vào `~/.clawdometer/`, khóa `statusLine` trong
`~/.claude/settings.json` (khi `install`/`uninstall`), và thao tác ghi-lại
credentials mô tả ở trên. Ngoại lệ: nút "Start with Windows" trong menu khay
ghi khóa registry HKCU Run tiêu chuẩn, chỉ khi bạn bấm vào.

**Hai điều nên biết:**

- `clawdometer install` sao lưu toàn bộ `settings.json` vào
  `~/.clawdometer/backups/` trước khi chỉnh sửa. Nếu settings của bạn chứa
  bí mật (khối `env` có API key, lệnh `apiKeyHelper`), chúng cũng nằm trong
  bản sao lưu — hãy xóa `~/.clawdometer/backups/` khi không cần nữa, hoặc
  dùng `uninstall --purge`.
- Tránh chạy `install`/`uninstall` khi một phiên Claude Code đang chủ động
  thay đổi settings — cả hai đều sửa `settings.json`, và bên ghi sau cùng
  sẽ thắng.

## Yêu cầu

- Windows 10 1803+ (cần `curl.exe` đi kèm hệ điều hành) hoặc Windows 11.
- Microsoft Edge WebView2 runtime — có sẵn trên Windows 11 và Windows 10 đã
  cập nhật. Nếu thiếu, trình cài đặt sẽ tải về, việc này cần internet.
- Đã cài và đăng nhập Claude Code (HUD đọc OAuth token từ
  `~/.claude/.credentials.json` — hoặc `%CLAUDE_CONFIG_DIR%\.credentials.json`
  nếu bạn đã đặt biến đó; dùng Claude Code sẽ làm mới token).

## Cài đặt

### Từ GitHub Releases

1. Tải bộ cài (`Clawdometer_<version>_x64-setup.exe`) từ
   [release mới nhất](../../releases/latest).
2. Chạy nó. **Windows SmartScreen sẽ cảnh báo** ("Windows protected your
   PC") vì binary chưa được ký số — chứng chỉ code-signing tốn tiền mà dự án
   sở thích này không có. Bấm *More info* → *Run anyway*, nhưng chỉ khi bạn
   tải từ trang Releases của chính repository này. Nếu bước tin tưởng đó làm
   bạn lấn cấn (và nên thế!), hãy tự biên dịch từ mã nguồn — xem bên dưới.
3. Mở **Clawdometer** từ Start menu. Biểu tượng khay xuất hiện cùng cửa sổ
   HUD; trong vòng một phút nó hiển thị phần trăm trực tiếp — không cần
   thiết lập gì thêm, miễn là Claude Code đã cài và đăng nhập.

Để dùng tích hợp statusline (tùy chọn) bạn cần thêm CLI
(`clawdometer.exe`): tải từ cùng release (nếu có đính kèm) hoặc tự biên
dịch, rồi xem mục "Bắt đầu" bên dưới.

### Từ mã nguồn

Cần Rust (toolchain MSVC, ghim qua `rust-toolchain.toml`) và
[tauri-cli](https://tauri.app):

```
cargo build --release -p clawdometer-cli   # -> target/release/clawdometer.exe (CLI)
cd app/src-tauri && cargo tauri build      # -> ứng dụng HUD + bộ cài NSIS
```

## Bắt đầu

1. **Chạy HUD** (`Clawdometer.exe`). Biểu tượng khay xuất hiện cùng cửa sổ
   HUD. Trong vòng một phút nó hiển thị phần trăm trực tiếp — không cần bước
   CLI nào. Chạy lần thứ hai chỉ đưa HUD hiện có lên trước (một phiên bản
   duy nhất).
2. **Tùy chọn — tích hợp statusline:** chạy `clawdometer install` trong
   terminal. Lệnh này đặt Clawdometer làm lệnh statusline của Claude Code,
   để mỗi phản hồi của Claude Code cũng cập nhật HUD tức thì và statusline
   hiển thị `[Model] 5h X% · 7d Y%`.

## Sử dụng HUD

- **Di chuyển:** kéo thẻ tới bất kỳ đâu; vị trí được lưu khi thao tác kéo
  dừng lại và được nhớ qua các lần khởi động (có kiểm tra với các màn hình
  hiện tại, nên màn hình đã rút không thể làm HUD kẹt ngoài vùng nhìn thấy).
- **Biểu tượng khay, chuột trái:** ẩn/hiện HUD.
- **Biểu tượng khay, chuột phải:** menu gồm *Show/Hide*, *Compact size*,
  *Opacity*, *Start with Windows* (dấu tích phản ánh đúng trạng thái khóa
  HKCU Run hiện tại), và *Quit*.
- **Compact size:** thu thẻ còn khoảng nửa chiều rộng (chỉ thanh và phần
  trăm — không có chân và thời điểm reset). Cũng bật/tắt được bằng cách
  nhấp đúp vào thẻ. Được nhớ qua các lần khởi động.
- **Opacity:** 100/85/70/55% — giúp thẻ luôn-nổi-trên-cùng bớt che khuất.
  Cũng mở được bằng chuột phải vào thẻ. Được nhớ qua các lần khởi động.
- **Chân HUD:** tuổi dữ liệu ("as of 1m ago"). Nếu chuyển đỏ, việc truy vấn
  đã thất bại hơn 10 phút; thông báo cho biết do mạng ("offline, retrying")
  hay do đăng nhập ("sign-in expired, open Claude Code").

## CLI

```
clawdometer install      # sao lưu settings.json, đặt/nối chuỗi statusLine
clawdometer status       # in ảnh chụp gộp hiện tại + thời điểm ghi nhận
clawdometer uninstall    # khôi phục statusLine gốc (hoặc xóa khóa)
clawdometer uninstall --purge   # đồng thời xóa ~/.clawdometer/
```

- `install` ghi bản sao lưu có dấu thời gian của `settings.json` vào
  `~/.clawdometer/backups/` trước khi động vào bất cứ thứ gì, và không bao
  giờ ghi đè bản sao lưu đã có.
- `install` chạy lại vô hại; chạy lại sau khi di chuyển binary sẽ cập nhật
  đường dẫn cũ tại chỗ.
- Nếu bạn tự sửa `statusLine` sau khi cài, `uninstall` sẽ từ chối động vào
  và cho biết bản gốc được lưu ở đâu.
- `uninstall --purge`: hãy thoát HUD trước — poller của HUD đang chạy sẽ tạo
  lại `~/.clawdometer/` trong vòng một phút.
- `--settings <path>` (cho `install`/`uninstall`) nhắm tới settings.json
  không mặc định — chủ yếu để kiểm thử.
- **Gỡ bỏ hoàn toàn:** chạy `clawdometer uninstall --purge`, rồi gỡ app HUD
  trong *Apps & features* của Windows — trình gỡ cài đặt cũng xóa mục tự
  khởi động "Start with Windows". Nếu bạn dùng bản portable hoặc tự build
  (không qua installer), hãy tắt *Start with Windows* trong menu khay
  **trước khi** xóa binary; nếu không, giá trị registry HKCU Run mà nó tạo
  sẽ trỏ tới một exe đã bị xóa.

## Các file

Mọi thứ nằm trong `~/.clawdometer/`:

| File | Mục đích |
|------|----------|
| `state.json` | ảnh chụp statusline mới nhất (do hook ghi) |
| `live.json` | ảnh chụp poller mới nhất (HUD ghi mỗi 60 giây) |
| `poll_error.json` | lý do lần truy vấn cuối thất bại (`auth`/`network`); xóa khi thành công |
| `wrapped.json` | lệnh statusline gốc của bạn, được nối chuỗi + khôi phục khi gỡ |
| `ui.json` | vị trí cửa sổ HUD, độ mờ, chế độ compact |
| `backups/` | bản sao settings.json có dấu thời gian trước mỗi lần cài (có thể chứa bí mật từ settings — xem mục Bảo mật) |

## Biên dịch từ mã nguồn

Xem "Cài đặt → Từ mã nguồn" ở trên. Ngoài ra:

```
cargo test --workspace     # toàn bộ bộ kiểm thử
cargo deny check bans      # kiểm chứng lệnh cấm crate mạng
```

## Ghi chú

- Phần trăm có độ chi tiết 1% — giống `/usage` bên trong Claude Code.
- Chân HUD hiển thị tuổi dữ liệu ("as of Xm ago"). Khi poller hoạt động bình
  thường, con số này không bao giờ quá một phút.
- Poller dùng `curl.exe` đi kèm Windows, vốn bỏ qua proxy hệ thống
  (WinHTTP/IE). Nếu ở sau proxy công ty, hãy đặt biến môi trường
  `HTTPS_PROXY` (mức người dùng) để poller đi qua proxy; dữ liệu từ
  statusline hook không bị ảnh hưởng.

## Giấy phép

MIT
