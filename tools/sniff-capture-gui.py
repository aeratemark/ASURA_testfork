#!/usr/bin/env python3
"""sniff-capture-gui.py - a window for tools/sniff-capture.py.

Double-click it. It asks for administrator rights once (the capture needs them: connecting to the client
is a write), then shows one window: pick a capture type, press Go, play, press Stop. Go starts the
sniffer immediately; the type dropdown sets the header filter and the output folder.

Everything the capture does is still `tools/sniff-capture.py`: this window builds its command line, runs
it, shows what it prints, and stops it the clean way (`--stop-file`), so the client's packet handlers are
put back exactly as they are after a console capture. The 32-bit re-exec, the sanitising and the file
format are that script's and are not repeated here.

When a capture ends, the window lists the title records it holds (`0x00F3` rank records, `0x00F6`
tracks, `0x00F4` display bindings): the rank names `config/Server/titles.json` is waiting for.

The Items tab runs `tools/item-id-probe.py` against the running client and writes the item records
(model id, model file id, type, flags and modifier words) to JSON. It is read-only.

Needs Python 3.8+ with tkinter (part of the standard Windows installer) and nothing from pip. Windows
only, like the capture itself.
"""

import json
import os
import queue
import struct
import subprocess
import sys
import threading
import time
from datetime import datetime
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
ROOT = TOOLS.parent
CAPTURE_SCRIPT = TOOLS / "sniff-capture.py"
ITEM_PROBE_SCRIPT = TOOLS / "item-id-probe.py"
FEATURE_DIR = ROOT / "recordings" / "client" / "feature"
SETTINGS_FILE = Path.home() / ".asura_sniff_gui.json"
IS_WIN = os.name == "nt"

# A capture type IS the folder under recordings/client/feature. The dropdown lists the folders that
# exist, and each one has the header filter that belongs in it; an unknown folder captures everything.
TYPE_HEADERS = {
    "cartography": "0x008A,0x008B,0x008C",
    "titles": "0x00F3,0x00F4,0x00F6",
    "movement": "0x0029,0x002D,0x0030,0x803D,0x803E,0x8028",
    "travel": "0x0199,0x0099,0x01A6,0x017C,0x0186,0x017D,0x019A,0x008B,0x008A",
    "maps": "0x008A,0x008B,0x008C,0x0099,0x0091,0x0199,0x0094",
    "chat": "0x005D,0x005E,0x0061,0x8064",
    "items": ("0x0020,0x0021,0x0022,0x006E,0x006F,0x013E,0x013F,0x0144,0x0145,"
              "0x0148,0x014B,0x014D,0x0152,0x015A,0x015E,0x0161,0x802C,0x8030,"
              "0x803F,0x804D,0x804F,0x8069,0x806C,0x8072,0x8075"),
    "guild": "0x011C,0x0120,0x0121,0x0127,0x012E,0x012F,0x0133",
    "multiplay": "0x0020,0x0021,0x0022,0x00B0,0x00B1,0x01DA,0x01DF,0x01E0,0x01E1",
}


def feature_types():
    """The dropdown: the folders that exist, plus Cartography (created on demand) and Everything."""
    names = []
    try:
        names = sorted(path.name for path in FEATURE_DIR.iterdir() if path.is_dir())
    except OSError:
        pass
    if "cartography" not in names:
        names.append("cartography")
        names.sort()
    return ["Everything"] + names

# The title family's StoC headers in the client build this repository targets (39112):
# docs/40-titles-system-research.md, section 3.
TITLE_RANK_DATA = 0x00F3
TITLE_RANK_DISPLAY = 0x00F4
TITLE_TRACK_INFO = 0x00F6


# ---------------------------------------------------------------------------------------------- #
# elevation
# ---------------------------------------------------------------------------------------------- #

def is_admin():
    if not IS_WIN:
        return False
    try:
        import ctypes
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception:  # noqa: BLE001 - treat "cannot tell" as "no"
        return False


def relaunch_elevated():
    """Start this script again as administrator (the UAC prompt). True when the prompt was accepted."""
    import ctypes
    interpreter = sys.executable
    # A console-less interpreter for the window, when there is one next to this one.
    windowless = Path(interpreter).with_name("pythonw.exe")
    if windowless.is_file():
        interpreter = str(windowless)
    result = ctypes.windll.shell32.ShellExecuteW(
        None, "runas", interpreter, '"%s"' % Path(__file__).resolve(), str(ROOT), 1
    )
    return result > 32


