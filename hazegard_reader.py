"""
haze-gard plus (BYK) -> PC  |  Nhan ket qua do qua RS232 (USB-to-RS232)

Cai dat:   pip install pyserial
Cach dung:
  python hazegard_reader.py --list                 # liet ke cong COM
  python hazegard_reader.py --port COM3 --sniff    # do toc do baud phu hop
  python hazegard_reader.py --port COM3 --online   # PC dieu khien may (lenh 4Fh) - KHUYEN DUNG
  python hazegard_reader.py --port COM3 --baud 9600   # chi nghe thu dong (Print Online)
  python hazegard_reader.py --fix hazegard_results.csv  # doi file CSV cu sang dinh dang moi

Moi lan do, ket qua hien len man hinh va duoc ghi vao:
  - hazegarddata<yyyymmdd>.csv  (MOI NGAY 1 FILE, chi chua du lieu do trong ngay do)
      cot: Line, Measurement time, RunNumber, RunMinute, Position,
           CTransmission, CHaze, TOffsetApplied, HOffsetApplied
  - hazegard_raw.log            (du lieu tho, de kiem tra dinh dang)
Thu muc luu file chon bang --outdir (mac dinh: thu muc dang chay).
"""

import argparse
import csv
import os
import re
import sys
import time
from datetime import datetime

try:
    import serial
    from serial.tools import list_ports
except ImportError:
    sys.exit("Chua cai pyserial. Chay: pip install pyserial")

COMMON_BAUDS = [9600, 4800, 19200, 2400, 38400, 1200, 57600, 115200]
NUM_RE = re.compile(r"[-+]?\d+(?:[.,]\d+)?")
# Ngay may in ra: 11.05.26 / 05-11-2026 / 11/05/26 ; Gio: 12:03pm / 14:03 / 14:03:52
DATE_RE = re.compile(r"\b\d{1,2}[./-]\d{1,2}[./-]\d{2,4}\b")
TIME_RE = re.compile(r"\b\d{1,2}:\d{2}(?::\d{2})?\s*(?:[ap]\.?m\.?)?", re.I)

# Tu khoa nhan dien tung dai luong (khong phan biet hoa thuong)
LABELS = {
    "transmission": re.compile(r"\b(t|tt|trans\w*|transmission)\b", re.I),
    "haze": re.compile(r"\b(h|haze)\b", re.I),
    "clarity": re.compile(r"\b(c|cl|clar\w*|clarity)\b", re.I),
}


def list_com_ports():
    ports = list(list_ports.comports())
    if not ports:
        print("Khong tim thay cong COM nao. Kiem tra driver cap USB-RS232.")
    for p in ports:
        print(f"{p.device:10s}  {p.description}")


def open_port(port, baud, parity="N", stopbits=1, timeout=1.0):
    return serial.Serial(
        port=port,
        baudrate=baud,
        bytesize=serial.EIGHTBITS,
        parity={"N": serial.PARITY_NONE, "E": serial.PARITY_EVEN,
                "O": serial.PARITY_ODD}[parity],
        stopbits=serial.STOPBITS_ONE if stopbits == 1 else serial.STOPBITS_TWO,
        timeout=timeout,
    )


def printable_ratio(data: bytes) -> float:
    if not data:
        return 0.0
    ok = sum(1 for b in data if 32 <= b < 127 or b in (9, 10, 13))
    return ok / len(data)


def sniff(port, seconds_per_baud=15):
    """Thu tung toc do baud; trong luc do, bam DO tren may haze-gard."""
    print("Che do do baud. Voi MOI toc do, hay bam do 1 lan tren may.\n")
    best = None
    for baud in COMMON_BAUDS:
        print(f"--> Thu {baud} baud trong {seconds_per_baud}s ... (bam do ngay)")
        with open_port(port, baud, timeout=0.2) as ser:
            ser.reset_input_buffer()
            buf = b""
            t0 = time.time()
            while time.time() - t0 < seconds_per_baud:
                buf += ser.read(256)
        ratio = printable_ratio(buf)
        preview = buf[:120].decode("latin-1", errors="replace")
        print(f"    nhan {len(buf)} byte, ty le ky tu doc duoc {ratio:.0%}")
        if buf:
            print(f"    mau: {preview!r}")
        if buf and ratio > 0.9 and (best is None or ratio > best[1]):
            best = (baud, ratio)
        if best and best[1] > 0.98:
            break
    if best:
        print(f"\nToc do phu hop nhat: {best[0]} baud")
        print(f"Chay: python {os.path.basename(__file__)} --port {port} --baud {best[0]}")
    else:
        print("\nKhong nhan duoc du lieu ro rang o toc do nao. Xem muc 'Neu khong co du lieu'.")


