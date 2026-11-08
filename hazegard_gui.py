"""
Haze Gard  -  giao dien nhan ket qua do tu BYK haze-gard plus qua RS232

Can: Python 3 + pyserial (pip install pyserial)
     file hazegard_reader.py nam CUNG THU MUC voi file nay.
Chay: python hazegard_gui.py

Quy trinh:
  1. Nhap RunNumber, RunMinute; chon cac vi tri can do (LE, LC, CC, RC, RE).
  2. Bam nut operate tren may / dap foot switch (hoac phim F5 tren may tinh).
     Moi ket qua nhan duoc gan lan luot cho vi tri ke tiep theo thu tu LE, LC, CC, RC, RE.
  3. Do du so vi tri da chon -> RunMinute tu xoa de nhap so moi, RunNumber giu nguyen.
Ket qua (da cong offset) ghi vao hazegarddata<yyyymmdd>.csv, moi ngay 1 file.

Cai dat (cong COM, ten Line, thu muc luu, offset %T / %H):
  - nhap dup vao dong "Connected: ..." o tren cung, hoac bam F12
  - luu trong file hazegard_config.json canh chuong trinh
"""

import sys as _sys, os as _os, traceback as _tb


def _report_fatal(text):
    """Exe --noconsole khong hien loi -> ghi hazegard_crash.log va bao bang hop thoai."""
    try:
        base = _os.path.dirname(_sys.executable if getattr(_sys, "frozen", False)
                                else _os.path.abspath(__file__))
        with open(_os.path.join(base, "hazegard_crash.log"), "a", encoding="utf-8") as f:
            f.write(text + "\n")
    except Exception:
        pass
    try:
        import tkinter as _tk
        from tkinter import messagebox as _mb
        r = _tk.Tk(); r.withdraw()
        _mb.showerror("HazeGard - loi khoi dong", text[-1500:])
    except Exception:
        pass


def _excepthook(et, ev, tb):
    _report_fatal("".join(_tb.format_exception(et, ev, tb)))


_sys.excepthook = _excepthook

import hashlib
import json
import os
import queue
import re
import sys
import threading
import time
from datetime import datetime
import tkinter as tk
from tkinter import ttk, messagebox, filedialog, simpledialog

import hazegard_reader as hr

try:
    import serial
    from serial.tools import list_ports
except ImportError:
    _report_fatal("Chua cai pyserial. Chay: pip install pyserial")
    sys.exit(1)

if getattr(sys, "frozen", False):          # chay tu file .exe (PyInstaller)
    APP_DIR = os.path.dirname(sys.executable)
else:
    APP_DIR = os.path.dirname(os.path.abspath(__file__))

CONFIG_PATH = os.path.join(APP_DIR, "hazegard_config.json")
RAW_LOG = os.path.join(APP_DIR, "hazegard_raw.log")
DEFAULT_CONFIG = {"port": "", "line": "VGI3", "outdir": "",   # "" = thu muc chua chuong trinh
                  "t_offset": 0.0, "h_offset": 0.0,
                  "release_first": True}
# Password xac nhan khi luu cai dat (chi luu ma bam SHA-256, khong luu chu ro)
SETTINGS_PW_SHA256 = "f51413286fbac40dfc6e9c3757427acf784c86fd80b749f1ccb5d23de51414df"

POSITIONS = ["LE", "LC", "CC", "RC", "RE"]         # thu tu do
BUTTON_ORDER = ["RE", "RC", "CC", "LC", "LE"]      # thu tu hien thi tu tren xuong

C_BG = "#f3f3f3"
C_OFF = "#e9e9e9"       # vi tri khong chon
C_ON = "#cfe5ff"        # vi tri da chon
C_NEXT = "#ffd966"      # vi tri do tiep theo
C_DONE = "#c6efce"      # vi tri da do xong trong luot nay
C_OK = "#1e8c5a"
C_WARN = "#c55a11"
C_ERR = "#c00000"


def load_config():
    cfg = dict(DEFAULT_CONFIG)
    try:
        with open(CONFIG_PATH, encoding="utf-8") as f:
            cfg.update(json.load(f))
    except (OSError, ValueError):
        pass
    return cfg


def save_config(cfg):
    with open(CONFIG_PATH, "w", encoding="utf-8") as f:
        json.dump(cfg, f, indent=2, ensure_ascii=False)


def fmt_offset(v):
    return f"{v:g}"


