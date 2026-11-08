# Haze Gard (bản Rust)

Phần mềm nhận kết quả đo từ BYK haze-gard plus qua RS232, ghi `hazegarddata<yyyymmdd>.csv`
(mỗi ngày 1 file). Chức năng giống bản Python `hazegard_gui.py`; dùng chung file
`hazegard_config.json` và `hazegard_raw.log`.

## Lấy file .exe (không cần cài Rust)
1. Tạo repository GitHub (private), upload toàn bộ thư mục này (giữ nguyên `.github/workflows/build.yml`).
2. Vào tab **Actions** → **Build Windows exe** → chờ chạy xong (khoảng 5–10 phút lần đầu).
3. Mở lần chạy đó → mục **Artifacts** → tải `hazegard_gui` (file zip chứa `hazegard_gui.exe`).
   Có thể bấm **Run workflow** để build lại bất cứ lúc nào.

## Tự build trên máy Windows
Cài Rust (https://rustup.rs) và Visual Studio Build Tools (workload "Desktop development with C++"), rồi:
```
cargo build --release
```
File nằm ở `target\release\hazegard-gui.exe`.

## Dùng
- Chỉ cần 1 file `.exe`. Cạnh nó sẽ tự sinh `hazegard_config.json` và các file CSV.
- Muốn hiện ảnh máy thật: đặt `hazegard.png` (hoặc `.jpg`) cạnh file exe.
- Khởi động an toàn: chương trình tự thử lần lượt DirectX 12 → Vulkan → OpenGL; nếu một kiểu làm sập
  lúc khởi động thì tự chuyển sang kiểu kế tiếp. Mỗi lần thử được ghi vào `hazegard_startup.log`
  (gửi file này để chẩn đoán nếu máy nào đó không mở được).
- Gỡ lỗi: chạy `hazegard_gui.exe --direct` để chạy thẳng không qua bước thử tự động.
- Nếu chương trình không mở được: sẽ hiện hộp thoại báo lỗi và ghi `hazegard_crash.log` cạnh file exe
  (gửi nội dung file này để chẩn đoán).
- Windows có thể chặn file tải về: chuột phải file .exe → Properties → tick **Unblock** → OK;
  hoặc ở cảnh báo SmartScreen chọn **More info → Run anyway**. Nếu bạn giải nén từ file .zip, hãy
  giải nén ra thư mục riêng trước khi chạy.
- Yêu cầu đồ họa: cần driver card màn hình bình thường (DirectX 12 / Vulkan / OpenGL).
  Nếu máy quá cũ hoặc chạy qua Remote Desktop mà không mở được, dùng bản Python.

## Thử trên Linux (phát triển)
```
cargo test --lib
cargo run --example gui_demo -- <thư mục lưu ảnh chụp>   # máy giả lập + chụp màn hình
```
