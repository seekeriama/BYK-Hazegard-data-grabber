# Haze Gard (bản Rust)

Phần mềm nhận kết quả đo từ BYK haze-gard plus qua RS232, ghi `hazegarddata<yyyymmdd>.csv`
(mỗi ngày 1 file). Chức năng giống bản Python `hazegard_gui.py`; dùng chung file
`hazegard_config.json` và `hazegard_raw.log`.
<img width="1094" height="1093" alt="image" src="https://github.com/user-attachments/assets/7c859f21-08c7-4bf2-9630-48639d811a85" />

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
- Nếu chương trình đóng đột ngột, xem `hazegard_crash.log` cạnh file exe.
- Yêu cầu đồ họa: cần driver card màn hình bình thường (DirectX 12 / Vulkan / OpenGL).
  Nếu máy quá cũ hoặc chạy qua Remote Desktop mà không mở được, dùng bản Python.

## Thử trên Linux (phát triển)
```
cargo test --lib
cargo run --example gui_demo -- <thư mục lưu ảnh chụp>   # máy giả lập + chụp màn hình
```
