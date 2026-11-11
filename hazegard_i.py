"""
Ket noi may BYK haze-gard i (USB / mang) qua BYKCommunication.dll (32-bit) cua BYK-Gardner.

 - Giao thuc (lenh XML, ma lenh, ma su kien) lay theo phan mem goc Client_HazeGard.exe.
 - Phai chay bang Python 32-bit (DLL la 32-bit) va de BYKCommunication.dll CUNG THU MUC voi chuong trinh.
 - Dung chung hang doi su kien voi class Instrument (may plus) trong hazegard_gui.py:
       ("connected", {...}), ("result", (T, H, chuoi_goc)), ("error", "thong bao")
"""

import ctypes
import os
import sys
import threading
import time
import xml.etree.ElementTree as ET
from ctypes import byref, c_int, c_uint, create_string_buffer

# --- thong so khoi tao DLL (giong phan mem goc) ---
INIT_NAME = b"BykGardnerInstrumentUnits"
INIT_KEY = b"BYKUnits81!654"

# --- loai su kien tra ve tu waitForEventDataEx (mask = 7) ---
EV_ADD, EV_REMOVE, EV_STATUS = 1, 2, 4

# --- ma trang thai may (event "status") ---
ST_BUSY = -2147483237
ST_MEASURE_END = -2147483236      # phan mem goc doc ket qua (lenh 403) khi nhan ma nay
ST_ONLINE_END = -2147483235
ST_PORT_HAZE = -2147483234
ST_PORT_CLARITY = -2147483233
ST_NO_PORT = -2147483360
EVENT_IDS = (ST_BUSY, ST_MEASURE_END, ST_ONLINE_END, ST_PORT_HAZE, ST_PORT_CLARITY, ST_NO_PORT)

# --- ma lenh XML ---
CMD_DEVICE_INFO = 4
CMD_START_ONLINE = 400
CMD_STOP_ONLINE = 401
CMD_MEASURE_ONLINE = 402
CMD_READ_ONLINE = 403

# --- ma "scale" trong ket qua do ---
SCALE_TRANSMISSION_C = 500
SCALE_HAZE_C = 501

BUF = 5 * 1024 * 1024


class DllError(Exception):
    pass


class BykDll:
    """Boc cac ham export dang stdcall decorated: _ten@N."""

    SIZES = {
        "init": 8, "deInit": 0, "initConnection": 4, "deinitConnection": 4,
        "Send": 12, "SendReceive": 24, "listAllInstruments": 12,
        "registerEventMessage": 4, "waitForEventDataEx": 32, "cancelWaitForEvent": 0,
        "enableTCPIPConnections": 0, "setIPAddressRange": 4,
    }

    def __init__(self, path):
        self.dll = ctypes.WinDLL(path)          # chi co tren Windows
        self.f = {n: self.dll[f"_{n}@{s}"] for n, s in self.SIZES.items()}

    def __getattr__(self, name):
        try:
            return self.__dict__["f"][name]
        except KeyError:
            raise AttributeError(name)


def find_dll(app_dir):
    for d in (app_dir, os.getcwd()):
        p = os.path.join(d, "BYKCommunication.dll")
        if os.path.exists(p):
            return p
    return None


def parse_measure(xml_text):
    """Tra ve (T, H, thoi_gian) tu ket qua lenh 403; nem DllError neu may bao loi."""
    root = ET.fromstring(xml_text)
    err = root.find("Error")
    if err is not None:
        raise DllError("Máy báo lỗi: " + (err.text or err.attrib.get("Text", "") or ET.tostring(err, "unicode")))
    vals = {}
    for sv in root.iter("ScaleValue"):
        try:
            vals[int(sv.attrib["Id"])] = float(sv.attrib["Value"].replace(",", "."))
        except (KeyError, ValueError):
            continue
    mv = root.find(".//MeasureValue")
    ts = mv.attrib.get("TimeStamp") if mv is not None else None
    return vals.get(SCALE_TRANSMISSION_C), vals.get(SCALE_HAZE_C), ts


def parse_serial(xml_text):
    root = ET.fromstring(xml_text)
    for el in root.iter():
        if el.attrib.get("Name") == "SerialNumber":
            return el.attrib.get("Value", "")
    return ""


