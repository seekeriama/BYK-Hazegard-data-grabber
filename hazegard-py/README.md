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
- Khi copy sang máy khác, chỉ cần file `.exe` (+ `BYKCommunication.dll` nếu dùng máy i).
- Cài đặt được lưu trong **Registry Windows** (`HKEY_CURRENT_USER\Software\Pilkington\HazeGardLogger`), KHÔNG tạo file json; mỗi tài khoản Windows có cài đặt riêng, máy mới sẽ phải cài đặt lại. Nếu cạnh exe còn `hazegard_config.json` cũ và Registry chưa có thì tự nhập vào Registry. Reset: `reg delete HKCU\Software\Pilkington\HazeGardLogger /f`.


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


## Tên file CSV theo hệ thống

Trong Cài đặt (F12), nhóm **Định dạng tên file CSV** có 3 ô chọn, bắt buộc chọn ít nhất 1, tối đa cả 3:
- **Default** → `hazegarddata<yyyymmdd>.csv`
- **ERS** → `Hazegard_Data_ERS_<yyyymmdd>.csv`
- **Sequoia** → `Hazegard_Data_Sequoia_<yyyymmdd>.csv`

Chọn nhiều định dạng thì mỗi lần đo được ghi vào tất cả các file đã chọn. Nếu một file đang mở trong Excel, chương trình vẫn ghi các file còn lại và tự ghi bù file kia khi đóng. Cấu trúc cột bên trong giữ nguyên.

## Ảnh nền

Trong Cài đặt (F12) → **Ảnh nền**: bấm "..." chọn file ảnh (PNG/GIF/JPG/BMP), ảnh tự thu nhỏ vừa khung 560×280; "Mặc định" để quay lại hình minh họa. Nếu không chọn, chương trình dùng `hazegard.png/.gif/.jpg` đặt cạnh exe (nếu có). PNG/GIF đọc trực tiếp; JPG/BMP cần Pillow (workflow đã cài sẵn; build local: `pip install pillow`). Nếu không đọc được ảnh, lý do hiện trong khung log.
