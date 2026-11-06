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