class InstrumentI(threading.Thread):
    """May haze-gard i. Giao dien giong Instrument (plus): .online, .send(), .stop()."""

    def __init__(self, events, app_dir, raw_log, ip=""):
        super().__init__(daemon=True)
        self.q = events
        self.app_dir = app_dir
        self.raw_log = raw_log
        self.ip = ip.strip()
        self.stop_evt = threading.Event()
        self.online = False
        self.identifier = None
        self.dll = None
        self._raw = None

    # ---------- tien ich ----------
    def log(self, s):
        if self._raw:
            self._raw.write(s.encode("utf-8", "replace") + b"\n")
            self._raw.flush()

    def command(self, ident, cmd, params="", receive=True):
        xml = f'<?xml version="1.0" encoding="utf-8"?><Command Id="{cmd}">{params}</Command>'
        data = xml.encode("utf-8")
        self.log(f">> {xml}")
        if receive:
            out = create_string_buffer(BUF)
            n = c_int(0)
            rc = self.dll.SendReceive(ident.encode(), data, len(data), out, BUF, byref(n))
            if rc:
                raise DllError(f"BykCommunicationDLL Error: {rc}")
            text = out.raw[:n.value].decode("utf-8", "replace")
            self.log(f"<< {text}")
            return text
        rc = self.dll.Send(ident.encode(), data, len(data))
        if rc:
            raise DllError(f"BykCommunicationDLL Error: {rc}")
        return ""

    def send(self, code):
        return False            # may i do truc tiep tren may, khong kich tu PC

    def stop(self):
        self.stop_evt.set()
        try:
            if self.dll:
                self.dll.cancelWaitForEvent()
        except Exception:
            pass

    # ---------- luong chinh ----------
    def run(self):
        try:
            self._raw = open(self.raw_log, "ab")
        except OSError:
            self._raw = None
        try:
            path = find_dll(self.app_dir)
            if not path:
                self.q.put(("error", "Không thấy BYKCommunication.dll cạnh chương trình."))
                return
            if sys.maxsize > 2**32:
                self.q.put(("error", "Máy i cần Python/chương trình 32-bit (BYKCommunication.dll là bản 32-bit)."))
                return
            try:
                self.dll = BykDll(path)
            except (OSError, KeyError) as e:
                self.q.put(("error", f"Không nạp được BYKCommunication.dll: {e}"))
                return
            self._session()
        except Exception as e:                  # noqa: BLE001
            self.q.put(("error", f"Lỗi máy i: {e}"))
        finally:
            self._cleanup()

    def _session(self):
        d = self.dll
        d.enableTCPIPConnections()
        if self.ip:
            parts = [int(x) for x in self.ip.split(".")]
            d.setIPAddressRange(bytes(parts))
        if d.init(INIT_NAME, INIT_KEY):
            self.q.put(("error", "BykCommunication.dll đang được chương trình khác dùng "
                                 "(đóng Client_HazeGard.exe rồi thử lại)."))
            return
        for ev in EVENT_IDS:
            d.registerEventMessage(ev)

        # liet ke may dang co
        out = create_string_buffer(BUF)
        n = c_int(0)
        d.listAllInstruments(out, BUF, byref(n))
        text = out.raw[:n.value].decode("utf-8", "replace")
        self.log(f"listAllInstruments: {text}")
        idents = []
        try:
            root = ET.fromstring(text)
            idents = [e.attrib["Identifier"] for e in root.iter() if "Identifier" in e.attrib]
        except ET.ParseError:
            pass
        for i in idents:
            self._add(i)

        # vong nhan su kien
        while not self.stop_evt.is_set():
            ev, code, dlen = c_int(0), c_int(0), c_int(0)
            ident = create_string_buffer(60)
            data = create_string_buffer(5125)
            d.waitForEventDataEx(7, byref(ev), ident, 60, byref(code), data, 5125, byref(dlen))
            if self.stop_evt.is_set():
                break
            who = ident.value.decode("ascii", "replace")
            if ev.value == EV_ADD:
                self._add(who)
            elif ev.value == EV_REMOVE:
                if who == self.identifier:
                    self.online = False
                    self.q.put(("error", "Máy i bị rút / mất kết nối."))
                    self.identifier = None
            elif ev.value == EV_STATUS and who == self.identifier:
                self._status(code.value)
            else:
                time.sleep(0.05)

    def _add(self, ident):
        if self.identifier:                      # chi dung may dau tien
            return
        self.dll.initConnection(ident.encode())
        serial = ""
        try:
            serial = parse_serial(self.command(ident, CMD_DEVICE_INFO))
        except Exception as e:                   # noqa: BLE001
            self.log(f"device info loi: {e}")
        self.identifier = ident
        params = ('<Parameters><ParameterBool Name="Transmission" Value="true" />'
                  '<ParameterBool Name="Haze" Value="true" />'
                  '<ParameterBool Name="Clarity" Value="false" />'
                  '<ParameterBool Name="Continuous" Value="false" />'
                  '<ParameterInt32 Name="Continuous_Delay" Value="0" /></Parameters>')
        self.command(ident, CMD_START_ONLINE, params)
        self.online = True
        self.q.put(("connected", {"online": True, "status": None, "model": "i",
                                  "serial": serial, "identifier": ident}))

    def _status(self, code):
        if code == ST_MEASURE_END:               # do xong -> doc ket qua
            try:
                t, h, ts = parse_measure(self.command(self.identifier, CMD_READ_ONLINE))
                self.q.put(("result", (t, h, f"T={t} H={h} {ts or ''}".strip())))
            except Exception as e:               # noqa: BLE001
                self.q.put(("error", f"Đọc kết quả lỗi: {e}"))
        elif code == ST_ONLINE_END:
            self.online = False
        elif code == ST_PORT_CLARITY:
            self.q.put(("port", None))

    def _cleanup(self):
        try:
            if self.dll and self.identifier and self.online:
                self.command(self.identifier, CMD_STOP_ONLINE)
        except Exception:
            pass
        try:
            if self.dll:
                self.dll.deInit()
        except Exception:
            pass
        if self._raw:
            self._raw.close()