# =====================================================================
#  Giao tiep voi may (chay o luong rieng, gui su kien ve giao dien qua queue)
# =====================================================================
class Instrument(threading.Thread):
    def __init__(self, port, events, release_first=True):
        super().__init__(daemon=True)
        self.port = port
        self.release_first = release_first
        self.q = events
        self.stop_evt = threading.Event()
        self.ser = None
        self.online = False

    def run(self):
        try:
            self.ser = hr.open_port(self.port, 9600, timeout=0.2)
        except (serial.SerialException, ValueError) as e:
            self.q.put(("error", f"Không mở được {self.port}: {e}"))
            return
        try:
            raw_f = open(RAW_LOG, "ab")
        except OSError:
            raw_f = None

        def log_raw(b):
            if raw_f and b:
                raw_f.write(b)
                raw_f.flush()

        try:
            # 1) Thu vao online mode (lenh 4Fh)
            if self.release_first:
                # 0) Giai phong may dang ket o Online Mode (lenh 50h), roi moi handshake
                self.ser.reset_input_buffer()
                self.ser.write(bytes([hr.CMD_END]))
                time.sleep(0.4)
            self.ser.reset_input_buffer()
            self.ser.write(bytes([hr.CMD_ONLINE]))
            resp, t0 = b"", time.time()
            while time.time() - t0 < 2 and len(resp) < 12 and not self.stop_evt.is_set():
                resp += self.ser.read(12 - len(resp))
            log_raw(resp)
            if len(resp) >= 12 and resp[0] == hr.CMD_ONLINE and resp[1] == hr.ACK:
                self.online = True
                self.q.put(("connected", {"online": True, "status": resp[2:12]}))
            else:
                # 2) Khong co phan hoi -> nghe thu dong (may o che do Print Online)
                self.q.put(("connected", {"online": False, "status": None}))

            parser = hr.OnlineParser(on_switch=lambda: self.q.put(("port", "clarity")))
            # che do thu dong: byte nhan duoc trong luc cho 4Fh co the la ket qua do -> giu lai
            buf, last_rx = (b"" if self.online else resp), time.time()
            while not self.stop_evt.is_set():
                chunk = self.ser.read(64)
                if chunk:
                    log_raw(chunk)
                    last_rx = time.time()
                if self.online:
                    for vals, raw in parser.feed(chunk):
                        t, h = (vals + [None, None])[:2]
                        self.q.put(("result", (t, h, " | ".join(raw))))
                    continue
                # che do thu dong: tach dong theo CR/LF hoac 1 s im lang
                buf += chunk
                lines = re.split(rb"[\r\n]+", buf)
                if buf and time.time() - last_rx > 1.0:
                    complete, buf = lines, b""
                else:
                    complete, buf = lines[:-1], lines[-1]
                for rawline in complete:
                    text = rawline.decode("latin-1", errors="replace").strip()
                    if not text:
                        continue
                    v = hr.parse_line(text)
                    if v["transmission"] is not None or v["haze"] is not None:
                        self.q.put(("result", (v["transmission"], v["haze"], text)))
        except serial.SerialException as e:
            self.q.put(("error", f"Mất kết nối {self.port}: {e}"))
        finally:
            try:
                if self.online:
                    self.ser.write(bytes([hr.CMD_END]))     # thoat online mode, mo khoa ban phim may
                    time.sleep(0.2)
                self.ser.close()
            except Exception:
                pass
            if raw_f:
                raw_f.close()

    def send(self, code):
        if self.ser and self.ser.is_open:
            try:
                self.ser.write(bytes([code]))
                return True
            except serial.SerialException:
                return False
        return False

    def stop(self):
        self.stop_evt.set()