def parse_line(line: str) -> dict:
    """Tach T / Haze / Clarity tu 1 dong. Uu tien theo nhan; neu khong co nhan thi theo thu tu."""
    result = {"transmission": None, "haze": None, "clarity": None}

    # 1) Tim theo cap "nhan  so" (vd: "T 91.2", "Haze: 0.45", "C=99.8")
    for m in re.finditer(r"([A-Za-z]+)\s*[:=]?\s*([-+]?\d+(?:[.,]\d+)?)", line):
        label, num = m.group(1), float(m.group(2).replace(",", "."))
        for key, pat in LABELS.items():
            if result[key] is None and pat.fullmatch(label):
                result[key] = num
                break

    # 2) Neu khong co nhan: bo ngay/gio may in kem (vd "11.05.26  12:03pm"),
    #    roi lay cac so con lai theo thu tu T, H, C
    if all(v is None for v in result.values()):
        rest = DATE_RE.sub(" ", line)
        rest = TIME_RE.sub(" ", rest)
        nums = [float(n.replace(",", ".")) for n in NUM_RE.findall(rest)]
        for key, val in zip(("transmission", "haze", "clarity"), nums):
            result[key] = val
    return result


CSV_HEADER = ["Line", "Measurement time", "RunNumber", "RunMinute", "Position",
              "CTransmission", "CHaze", "TOffsetApplied", "HOffsetApplied"]


def fmt_num(v, nd):
    return "" if v is None else f"{v:.{nd}f}"


class DailyCsv:
    """Ghi moi ngay 1 file hazegarddata<yyyymmdd>.csv, chi chua du lieu cua ngay do.
    Ten file lay theo ngay cua tung lan do nen qua nua dem tu dong sang file moi."""

    def __init__(self, outdir=".", **extras):
        self.outdir = outdir
        self.extras = {"line": "", "run_number": "", "run_minute": "", "position": "",
                       "t_offset": "", "h_offset": ""}
        self.extras.update({k: v for k, v in extras.items() if v is not None})
        os.makedirs(outdir, exist_ok=True)

    def path_for(self, dt):
        return os.path.join(self.outdir, f"hazegarddata{dt:%Y%m%d}.csv")

    def write(self, dt, t, h, **override):
        e = {**self.extras, **override}
        path = self.path_for(dt)
        new = not os.path.exists(path) or os.path.getsize(path) == 0
        row = [e["line"], dt.strftime("%d/%m/%Y %H:%M:%S"), e["run_number"],
               e["run_minute"],                   # ghi dang so (vd 540), khong co =" "
               e["position"], fmt_num(t, 1), fmt_num(h, 2), e["t_offset"], e["h_offset"]]
        with open(path, "a", newline="", encoding="utf-8") as f:
            if new:
                f.write(",".join(CSV_HEADER) + "\r\n")
            # O trong ghi la "" (co dau nhay) de Excel/LibreOffice khong gop cac dau phay lien tiep
            f.write(",".join('""' if str(x) == "" else str(x) for x in row) + "\r\n")
        return path


def convert_old_csv(path, store):
    """Doi file CSV cu (Thoi gian, Transmission, Haze, Clarity, Chuoi goc) sang
    dinh dang moi, tach theo ngay; tinh lai T/H tu cot 'Chuoi goc'."""
    with open(path, newline="", encoding="utf-8-sig") as f:
        rows = list(csv.reader(f))[1:]
    files = set()
    for r in rows:
        if len(r) < 5:
            continue
        try:
            dt = datetime.strptime(r[0].strip(), "%Y-%m-%d %H:%M:%S")
        except ValueError:
            continue
        v = parse_line(r[4])
        files.add(store.write(dt, v["transmission"], v["haze"]))
    print(f"Da doi {len(rows)} dong -> {len(files)} file:")
    for fp in sorted(files):
        print("  " + os.path.abspath(fp))