def console_interpreter():
    """python.exe for the capture: pythonw.exe has no stdout to read."""
    path = Path(sys.executable)
    if path.name.lower() == "pythonw.exe" and path.with_name("python.exe").is_file():
        return str(path.with_name("python.exe"))
    return sys.executable


# ---------------------------------------------------------------------------------------------- #
# settings
# ---------------------------------------------------------------------------------------------- #

def load_settings():
    try:
        return json.loads(SETTINGS_FILE.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def save_settings(values):
    try:
        SETTINGS_FILE.write_text(json.dumps(values, indent=2), encoding="utf-8")
    except OSError:
        pass


# ---------------------------------------------------------------------------------------------- #
# reading a finished capture
# ---------------------------------------------------------------------------------------------- #

def coded_units(data, offset):
    """The UTF-16 units of a client-struct string field, up to its terminator (8 units at most)."""
    units = []
    for (unit,) in struct.iter_unpack("<H", data[offset:offset + 16]):
        if unit == 0:
            break
        units.append(unit)
    return " ".join("%04X" % unit for unit in units)


def title_report(path):
    """Lines describing the title records in a capture file."""
    ranks, tracks, displays = {}, {}, set()
    try:
        handle = open(path, encoding="utf-8")
    except OSError as err:
        return ["could not open %s: %s" % (path, err)]
    with handle:
        for line in handle:
            try:
                record = json.loads(line)
            except ValueError:
                continue
            if record.get("direction") != "StoC" or not record.get("data"):
                continue
            header = record.get("header")
            try:
                data = bytes.fromhex(record["data"])
            except ValueError:
                continue
            # The sniffer records the client's own struct: a 4-byte header, then every field widened
            # to 4 bytes (tools/sniff-capture.py; docs/40 section 5.7).
            if header == TITLE_RANK_DATA and len(data) >= 32:
                rank_id, flags, value = struct.unpack_from("<3I", data, 4)
                ranks[rank_id] = (flags, value, coded_units(data, 16))
            elif header == TITLE_TRACK_INFO and len(data) >= 76:
                fields = struct.unpack_from("<10I", data, 4)
                tracks[fields[0]] = (fields, coded_units(data, 44), coded_units(data, 60))
            elif header == TITLE_RANK_DISPLAY and len(data) >= 12:
                displays.add(struct.unpack_from("<2I", data, 4))
    lines = [
        "Title records in %s:" % Path(path).name,
        "  %d rank record(s), %d track(s), %d display binding(s)"
        % (len(ranks), len(tracks), len(displays)),
    ]
    if tracks:
        lines.append("  tracks (title: points, current rank id -> next rank id):")
        for title_id in sorted(tracks):
            fields, _, _ = tracks[title_id]
            lines.append("    title %2d: %d point(s), rank %d -> %d"
                         % (title_id, fields[2], fields[3], fields[6]))
    if ranks:
        lines.append("  rank records (id, flags, value, name):")
        for rank_id in sorted(ranks):
            flags, value, name = ranks[rank_id]
            shown = "max" if value == 0xFFFFFFFF else str(value)
            lines.append("    %4d  %d  %-6s %s" % (rank_id, flags, shown, name))
    if not (ranks or tracks or displays):
        lines.append("  none - a title record arrives on a map load, or when a player in view shows a title")
    return lines


# ---------------------------------------------------------------------------------------------- #
# the window
# ---------------------------------------------------------------------------------------------- #

def run_window():
    import tkinter as tk
    from tkinter import filedialog, messagebox, ttk

    settings = load_settings()
    root = tk.Tk()
    root.title("ASURA - packet capture")
    root.geometry("860x680")

    lines = queue.Queue()
    state = {"proc": None, "stop_file": None, "out": None, "started": None, "item_proc": None}

    notebook = ttk.Notebook(root)
    notebook.pack(fill="both", expand=True)
    capture_tab = ttk.Frame(notebook)
    items_tab = ttk.Frame(notebook)
    notebook.add(capture_tab, text="Capture")
    notebook.add(items_tab, text="Items")

    form = ttk.Frame(capture_tab, padding=10)
    form.pack(fill="x")
    form.columnconfigure(1, weight=1)

    available = feature_types()
    default_type = settings.get("type")
    if default_type not in available:
        default_type = "cartography" if "cartography" in available else available[0]
    type_var = tk.StringVar(value=default_type)
    seconds_var = tk.StringVar(value=str(settings.get("seconds", 0)))
    dir_var = tk.StringVar(value=settings.get("dir", "both"))

    def type_folder():
        name = type_var.get()
        return "general" if name == "Everything" else name

    def default_out(kind=None):
        kind = kind or type_folder()
        stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
        return str(FEATURE_DIR / kind / ("%s-live-%s.jsonl" % (kind, stamp)))

    def default_items_out():
        stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
        return str(FEATURE_DIR / "items" / ("items-live-%s.json" % stamp))

    out_var = tk.StringVar(value=default_out())
    header_var = tk.StringVar(value=settings.get("header", TYPE_HEADERS.get(type_var.get(), "")))
    py4gw_var = tk.StringVar(value=settings.get("py4gw_root") or os.environ.get("PY4GW_STEALTH", ""))
    raw_var = tk.BooleanVar(value=False)

    def apply_type(_event=None):
        header_var.set(TYPE_HEADERS.get(type_var.get(), ""))
        out_var.set(default_out())

    row = 0
    ttk.Label(form, text="Capture type (folder)").grid(row=row, column=0, sticky="w")
    type_box = ttk.Combobox(form, textvariable=type_var, width=38, state="readonly",
                            values=available)
    type_box.grid(row=row, column=1, columnspan=2, sticky="w")
    type_box.bind("<<ComboboxSelected>>", apply_type)
    row += 1
    ttk.Label(form, text="Seconds (0 = until Stop)").grid(row=row, column=0, sticky="w")
    ttk.Entry(form, textvariable=seconds_var, width=10).grid(row=row, column=1, sticky="w")
    row += 1
    ttk.Label(form, text="Direction").grid(row=row, column=0, sticky="w")
    ttk.Combobox(form, textvariable=dir_var, values=("both", "StoC", "CToS"), width=8,
                 state="readonly").grid(row=row, column=1, sticky="w")
    row += 1
    ttk.Label(form, text="Save to").grid(row=row, column=0, sticky="w")
    ttk.Entry(form, textvariable=out_var).grid(row=row, column=1, sticky="ew")

    def browse_out():
        chosen = filedialog.asksaveasfilename(
            initialdir=str(FEATURE_DIR / type_folder()), defaultextension=".jsonl",
            filetypes=[("Capture", "*.jsonl"), ("All files", "*.*")])
        if chosen:
            out_var.set(chosen)

    ttk.Button(form, text="Browse...", command=browse_out).grid(row=row, column=2, padx=4)
    row += 1
    ttk.Label(form, text="Only these headers").grid(row=row, column=0, sticky="w")
    ttk.Entry(form, textvariable=header_var).grid(row=row, column=1, sticky="ew")
    ttk.Label(form, text="empty = everything (best)").grid(row=row, column=2, sticky="w")
    row += 1
    ttk.Label(form, text="Py4GW Stealth folder").grid(row=row, column=0, sticky="w")
    ttk.Entry(form, textvariable=py4gw_var).grid(row=row, column=1, sticky="ew")

    def browse_py4gw():
        chosen = filedialog.askdirectory(title="The folder that contains py4gw")
        if chosen:
            py4gw_var.set(chosen)

    ttk.Button(form, text="Browse...", command=browse_py4gw).grid(row=row, column=2, padx=4)
    row += 1
    ttk.Checkbutton(form, text="Raw (keeps player names - never commit a raw capture)",
                    variable=raw_var).grid(row=row, column=1, sticky="w")

    buttons = ttk.Frame(capture_tab, padding=(10, 0))
    buttons.pack(fill="x")
    start_button = ttk.Button(buttons, text="Go")
    stop_button = ttk.Button(buttons, text="Stop", state="disabled")
    report_button = ttk.Button(buttons, text="Show title records of a file...")
    start_button.pack(side="left")
    stop_button.pack(side="left", padx=6)
    report_button.pack(side="right")
    status_var = tk.StringVar(value="Ready." if is_admin() or not IS_WIN
                              else "Not running as administrator: the capture will refuse.")
    ttk.Label(capture_tab, textvariable=status_var, padding=(10, 6)).pack(fill="x")

    items_form = ttk.Frame(items_tab, padding=10)
    items_form.pack(fill="x")
    items_form.columnconfigure(1, weight=1)
    items_out_var = tk.StringVar(value=default_items_out())
    items_status_var = tk.StringVar(
        value="Read the running client's item records with Py4GW (read-only)."
    )
    ttk.Label(items_form, text="Py4GW Stealth folder").grid(row=0, column=0, sticky="w")
    ttk.Entry(items_form, textvariable=py4gw_var).grid(row=0, column=1, sticky="ew")
    ttk.Button(items_form, text="Browse...", command=browse_py4gw).grid(row=0, column=2, padx=4)
    ttk.Label(items_form, text="Write item JSON to").grid(row=1, column=0, sticky="w")
    ttk.Entry(items_form, textvariable=items_out_var).grid(row=1, column=1, sticky="ew")
    read_items_button = ttk.Button(items_form, text="Read items from the running client")
    read_items_button.grid(row=2, column=1, sticky="w", pady=(8, 0))
    ttk.Label(items_tab, textvariable=items_status_var, padding=(10, 6)).pack(fill="x")

    log_frame = ttk.Frame(root)
    log_frame.pack(fill="both", expand=True)
    log = tk.Text(log_frame, wrap="none", font=("Consolas", 9))
    scroll = ttk.Scrollbar(log_frame, command=log.yview)
    log.configure(yscrollcommand=scroll.set)
    scroll.pack(side="right", fill="y")
    log.pack(fill="both", expand=True, padx=(10, 0), pady=(0, 10))

    def show(text):
        log.insert("end", text + "\n")
        log.see("end")

    def reader(proc):
        for raw in proc.stdout:
            lines.put(raw.rstrip("\r\n"))
        lines.put(("done", proc.wait()))

    def start():
        if state["proc"] is not None:
            return
        try:
            seconds = float(seconds_var.get() or "0")
        except ValueError:
            messagebox.showerror("Seconds", "Seconds must be a number (0 = until Stop).")
            return
        out = Path(out_var.get().strip() or default_out())
        out.parent.mkdir(parents=True, exist_ok=True)
        stop_file = Path(os.environ.get("TEMP", str(TOOLS))) / ("asura-capture-stop-%d" % os.getpid())
        try:
            stop_file.unlink()
        except OSError:
            pass
        command = [console_interpreter(), "-u", str(CAPTURE_SCRIPT),
                   "--seconds", str(seconds), "--dir", dir_var.get(),
                   "--out", str(out), "--stop-file", str(stop_file)]
        if header_var.get().strip():
            command += ["--header", header_var.get().strip()]
        if py4gw_var.get().strip():
            command += ["--py4gw-root", py4gw_var.get().strip()]
        if raw_var.get():
            command.append("--raw")
        save_settings({"type": type_var.get(), "seconds": seconds, "dir": dir_var.get(),
                       "header": header_var.get().strip(),
                       "py4gw_root": py4gw_var.get().strip()})
        env = dict(os.environ, PYTHONUNBUFFERED="1")
        flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
        show("$ " + " ".join(command))
        try:
            proc = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    text=True, encoding="utf-8", errors="replace", env=env,
                                    cwd=str(ROOT), creationflags=flags)
        except OSError as err:
            show("could not start the capture: %s" % err)
            return
        state.update(proc=proc, stop_file=stop_file, out=out, started=time.time())
        threading.Thread(target=reader, args=(proc,), daemon=True).start()
        start_button.configure(state="disabled")
        stop_button.configure(state="normal")
        status_var.set("Capturing to %s - play now; press Stop when done." % out.name)

    def stop():
        if state["stop_file"] is not None:
            try:
                Path(state["stop_file"]).write_text("stop", encoding="utf-8")
            except OSError as err:
                show("could not ask the capture to stop: %s" % err)
            status_var.set("Stopping - the client's handlers are being put back...")
            stop_button.configure(state="disabled")

    def finished(code):
        out = state["out"]
        state.update(proc=None)
        try:
            Path(state["stop_file"]).unlink()
        except (OSError, TypeError):
            pass
        start_button.configure(state="normal")
        stop_button.configure(state="disabled")
        meaning = {0: "done", 2: "refused (administrator, or 32-bit Python missing - see above)",
                   3: "no Guild Wars client found, or Py4GW Stealth not found", 4: "capture failed",
                   130: "stopped"}
        status_var.set("Capture %s (exit %s)." % (meaning.get(code, "ended"), code))
        if out is not None and Path(out).is_file():
            for line in title_report(out):
                show(line)
        out_var.set(default_out())

    def read_items():
        if state["item_proc"] is not None:
            return
        out = Path(items_out_var.get().strip() or default_items_out())
        out.parent.mkdir(parents=True, exist_ok=True)
        command = [console_interpreter(), "-u", str(ITEM_PROBE_SCRIPT), "--out", str(out)]
        if py4gw_var.get().strip():
            command += ["--py4gw", py4gw_var.get().strip()]
        show("$ " + " ".join(command))
        try:
            proc = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                env=dict(os.environ, PYTHONUNBUFFERED="1"),
                cwd=str(ROOT),
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
        except OSError as err:
            show("could not start the item probe: %s" % err)
            return
        state["item_proc"] = proc

        def item_reader():
            for raw in proc.stdout:
                lines.put(raw.rstrip("\r\n"))
            lines.put(("items-done", proc.wait(), str(out)))

        threading.Thread(target=item_reader, daemon=True).start()
        read_items_button.configure(state="disabled")
        items_status_var.set("Reading the running client's item records...")

    def finished_items(code, out):
        state["item_proc"] = None
        read_items_button.configure(state="normal")
        meaning = {
            0: "done",
            2: "refused (administrator, 32-bit Python, or Py4GW missing - see above)",
            3: "no Guild Wars client found, or the read was refused",
            4: "probe failed",
        }
        items_status_var.set("Item read %s (exit %s)." % (meaning.get(code, "ended"), code))
        path = Path(out)
        if path.is_file():
            try:
                report = json.loads(path.read_text(encoding="utf-8"))
                count = len(report.get("items", []))
                show("items: %d written to %s" % (count, path))
            except (OSError, ValueError) as err:
                show("could not read %s: %s" % (path, err))
        items_out_var.set(default_items_out())

    def report_file():
        chosen = filedialog.askopenfilename(
            initialdir=str(FEATURE_DIR), filetypes=[("Capture", "*.jsonl"), ("All files", "*.*")])
        if chosen:
            for line in title_report(chosen):
                show(line)

    def pump():
        try:
            while True:
                item = lines.get_nowait()
                if isinstance(item, tuple) and item and item[0] == "done":
                    finished(item[1])
                elif isinstance(item, tuple) and item and item[0] == "items-done":
                    finished_items(item[1], item[2])
                else:
                    show(item)
        except queue.Empty:
            pass
        if state["proc"] is not None and state["started"] is not None:
            elapsed = int(time.time() - state["started"])
            if "Capturing" in status_var.get():
                status_var.set("Capturing to %s - %d s - press Stop when done."
                               % (Path(state["out"]).name, elapsed))
        root.after(200, pump)

    def on_close():
        if state["proc"] is not None:
            if not messagebox.askyesno("Capture running",
                                       "A capture is running. Stop it cleanly and close?"):
                return
            stop()
            deadline = time.time() + 15
            while state["proc"] is not None and state["proc"].poll() is None and time.time() < deadline:
                root.update()
                time.sleep(0.1)
        root.destroy()

    start_button.configure(command=start)
    stop_button.configure(command=stop)
    report_button.configure(command=report_file)
    read_items_button.configure(command=read_items)
    root.protocol("WM_DELETE_WINDOW", on_close)
    show("Steps: start Guild Wars to character select, press Go, then enter the map. Press Stop "
         "when the load is finished.")
    show("Go starts immediately. The capture type sets the header filter and the folder under "
         "recordings/client/feature/.")
    root.after(200, pump)
    root.mainloop()


def main():
    if "--report" in sys.argv:
        for path in sys.argv[sys.argv.index("--report") + 1:]:
            print("\n".join(title_report(path)))
        return 0
    if IS_WIN and not is_admin():
        if relaunch_elevated():
            return 0
        # The prompt was declined: open anyway and say why a capture will refuse.
    run_window()
    return 0


if __name__ == "__main__":
    sys.exit(main())