# =====================================================================
#  Giao dien
# =====================================================================
class HazeGardApp:
    def __init__(self, root):
        self.root = root
        self.cfg = load_config()
        self.events = queue.Queue()
        self.inst = None
        self.connected = False
        self.k = 0                  # so ket qua da nhan trong luot do hien tai
        self.seq = []               # thu tu vi tri cua luot hien tai
        self.pending = []           # dong CSV chua ghi duoc (file dang mo trong Excel)
        self.selected = {p: True for p in POSITIONS}
        self.store = self.make_store()

        root.title("Haze Gard")
        root.configure(bg=C_BG)
        root.resizable(False, False)
        root.protocol("WM_DELETE_WINDOW", self.on_exit)
        self._build()
        self.apply_config()
        if getattr(self, "_outdir_warning", None):
            self.write_log(self._outdir_warning, "warn")
            self._outdir_warning = None
        root.bind("<F5>", lambda e: self.trigger_measure())
        root.bind("<F12>", lambda e: self.open_settings())
        root.after(100, self.poll)
        root.after(5000, self.retry_pending)
        root.after(200, self.start_connection)

    # ---------- dung giao dien ----------
    def _build(self):
        r = self.root
        top = tk.Frame(r, bg="#ffffff", bd=0)
        top.grid(row=0, column=0, columnspan=3, sticky="ew", padx=12, pady=(8, 0))
        self.lbl_conn = tk.Label(top, text="Not connected", bg="#ffffff",
                                 font=("Segoe UI", 10), anchor="w", padx=8, pady=4)
        self.lbl_conn.pack(fill="x")
        self.lbl_conn.bind("<Double-Button-1>", lambda e: self.open_settings())

        self.canvas = tk.Canvas(r, width=560, height=280, bg="#eef3f8",
                                highlightthickness=0)
        self.canvas.grid(row=1, column=0, columnspan=3, padx=12, pady=6)
        self._draw_picture()

        # --- cot trai: thong tin ---
        left = tk.Frame(r, bg=C_BG)
        left.grid(row=2, column=0, sticky="nw", padx=(12, 4))
        self.var = {k: tk.StringVar() for k in
                    ("address", "serial", "device", "run_number", "run_minute",
                     "delay", "reading", "of", "tcorr", "hcorr", "status")}
        self.var["delay"].set("0")
        self.var["reading"].set("0")

        def lbl(row, text):
            tk.Label(left, text=text, bg=C_BG, anchor="e", width=12).grid(
                row=row, column=0, sticky="e", pady=2)

        def ro_entry(row, key, width=18, col=1, colspan=3):
            e = tk.Entry(left, textvariable=self.var[key], width=width, state="readonly",
                         readonlybackground="#f7f7f7", relief="solid", bd=1, justify="center")
            e.grid(row=row, column=col, columnspan=colspan, sticky="w", pady=2)
            return e

        lbl(0, "Address");   ro_entry(0, "address")
        lbl(1, "Serial #");  ro_entry(1, "serial")
        lbl(2, "Device ID"); ro_entry(2, "device")

        self.chk = {k: tk.BooleanVar(value=k in ("T", "H")) for k in ("T", "H", "C", "CONT")}
        f_chk = tk.Frame(left, bg=C_BG)
        f_chk.grid(row=3, column=0, columnspan=4, rowspan=2, sticky="w")
        for i, (txt, key, col) in enumerate([("Transmission", "T", 0), ("Clarity", "C", 2),
                                             ("Haze", "H", 0), ("Continuous", "CONT", 2)]):
            tk.Label(f_chk, text=txt, bg=C_BG, anchor="e", width=12 if col == 0 else 10).grid(
                row=i // 2, column=col, sticky="e")
            tk.Checkbutton(f_chk, variable=self.chk[key], state="disabled", bg=C_BG).grid(
                row=i // 2, column=col + 1, sticky="w")

        lbl(5, "RunNumber")
        self.ent_rn = tk.Entry(left, textvariable=self.var["run_number"], width=22,
                               relief="solid", bd=1)
        self.ent_rn.grid(row=5, column=1, columnspan=3, sticky="w", pady=3)
        lbl(6, "RunMinute")
        self.ent_rm = tk.Entry(left, textvariable=self.var["run_minute"], width=22,
                               relief="solid", bd=1)
        self.ent_rm.grid(row=6, column=1, columnspan=3, sticky="w", pady=3)

        lbl(7, "Delay (s)");  ro_entry(7, "delay", width=6, colspan=1)
        lbl(8, "Reading")
        ro_entry(8, "reading", width=6, col=1, colspan=1)
        tk.Label(left, text="of", bg=C_BG).grid(row=8, column=2)
        ro_entry(8, "of", width=6, col=3, colspan=1)

        lbl(9, "%T Corrected")
        self.ent_t = ro_entry(9, "tcorr")
        lbl(10, "%H Corrected")
        self.ent_h = ro_entry(10, "hcorr")
        for w in (self.ent_t, self.ent_h):
            w.bind("<Double-Button-1>", lambda e: self.open_settings())

        # --- cot giua: nhat ky ket qua ---
        mid = tk.Frame(r, bg=C_BG)
        mid.grid(row=2, column=1, sticky="nsew", padx=4)
        self.log = tk.Text(mid, width=30, height=17, font=("Consolas", 10),
                           relief="solid", bd=1, wrap="word")
        sb = tk.Scrollbar(mid, command=self.log.yview)
        self.log.configure(yscrollcommand=sb.set, state="disabled")
        self.log.pack(side="left", fill="both", expand=True)
        sb.pack(side="right", fill="y")
        self.log.tag_configure("warn", foreground=C_WARN)
        self.log.tag_configure("err", foreground=C_ERR)

        # --- cot phai: 5 vi tri ---
        right = tk.Frame(r, bg=C_BG, relief="groove", bd=1)
        right.grid(row=2, column=2, sticky="ns", padx=(4, 12), pady=(60, 20))
        self.btn = {}
        for p in BUTTON_ORDER:
            b = tk.Button(right, text=p, width=4, relief="raised", bd=1,
                          command=lambda p=p: self.toggle(p))
            b.pack(padx=8, pady=6)
            self.btn[p] = b

        # --- hang duoi ---
        bottom = tk.Frame(r, bg=C_BG)
        bottom.grid(row=3, column=0, columnspan=3, sticky="ew", padx=12, pady=(4, 10))
        bottom.columnconfigure(1, weight=1)
        tk.Button(bottom, text="Exit", width=10, command=self.on_exit).grid(
            row=0, column=0, padx=(150, 0))
        self.lbl_status = tk.Label(bottom, textvariable=self.var["status"], bg=C_BG,
                                   fg=C_OK, font=("Segoe UI", 12, "bold"), width=18)
        self.lbl_status.grid(row=0, column=1)
        tk.Button(bottom, text="Clear", width=12, command=self.clear).grid(row=0, column=2)

        self.ent_rn.focus_set()

    def _draw_picture(self):
        """Hien anh hazegard.png / .gif dat canh chuong trinh neu co; neu khong ve hinh minh hoa."""
        c = self.canvas
        for name in ("hazegard.png", "hazegard.gif", "hazegard.jpg"):
            path = os.path.join(APP_DIR, name)
            if not os.path.exists(path):
                continue
            try:
                try:
                    from PIL import Image, ImageTk
                    im = Image.open(path)
                    im.thumbnail((560, 280))
                    self._img = ImageTk.PhotoImage(im)
                except ImportError:
                    self._img = tk.PhotoImage(file=path)
                c.create_image(280, 140, image=self._img)
                return
            except Exception:
                pass
        # Hinh minh hoa don gian: than may, cong do, gia kep mau
        blue, dark = "#2f86d1", "#1f5f99"
        c.create_rectangle(0, 240, 560, 280, fill="#d0d6dc", outline="")
        c.create_rectangle(60, 222, 500, 240, fill="#8a9099", outline="")       # ray
        c.create_rectangle(30, 50, 230, 245, fill=blue, outline=dark, width=2)  # khoi trai
        c.create_rectangle(55, 75, 165, 160, fill="#4a4f57", outline="#30343a", width=3)
        c.create_oval(180, 105, 210, 135, fill="#e8e8e8", outline="#999")
        c.create_oval(222, 95, 262, 175, fill=dark, outline="")                # cong haze
        c.create_rectangle(360, 50, 530, 245, fill=blue, outline=dark, width=2)  # khoi phai
        c.create_oval(330, 80, 380, 190, fill=dark, outline="")                # quang cau
        c.create_rectangle(296, 120, 304, 222, fill="#222", outline="")         # gia kep
        c.create_oval(282, 100, 318, 180, outline="#222", width=4)
        c.create_rectangle(278, 205, 322, 222, fill="#333", outline="")

    # ---------- cau hinh ----------
    def apply_config(self):
        t, h = float(self.cfg["t_offset"]), float(self.cfg["h_offset"])
        self.var["tcorr"].set(f"True: {fmt_offset(t)}" if t else "False: 0")
        self.var["hcorr"].set(f"True: {fmt_offset(h)}" if h else "False: 0")
        self.store = self.make_store()
        self.update_conn_label()
        self.refresh_buttons()

    def make_store(self):
        """Thu muc luu CSV; neu khong tao/ghi duoc (vd duong dan cua may khac) -> dung thu muc chuong trinh."""
        want = (self.cfg.get("outdir") or "").strip() or APP_DIR
        try:
            store = hr.DailyCsv(want)
            test = os.path.join(want, ".hazegard_write_test")
            open(test, "w").close()
            os.remove(test)
            return store
        except OSError:
            self._outdir_warning = (f"Không dùng được thư mục lưu:\n  {want}\n"
                                    f"→ Tạm lưu CSV vào thư mục chương trình.\n"
                                    f"Bấm F12 để chọn lại thư mục.\n\n")
            if hasattr(self, "log"):
                self.write_log(self._outdir_warning, "warn")
                self._outdir_warning = None
            return hr.DailyCsv(APP_DIR)

    def update_conn_label(self):
        if self.connected:
            self.lbl_conn.configure(text=f"Connected: {self.cfg['line']}", fg="#000")
        else:
            self.lbl_conn.configure(text="Chưa kết nối  (nhấp đúp vào đây để cài đặt)", fg=C_ERR)

    def open_settings(self):
        SettingsDialog(self)

    # ---------- ket noi ----------
    def start_connection(self):
        if self.inst:
            self.inst.stop()
            self.inst.join(timeout=2)
            self.inst = None
        self.connected = False
        self.update_conn_label()
        for k in ("address", "serial", "device"):
            self.var[k].set("")
        port = self.cfg.get("port", "")
        if not port:
            self.set_status("chưa chọn cổng COM", C_ERR)
            self.open_settings()
            return
        self.set_status("đang kết nối ...", C_WARN)
        self.inst = Instrument(port, self.events, bool(self.cfg.get("release_first", True)))
        self.inst.start()

    # ---------- vi tri do ----------
    def toggle(self, p):
        if self.k > 0:
            self.write_log("Đang trong lượt đo – bấm Clear để đổi vị trí.\n", "warn")
            return
        self.selected[p] = not self.selected[p]
        self.refresh_buttons()

    def current_seq(self):
        return self.seq if self.k > 0 else [p for p in POSITIONS if self.selected[p]]

    def refresh_buttons(self):
        seq = self.current_seq()
        nxt = seq[self.k] if self.k < len(seq) else None
        done = set(seq[:self.k])
        for p, b in self.btn.items():
            if p == nxt:
                bg, fnt = C_NEXT, ("Segoe UI", 9, "bold")
            elif p in done:
                bg, fnt = C_DONE, ("Segoe UI", 9)
            elif self.selected[p]:
                bg, fnt = C_ON, ("Segoe UI", 9, "bold")
            else:
                bg, fnt = C_OFF, ("Segoe UI", 9)
            b.configure(bg=bg, activebackground=bg, font=fnt,
                        relief="sunken" if self.selected[p] else "raised")
        self.var["reading"].set(str(self.k))
        self.var["of"].set(str(len(seq)))
        lock = "disabled" if self.k > 0 else "normal"
        self.ent_rn.configure(state=lock)
        self.ent_rm.configure(state=lock)

    # ---------- nhan ket qua ----------
    def check_ready(self):
        if not self.current_seq():
            return "Chưa chọn vị trí đo"
        if not self.var["run_number"].get().strip():
            return "Chưa nhập RunNumber"
        if not self.var["run_minute"].get().strip():
            return "Chưa nhập RunMinute"
        return None

    def trigger_measure(self):
        if not (self.inst and self.inst.online):
            self.set_status("bấm operate trên máy", C_WARN)
            return
        problem = self.check_ready()
        if problem:
            self.set_status(problem, C_ERR)
            self.root.bell()
            return
        self.inst.send(hr.CMD_OPERATE)
        self.set_status("đang đo ...", C_WARN)

    def on_result(self, t, h, raw):
        problem = self.check_ready()
        if problem:
            self.write_log(f"Nhận T={t} H={h} nhưng KHÔNG LƯU: {problem}\n\n", "err")
            self.set_status(problem, C_ERR)
            self.root.bell()
            return
        if self.k == 0:
            self.seq = [p for p in POSITIONS if self.selected[p]]
        pos = self.seq[self.k]
        toff, hoff = float(self.cfg["t_offset"]), float(self.cfg["h_offset"])
        tc = round(t + toff, 1) if t is not None else None
        hc = round(h + hoff, 2) if h is not None else None
        now = datetime.now()
        row = dict(line=self.cfg["line"], run_number=self.var["run_number"].get().strip(),
                   run_minute=self.var["run_minute"].get().strip(), position=pos,
                   t_offset=fmt_offset(toff), h_offset=fmt_offset(hoff))
        self.save_row(now, tc, hc, row)

        ts = f"{tc:.1f}" if tc is not None else "-"
        hs = f"{hc:.2f}" if hc is not None else "-"
        rm = row["run_minute"]          # RunMinute do nguoi van hanh nhap (khong con so 04 co dinh)
        self.write_log(f"{rm}: {pos}- %T: {ts}\n{rm}: {pos}- %H: {hs}\n\n")
        self.k += 1
        if self.k >= len(self.seq):          # xong 1 luot
            self.k = 0
            self.seq = []
            self.refresh_buttons()
            self.var["run_minute"].set("")
            self.ent_rm.focus_set()
        else:
            self.refresh_buttons()
        self.set_status("port haze", C_OK)

    def save_row(self, now, tc, hc, row):
        try:
            self.store.write(now, tc, hc, **row)
        except OSError:
            self.pending.append((now, tc, hc, row))
            self.write_log("!! Không ghi được file CSV (đang mở trong Excel?). "
                           "Sẽ tự ghi lại khi đóng file.\n", "err")

    def retry_pending(self):
        still = []
        for item in self.pending:
            try:
                self.store.write(item[0], item[1], item[2], **item[3])
            except OSError:
                still.append(item)
        if self.pending and not still:
            self.write_log(f"Đã ghi bù {len(self.pending)} kết quả vào file CSV.\n")
        self.pending = still
        self.root.after(5000, self.retry_pending)

    # ---------- su kien tu luong doc cong COM ----------
    def poll(self):
        try:
            while True:
                kind, data = self.events.get_nowait()
                if kind == "connected":
                    self.on_connected(data)
                elif kind == "result":
                    self.on_result(*data)
                elif kind == "port":
                    self.set_status("port clarity", C_WARN)
                elif kind == "error":
                    self.connected = False
                    self.update_conn_label()
                    self.write_log(data + "\n", "err")
                    self.set_status("lỗi kết nối", C_ERR)
        except queue.Empty:
            pass
        self.root.after(100, self.poll)

    def on_connected(self, info):
        self.connected = True
        self.update_conn_label()
        self.var["address"].set(self.cfg["port"])
        st = info["status"]
        if info["online"] and st:
            mode = st[1]
            self.chk["T"].set(mode in (0, 3, 4, 6))
            self.chk["H"].set(mode in (1, 3, 5, 6))
            self.chk["C"].set(mode in (2, 4, 5, 6))
            self.chk["CONT"].set(bool(st[4]))
            self.var["device"].set(f"haze-gard plus  FW {st[9]:02X}{st[8]:02X}")
            if not st[5]:
                self.write_log("Cảnh báo: máy báo calibration KHÔNG hợp lệ.\n", "warn")
            if st[4]:
                self.write_log("Cảnh báo: máy đang ở Continuous mode.\n", "warn")
            self.write_log("Kết nối online (4Fh). F5 = đo từ máy tính.\n\n")
        else:
            self.write_log("Máy không trả lời lệnh 4Fh → nghe thụ động. "
                           "Bấm operate trên máy để đo.\n\n", "warn")
        self.set_status("port haze", C_OK)

    # ---------- tien ich ----------
    def write_log(self, text, tag=None):
        self.log.configure(state="normal")
        self.log.insert("end", text, tag)
        self.log.see("end")
        self.log.configure(state="disabled")

    def set_status(self, text, color=C_OK):
        self.var["status"].set(text)
        self.lbl_status.configure(fg=color)

    def clear(self):
        self.log.configure(state="normal")
        self.log.delete("1.0", "end")
        self.log.configure(state="disabled")
        self.k = 0
        self.seq = []
        self.refresh_buttons()
        if self.connected:
            self.set_status("port haze", C_OK)

    def on_exit(self):
        if self.pending and not messagebox.askyesno(
                "Haze Gard", f"Còn {len(self.pending)} kết quả chưa ghi được vào CSV "
                             "(file đang mở?). Vẫn thoát?"):
            return
        if self.inst:
            self.inst.stop()
            self.inst.join(timeout=2)
        self.root.destroy()


class SettingsDialog(tk.Toplevel):
    def __init__(self, app):
        super().__init__(app.root)
        self.app = app
        self.title("Cài đặt")
        self.resizable(False, False)
        self.transient(app.root)
        cfg = app.cfg
        self.v = {"port": tk.StringVar(value=cfg["port"]),
                  "line": tk.StringVar(value=cfg["line"]),
                  "outdir": tk.StringVar(value=cfg.get("outdir") or APP_DIR),
                  "t_offset": tk.StringVar(value=fmt_offset(float(cfg["t_offset"]))),
                  "h_offset": tk.StringVar(value=fmt_offset(float(cfg["h_offset"])))}
        self.release_var = tk.BooleanVar(value=bool(cfg.get("release_first", True)))
        ports = [p.device for p in list_ports.comports()]
        rows = [("Cổng COM", "port"), ("Line", "line"), ("Thư mục lưu CSV", "outdir"),
                ("%T offset", "t_offset"), ("%H offset", "h_offset")]
        for i, (txt, key) in enumerate(rows):
            tk.Label(self, text=txt, anchor="e", width=14).grid(row=i, column=0, padx=6, pady=4)
            if key == "port":
                w = ttk.Combobox(self, textvariable=self.v[key], values=ports, width=30)
            else:
                w = tk.Entry(self, textvariable=self.v[key], width=33)
            w.grid(row=i, column=1, sticky="w", pady=4)
            if key == "outdir":
                tk.Button(self, text="...", command=self.browse).grid(row=i, column=2, padx=4)
        tk.Checkbutton(self, text="Giải phóng máy đang kẹt Online Mode khi khởi động (gửi 50h)",
                       variable=self.release_var).grid(row=len(rows), column=0, columnspan=3,
                                                       sticky="w", padx=8)
        tk.Label(self, text="Kết quả ghi vào CSV = giá trị máy đo + offset",
                 fg="#666").grid(row=len(rows) + 1, column=0, columnspan=3, pady=(4, 0))
        f = tk.Frame(self)
        f.grid(row=len(rows) + 2, column=0, columnspan=3, pady=8)
        tk.Button(f, text="Lưu & kết nối", width=14, command=self.save).pack(side="left", padx=6)
        tk.Button(f, text="Hủy", width=8, command=self.destroy).pack(side="left", padx=6)
        self.grab_set()

    def browse(self):
        d = filedialog.askdirectory(parent=self, initialdir=self.v["outdir"].get() or APP_DIR)
        if d:
            self.v["outdir"].set(d)

    def save(self):
        try:
            t = float(self.v["t_offset"].get().replace(",", "."))
            h = float(self.v["h_offset"].get().replace(",", "."))
        except ValueError:
            messagebox.showerror("Cài đặt", "Offset phải là số, vd -0.6", parent=self)
            return
        outdir = self.v["outdir"].get().strip() or APP_DIR
        try:
            os.makedirs(outdir, exist_ok=True)
        except OSError as e:
            messagebox.showerror("Cài đặt", f"Không tạo được thư mục:\n{e}", parent=self)
            return
        cfg = self.app.cfg
        new = dict(port=self.v["port"].get().strip(), line=self.v["line"].get().strip(),
                   outdir="" if os.path.normcase(os.path.abspath(outdir)) ==
                   os.path.normcase(APP_DIR) else outdir,
                   t_offset=t, h_offset=h,
                   release_first=self.release_var.get())
        changed = any(new[k] != cfg.get(k) for k in new) or not os.path.exists(CONFIG_PATH)
        if changed and not self.confirm_password():
            return
        cfg.update(new)
        try:
            save_config(cfg)
        except OSError as e:
            messagebox.showwarning("Cài đặt", f"Không lưu được file cấu hình:\n{e}", parent=self)
        self.app.apply_config()
        self.destroy()
        self.app.start_connection()


    def confirm_password(self):
        """Hoi password truoc khi luu. Tra ve True neu dung."""
        pw = simpledialog.askstring("Xác nhận thay đổi",
                                    "Nhập password để lưu cài đặt:",
                                    show="*", parent=self)
        if pw is None:                        # bam Cancel
            return False
        if hashlib.sha256(pw.encode("utf-8")).hexdigest() != SETTINGS_PW_SHA256:
            messagebox.showerror("Cài đặt", "Sai password. Cài đặt CHƯA được lưu.", parent=self)
            return False
        return True


def main():
    try:
        root = tk.Tk()
        HazeGardApp(root)
        root.mainloop()
    except Exception:
        _report_fatal(_tb.format_exc())


if __name__ == "__main__":
    main()
