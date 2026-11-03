# Clawdometer: chi tiết đầy đủ

Bản đầy đủ. Hướng dẫn nhanh xem ở [README](../README.md#clawdometer-tiếng-việt).

## Ứng dụng làm gì

Claude Code gửi dữ liệu sử dụng (phần trăm giới hạn, thời điểm reset, model,
context window) tới lệnh statusline của bạn sau mỗi phản hồi API. Clawdometer
tự cài mình làm lệnh statusline đó (tự động ở lần chạy HUD đầu tiên nếu bạn
chưa cấu hình statusline nào), ghi ảnh chụp mới nhất vào
`~/.clawdometer/state.json`, và hiển thị phần trăm 5 giờ / 7 ngày trong một
cửa sổ HUD nhỏ luôn nổi trên cùng cùng tooltip ở khay hệ thống
(`5h X% · 7d Y%`).

Khi ảnh chụp bị cũ (bạn đang dùng claude.ai web/mobile hoặc desktop app thay
vì phiên terminal có statusline), HUD tự làm mới bằng cách chạy **CLI chính
thức ở chế độ headless** — `claude -p /usage` — và parse báo cáo mà chính
Claude Code in ra (khoảng mỗi phút một lần khi không có dữ liệu mới hơn,
hoặc theo yêu cầu qua menu khay → *Refresh usage*).

Đó là hai nguồn dữ liệu duy nhất, và cả hai đều là bề mặt chính thức của
Claude Code — Clawdometer **không tự thực hiện request mạng nào** và không
bao giờ động vào thông tin đăng nhập của bạn. (Các phiên bản trước truy vấn
endpoint usage của Anthropic bằng OAuth token của Claude Code; tính năng đó
đã bị gỡ bỏ: Điều khoản Dịch vụ Người dùng của Anthropic cấm dùng OAuth
token của tài khoản Claude trong công cụ bên thứ ba, và app này không được
phép đặt tài khoản của bạn vào rủi ro.)

Phần đầu HUD hiển thị đếm ngược tới lúc reset cửa sổ 5 giờ. Khi thời điểm
reset của một cửa sổ trôi qua trong lúc máy rảnh, HUD tự suy ra con số trung
thực 0% ngay tại chỗ — không cần mạng. (Mức sử dụng phát sinh trên claude.ai
web/mobile chỉ hiện ra ở phản hồi Claude Code kế tiếp.)

Nếu bạn đã có statusline cấu hình sẵn, HUD sẽ không động vào — hãy chạy
`clawdometer install` (CLI) để nối chuỗi: statusline gốc vẫn hiển thị output
của mình (với timeout 2 giây), và `uninstall` khôi phục lại chính xác.

## Bảo mật

Clawdometer không bao giờ yêu cầu, đọc, hay lưu trữ bất kỳ thông tin đăng
nhập nào, và thực hiện **zero request mạng dưới mọi hình thức** — mọi binary
(hook, CLI, HUD) đều được biên dịch với lệnh cấm (qua `cargo-deny`) toàn bộ
crate HTTP/TLS, nên điều này chứng minh được từ chính bản build, không phải
lời hứa. **Hoàn toàn không có telemetry.** Dữ liệu sử dụng chỉ đến qua các
bề mặt của chính Claude Code: hook statusline (Claude Code chạy hook và
truyền cho nó một ảnh chụp JSON), và lần chạy `claude -p /usage` headless
định kỳ mà HUD parse output dạng text — trong cả hai trường hợp, chính
Claude Code lấy số liệu bằng đăng nhập của nó, và tiến trình duy nhất
Clawdometer khởi chạy cho việc này là binary `claude` chính thức. Webview
chỉ nhận phần trăm sử dụng và thời điểm reset qua sự kiện một chiều,
chạy dưới CSP nghiêm ngặt, không có lệnh backend nào gọi được từ giao diện,
và không có quyền truy cập file hay shell. (Webview cũng nhận một biến boolean
`working` cho hiệu ứng hoạt động, và — chỉ khi một [giao diện](../README.md#giao-diện-skins)
Audiowave Orb đang được chọn — một luồng 36 giá trị độ lớn phổ âm thanh cho
vòng orb. Backend suy ra cả hai tại chỗ; không cái nào mang dữ liệu sử dụng, và
các dải phổ là chiều cao thanh chứ không phải âm thanh: không có gì được ghi
lại hay lưu trữ.)

Cho cờ hoạt động đó, HUD còn **đọc** (không bao giờ ghi) các file transcript
của Claude Code trong `~/.claude/projects/`, để biết khi nào một phiên đang tạo
phản hồi. Nó chỉ xem trạng thái lượt của tin nhắn cuối tại chỗ — không mạng,
không gì rời khỏi tiến trình.

Thiết kế này là chủ đích: Điều khoản Dịch vụ Người dùng của Anthropic cấm
dùng OAuth token của tài khoản Claude Free/Pro/Max trong bất kỳ công cụ bên
thứ ba nào, nên Clawdometer chỉ tiêu thụ những gì Claude Code tự trao cho
giao diện statusline được tài liệu hóa của nó — không giả danh gì, không
request thêm gì.

**Đừng tin — hãy kiểm chứng.** Mọi điều ở trên đều kiểm tra được ngay trong
repository này:

- `cargo deny check bans` chứng minh không crate HTTP/TLS nào được biên dịch
  vào bất kỳ binary nào (cấu hình trong [`deny.toml`](../deny.toml)) — và mã
  nguồn vốn không chứa dòng code mạng nào (`grep` thử: không socket, không
  curl, không URL).
- Toàn bộ quyền của webview nằm trong
  [`app/src-tauri/capabilities/default.json`](../app/src-tauri/capabilities/default.json)
  — chỉ sự kiện và kéo cửa sổ, không gì khác.
- Binary phát hành **không được ký số (code-signed)**. Nếu bạn không muốn
  tin một binary tải về, hãy tự biên dịch từ mã nguồn (bên dưới) — chỉ một
  lệnh. Chỉ tải release từ trang Releases chính thức của repository này;
  binary từ bất kỳ nơi nào khác có thể là bản đã bị can thiệp.

**Ghi dữ liệu:** chỉ vào `~/.clawdometer/` và khóa `statusLine` trong
`~/.claude/settings.json` (khi `install`/`uninstall`, hoặc khi HUD tự cài
một-lần-duy-nhất lúc chưa có statusline nào — cả hai đều sao lưu toàn bộ
settings trước). Ngoại lệ: nút "Start with Windows" trong menu khay ghi khóa
registry HKCU Run tiêu chuẩn, chỉ khi bạn bấm vào.

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

- Windows 10 hoặc 11.
- Microsoft Edge WebView2 runtime — có sẵn trên Windows 11 và Windows 10 đã
  cập nhật. Nếu thiếu, trình cài đặt sẽ tải về, việc này cần internet.
- Đã cài và đăng nhập Claude Code (CLI) — toàn bộ dữ liệu đến từ hook statusline
  của nó, nên HUD chỉ cập nhật khi Claude Code đang được dùng. (Clawdometer
  tôn trọng `%CLAUDE_CONFIG_DIR%` nếu bạn đã di dời `~/.claude`.)

## Bắt đầu

1. **Chạy HUD** (`Clawdometer.exe`). Biểu tượng khay xuất hiện cùng cửa sổ
   HUD. Nếu Claude Code chưa có statusline, HUD tự nhận vị trí đó (một lần
   duy nhất, sao lưu settings trước) — không cần bước CLI nào. Dùng Claude
   Code là phần trăm hiện ra theo từng phản hồi, statusline hiển thị
   `[Model] 5h X% · 7d Y%`. Chạy HUD lần thứ hai chỉ đưa HUD hiện có lên
   trước (một phiên bản duy nhất).
2. **Chỉ khi bạn đã có statusline riêng:** chạy `clawdometer install` trong
   terminal. Lệnh này nối chuỗi statusline gốc của bạn (output của nó vẫn
   hiển thị) đồng thời cấp dữ liệu cho HUD.

## Sử dụng HUD

- **Di chuyển:** kéo thẻ tới bất kỳ đâu; vị trí được lưu khi thao tác kéo
  dừng lại và được nhớ qua các lần khởi động (có kiểm tra với các màn hình
  hiện tại, nên màn hình đã rút không thể làm HUD kẹt ngoài vùng nhìn thấy).
- **Biểu tượng khay, chuột trái:** ẩn/hiện HUD.
- **Biểu tượng khay, chuột phải:** menu gồm *Show/Hide*, *Refresh usage*
  (chạy `claude /usage` headless ngay lập tức), *RICE*, *Compact size*,
  *Fable bar*, *Opacity*, *Start with Windows* (dấu tích phản ánh đúng trạng
  thái khóa HKCU Run hiện tại), và *Quit*.
- **RICE:** chọn giao diện — *Classic*, *Rowline*, *Bento Box*, *ClawdBoy* →
  *Table* / *Menu* / *Quad*, hoặc *Audiowave
  Orb* → *Bars* / *Peak hold* / *LED Bloom* (xem [Giao diện](../README.md#giao-diện-skins)). Cả chín là một
  nhóm radio, nên luôn chỉ đúng một cái được tích. Đổi giao diện sẽ đổi kích
  thước HUD (orb là ô vuông 160×160; mỗi giao diện thẻ có kích thước riêng) và
  được nhớ qua các lần khởi động. Chọn giao diện orb sẽ bật luồng thu loopback âm
  thanh hệ thống; quay lại một giao diện thẻ sẽ tắt nó.
- **Compact size:** thu thẻ còn khoảng nửa chiều rộng (vẫn các con số đó, với
  nhãn hàng 5H / 7D / FBL trên Rowline). Cũng bật/tắt được bằng cách
  nhấp đúp vào thẻ. Được nhớ qua các lần khởi động.
- **Fable bar:** mặc định bật. Cửa sổ tuần Fable là giới hạn riêng của gói Max,
  nên trên các gói khác hàng thứ ba đó không bao giờ có dữ liệu — tắt nó đi thì
  mọi giao diện thẻ tự sắp lại quanh hai cửa sổ: các thẻ dạng hàng ngắn lại,
  Bento cho Weekly chiếm luôn ô vừa trống, còn ClawdBoy giữ nguyên vỏ máy và
  dùng chỗ trống đó để phóng to chữ. Được nhớ qua các lần khởi động.
- **Opacity:** 100/85/70/55% — giúp thẻ luôn-nổi-trên-cùng bớt che khuất.
  Cũng mở được bằng chuột phải vào thẻ. Được nhớ qua các lần khởi động.
- **Hiệu ứng hoạt động:** khi bất kỳ phiên Claude Code nào đang tạo phản hồi,
  linh vật clawd chạy một hiệu ứng "đang làm việc" — mỗi lượt bốc ngẫu nhiên một
  động tác từ một tập hợp (nhún nhảy kiểu thợ, nhảy cóc, giậm bước, vỗ tay, rung
  như thạch, và nhiều nữa), rút không lặp nên hiếm khi làm lại y hệt hai lần liền.
  Nó nghỉ khi mọi thứ rảnh. Khác với các con số sử dụng, hiệu ứng này không phụ
  thuộc hook statusline (nên chạy được cả trên client GUI không gọi hook): HUD đọc
  trạng thái lượt (turn) từ các file transcript của Claude Code trong
  `~/.claude/projects/` — tại chỗ, không mạng — và vẫn sáng suốt các quãng "suy
  nghĩ" dài, dừng ngay khi một lượt kết thúc. Tôn trọng `prefers-reduced-motion`
  (linh vật đứng yên, không chuyển động).
- **Trạng thái trống:** trước khi có dữ liệu lần đầu, phần trăm hiển thị "—"
  và tooltip khay ghi "waiting for first usage refresh" — trình làm mới
  headless sẽ lấy số liệu trong khoảng một phút sau khi mở. Khi thời điểm
  reset của một cửa sổ giới hạn trôi qua, HUD tự hiển thị 0% cho nó (suy ra
  tại chỗ, không cần mạng).

## CLI

```
clawdometer install      # sao lưu settings.json, đặt/nối chuỗi statusLine
clawdometer status       # in ảnh chụp mới nhất + thời điểm ghi nhận
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
| `live.json` | ảnh chụp refresh mới nhất (HUD ghi sau mỗi lần chạy `claude /usage` headless) |
| `wrapped.json` | lệnh statusline gốc của bạn, được nối chuỗi + khôi phục khi gỡ |
| `ui.json` | vị trí cửa sổ HUD, độ mờ, chế độ compact |
| `statusline-autoinstall.done` | đánh dấu: HUD đã đề nghị nhận khóa statusLine trống một lần (nên gỡ ra sau đó sẽ được tôn trọng) |
| `backups/` | bản sao settings.json có dấu thời gian trước mỗi lần cài (có thể chứa bí mật từ settings — xem mục Bảo mật) |

## Biên dịch từ mã nguồn

Xem mục "Cài đặt" trong [README](../README.md#cài-đặt). Ngoài ra:

```
cargo test --workspace     # toàn bộ bộ kiểm thử
cargo deny check bans      # kiểm chứng lệnh cấm crate mạng
```

## Ghi chú

- Phần trăm có độ chi tiết 1% — giống `/usage` bên trong Claude Code.
- Dữ liệu cập nhật theo từng phản hồi của Claude Code; khi im ắng, HUD tự
  làm mới qua `claude /usage` headless khoảng mỗi phút một lần, nên mức sử
  dụng trên claude.ai web/mobile vẫn hiện ra. HUD cũng tự đưa một cửa sổ về 0%
  khi thời điểm reset của nó trôi qua.
- Các binary của Clawdometer hoàn toàn offline: proxy, VPN, firewall chỉ
  liên quan tới chính Claude Code.