def listen(port, baud, parity, stopbits, store, raw_log):
    raw_f = open(raw_log, "ab")

    print(f"Dang nghe {port} @ {baud} baud. Bam do tren may haze-gard. Ctrl+C de dung.\n")
    buf = b""
    last_rx = time.time()
    try:
        with open_port(port, baud, parity, stopbits, timeout=0.2) as ser:
            ser.reset_input_buffer()
            while True:
                chunk = ser.read(256)
                if chunk:
                    raw_f.write(chunk)
                    raw_f.flush()
                    buf += chunk
                    last_rx = time.time()

                # Tach dong theo CR/LF; neu may khong gui xuong dong, het 1s im lang thi coi la het ban tin
                lines = re.split(rb"[\r\n]+", buf)
                if buf and time.time() - last_rx > 1.0:
                    complete, buf = lines, b""
                else:
                    complete, buf = lines[:-1], lines[-1]

                for raw in complete:
                    text = raw.decode("latin-1", errors="replace").strip()
                    if not text:
                        continue
                    vals = parse_line(text)
                    now = datetime.now()
                    ts = now.strftime("%Y-%m-%d %H:%M:%S")
                    has_value = vals["transmission"] is not None or vals["haze"] is not None
                    if has_value:
                        store.write(now, vals["transmission"], vals["haze"])
                    print(f"[{ts}] {text!r}"
                          + (f"  ->  T={vals['transmission']}  H={vals['haze']}  C={vals['clarity']}"
                             if has_value else ""))
    except KeyboardInterrupt:
        print("\nDa dung.")
    except serial.SerialException as e:
        print(f"Loi cong COM: {e}")
    finally:
        raw_f.close()
        print(f"Ket qua: {os.path.abspath(store.path_for(datetime.now()))}")


# =====================================================================
#  CHE DO ONLINE: PC chu dong dieu khien may (lenh 4Fh)
#  Theo Interface Description trong Operating Instructions haze-gard plus:
#   - 9600 baud, 8N1, co dinh
#   - PC gui 4Fh ('O') -> may tra 4Fh 21h + 10 byte trang thai
#   - PC gui 04h (operate) -> may tra 04h 21h, do xong gui 66h ('f')
#     roi 3 gia tri, moi gia tri = 4 ky tu ASCII + 00h (vd "95,4"+00h)
#   - 77h ('w') co the xuat hien giua cac gia tri (doi tu cong haze sang clarity)
#   - PC gui 50h ('P') de thoat online mode
# =====================================================================
CMD_ONLINE, CMD_OPERATE, CMD_REFERENCE, CMD_END = 0x4F, 0x04, 0x06, 0x50
ACK, READOUT, SWITCH_PORT = 0x21, 0x66, 0x77

MODE_NAMES = {0: "T", 1: "H", 2: "C", 3: "T&H", 4: "T&C", 5: "H&C", 6: "T&H&C"}


def decode_status(st: bytes) -> str:
    if len(st) < 10:
        return f"Trang thai khong du 10 byte: {st.hex(' ')}"
    return (f"Mode do: {MODE_NAMES.get(st[1], hex(st[1]))} | "
            f"Continuous: {'Co' if st[4] else 'Khong'} | "
            f"Calibration hop le: {'Co' if st[5] else 'KHONG - can hieu chuan'} | "
            f"Firmware: {st[9]:02X}{st[8]:02X}")


def to_number(token: bytes):
    s = token.decode("latin-1").strip().replace(",", ".")
    try:
        return float(s)
    except ValueError:
        return None


class OnlineParser:
    """Bo tach du lieu: nhan luong byte, tra ve danh sach ket qua [T, H, C]."""

    def __init__(self, n_values=3, on_switch=None):
        self.n_values = n_values
        self.on_switch = on_switch
        self.in_frame = False
        self.token = b""
        self.values = []
        self.raw = []

    def feed(self, data: bytes):
        results = []
        for b in data:
            if not self.in_frame:
                if b == READOUT:                # 'f' -> bat dau ban tin ket qua
                    self.in_frame, self.token, self.values, self.raw = True, b"", [], []
                continue                        # bo qua ACK / echo lenh
            if b == SWITCH_PORT and not self.token:
                if self.on_switch:
                    self.on_switch()
                else:
                    print("   (may bao: chuyen mau sang cong CLARITY)")
                continue
            if b == 0x00:                       # ket thuc 1 gia tri
                self.raw.append(self.token.decode("latin-1"))
                self.values.append(to_number(self.token))
                self.token = b""
                if len(self.values) >= self.n_values:
                    results.append((self.values, self.raw))
                    self.in_frame = False
                continue
            self.token += bytes([b])
            if len(self.token) > 8:             # loi dong bo -> huy ban tin
                self.in_frame = False
        return results


