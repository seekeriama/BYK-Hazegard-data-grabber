# Haze Gard (bản Python)

Gồm `hazegard_gui.py` (giao diện) và `hazegard_reader.py` (giao tiếp máy). Hai file phải nằm cùng thư mục.

## Lấy file .exe bằng GitHub Actions
1. Tạo repository GitHub (private), upload toàn bộ thư mục này, giữ nguyên `.github/workflows/build-python.yml`.
2. Tab **Actions** → **Build Python exe** → chờ chạy xong (khoảng 2–3 phút).
3. Mở lần chạy đó → mục **Artifacts** → tải `hazegard_gui` (zip chứa `hazegard_gui.exe`).
Có thể bấm **Run workflow** để build lại bất cứ lúc nào, ví dụ sau khi sửa file .py.

## Lưu ý
- Windows có thể chặn file tải về: chuột phải .exe → Properties → tick **Unblock**.
- Phần mềm diệt virus đôi khi cảnh báo nhầm file đóng gói bằng PyInstaller; nếu gặp, thêm file vào danh sách cho phép.
- Khi copy sang máy khác, chỉ cần file `.exe`. Không copy `hazegard_config.json` của máy cũ.


## Hỗ trợ máy haze-gard i (USB)

- Trong Cài đặt (F12) chọn **Đời máy = i**. Máy plus vẫn dùng RS232/COM như cũ.
- Máy i dùng thư viện BYK `BYKCommunication.dll` (bản 32-bit): đặt file này **cùng thư mục với file exe**.
- Cần cài driver USB (WinUSB) của BYK cho máy i (có trong bộ cài phần mềm gốc). Device Manager không còn dấu cảnh báo vàng ở "haze-gard i".
- Phải build bằng **Python 32-bit** (workflow đã đặt `architecture: x86`). Build local: cài Python 3.12 bản Windows x86, rồi
  `python -m pip install pyserial pyinstaller` và
  `python -m PyInstaller --onefile --noconsole --hidden-import hazegard_i --name hazegard_gui hazegard_gui.py`.
- Đóng Client_HazeGard.exe trước khi chạy (DLL chỉ cho một chương trình dùng tại một thời điểm).
- Máy i đo trực tiếp trên máy (F5 không dùng); mỗi lần đo xong chương trình tự đọc kết quả.
- Nếu lỗi, xem `hazegard_raw.log` cạnh exe (ghi các lệnh XML gửi/nhận) và gửi cho tôi.