def online(port, store, raw_log):
    import threading

    raw_f = open(raw_log, "ab")
    lock = threading.Lock()
    stop = threading.Event()

    ser = open_port(port, 9600, timeout=0.2)
    ser.dtr = True
    ser.rts = True
    ser.reset_input_buffer()

    # --- Vao online mode ---
    print(f"Gui lenh 4Fh toi {port} ...")
    ser.write(bytes([CMD_ONLINE]))
    resp = b""
    t0 = time.time()
    while time.time() - t0 < 3 and len(resp) < 12:
        resp += ser.read(12 - len(resp))
    raw_f.write(resp)
    if len(resp) >= 2 and resp[0] == CMD_ONLINE and resp[1] == ACK:
        print("May da vao ONLINE MODE.")
        print("  " + decode_status(resp[2:12]))
    elif not resp:
        print("KHONG co phan hoi. Kiem tra: may da bat, dung cong COM, thu dau chuyen null-modem.")
        ser.close(); raw_f.close()
        return
    else:
        print(f"Phan hoi la: {resp.hex(' ')}  (van tiep tuc, xem file {raw_log})")

    parser = OnlineParser()

    def reader():
        while not stop.is_set():
            try:
                chunk = ser.read(64)
            except serial.SerialException as e:
                print(f"Loi cong COM: {e}")
                stop.set()
                break
            if not chunk:
                continue
            raw_f.write(chunk); raw_f.flush()
            for vals, raw in parser.feed(chunk):
                now = datetime.now()
                ts = now.strftime("%Y-%m-%d %H:%M:%S")
                t, h, c = (vals + [None] * 3)[:3]
                with lock:
                    store.write(now, t, h)
                print(f"\n[{ts}]  T = {t}   Haze = {h}")
                print("> ", end="", flush=True)

    th = threading.Thread(target=reader, daemon=True)
    th.start()

    print("\nPhim lenh:  [Enter] = do   |  r = reference   |  q = thoat")
    print("(Ban van co the bam nut operate tren may hoac dap foot switch)\n")
    try:
        while not stop.is_set():
            cmd = input("> ").strip().lower()
            if cmd == "q":
                break
            elif cmd == "r":
                ser.write(bytes([CMD_REFERENCE]))
                print("Da gui lenh REFERENCE (06h).")
            else:
                ser.write(bytes([CMD_OPERATE]))
                print("Da gui lenh DO (04h), dang cho ket qua ...")
    except (KeyboardInterrupt, EOFError):
        pass
    finally:
        try:
            ser.write(bytes([CMD_END]))      # thoat online mode, mo khoa ban phim
            time.sleep(0.3)
        except serial.SerialException:
            pass
        stop.set()
        th.join(timeout=1)
        ser.close()
        raw_f.close()
        print(f"\nDa thoat online mode. Ket qua: {os.path.abspath(store.path_for(datetime.now()))}")


def main():
    ap = argparse.ArgumentParser(description="Nhan ket qua do tu BYK haze-gard plus qua RS232")
    ap.add_argument("--list", action="store_true", help="liet ke cong COM")
    ap.add_argument("--port", help="vd: COM3 (Windows) hoac /dev/ttyUSB0 (Linux)")
    ap.add_argument("--baud", type=int, default=9600)
    ap.add_argument("--parity", choices=["N", "E", "O"], default="N")
    ap.add_argument("--stopbits", type=int, choices=[1, 2], default=1)
    ap.add_argument("--sniff", action="store_true", help="tu do toc do baud")
    ap.add_argument("--online", action="store_true",
                    help="PC dieu khien may bang lenh 4Fh (khuyen dung)")
    ap.add_argument("--fix", metavar="CSV", help="doi file CSV cu sang dinh dang hazegarddata<yyyymmdd>.csv")
    ap.add_argument("--outdir", default=".", help="thu muc luu file CSV (mac dinh: thu muc hien tai)")
    # Cac cot chua ro y nghia: tam de trong, co the dien gia tri co dinh bang tuy chon
    ap.add_argument("--line", help="gia tri cot Line (vd VGI1)")
    ap.add_argument("--run-number", help="gia tri cot RunNumber")
    ap.add_argument("--run-minute", help="gia tri cot RunMinute")
    ap.add_argument("--position", help="gia tri cot Position")
    ap.add_argument("--t-offset", help="gia tri cot TOffsetApplied")
    ap.add_argument("--h-offset", help="gia tri cot HOffsetApplied")
    ap.add_argument("--raw", default="hazegard_raw.log")
    a = ap.parse_args()
    store = DailyCsv(a.outdir, line=a.line, run_number=a.run_number, run_minute=a.run_minute,
                     position=a.position, t_offset=a.t_offset, h_offset=a.h_offset)

    if a.fix:
        convert_old_csv(a.fix, store)
        return
    if a.list or not a.port:
        list_com_ports()
        if not a.port:
            print("\nChi dinh cong bang --port, vd: --port COM3")
        return
    if a.online:
        online(a.port, store, a.raw)
    elif a.sniff:
        sniff(a.port)
    else:
        listen(a.port, a.baud, a.parity, a.stopbits, store, a.raw)


if __name__ == "__main__":
    main()
