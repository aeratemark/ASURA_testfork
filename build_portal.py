#!/usr/bin/env python3
"""build_portal.py - build portal.dll for the patched Guild Wars client, in one window.

The patched client (Gw.custom.exe) needs a portal.dll next to it. It replaces the client's portal login
and hands the client the three test accounts that ASURA seeds (user1/user2/user3@example.com).

This tool does what client-patcher/README.md describes by hand:
  1. downloads portal.c (public domain, OpenTyria project) or uses a portal.c you pick,
  2. finds the Visual Studio C++ build tools and opens their 32-bit (x86) environment,
  3. compiles a 32-bit DLL with:  cl /nologo /LD /O2 /MT portal.c /Fe:portal.dll
  4. checks the result is 32-bit and exports the portal functions,
  5. copies portal.dll into your client folder (an old portal.dll becomes portal.dll.bak).

Needs Windows, Python 3.8+ (nothing from pip) and the Visual Studio C++ build tools.

    python build_portal.py                           open the window
    python build_portal.py --client "F:\\GW\\GW1"     build and copy, no window
    python build_portal.py --source my_portal.c      use your own portal.c instead of downloading
"""

import argparse
import hashlib
import os
import queue
import re
import shutil
import subprocess
import sys
import threading
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent
BUILD_DIR = ROOT / "portal-build"
LAST_FOLDER = BUILD_DIR / "last_client_folder.txt"
SOURCE_URL = "https://raw.githubusercontent.com/ldufr/OpenTyria/master/portal/portal.c"
IS_WIN = os.name == "nt"

REQUIRED_EXPORTS = [b"PortalInitialize", b"PortalLogin", b"PortalRegisterNotify",
                    b"PortalRequestGameToken", b"PortalGetUserId"]

TOOLS_HINT = ("The Visual Studio C++ build tools are not installed. In PowerShell run: "
              "winget install Microsoft.VisualStudio.2022.BuildTools --override "
              "\"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended\" "
              "(or get 'Build Tools for Visual Studio' from https://visualstudio.microsoft.com/visual-cpp-build-tools/ "
              "and tick 'Desktop development with C++'), restart, then try again.")


# ---------------------------------------------------------------------------------------------
# Steps
# ---------------------------------------------------------------------------------------------

def download_source(dest, emit, url=None):
    """Download portal.c to dest. Returns True on success."""
    url = url or SOURCE_URL
    emit("Downloading " + url)
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "build_portal.py"})
        with urllib.request.urlopen(request, timeout=30) as reply:
            data = reply.read(1000001)
    except Exception as err:  # network errors come in many types
        emit("Download failed: %s" % err)
        emit("You can download the file in a browser, save it as portal.c, and pick it with 'Use my own portal.c'.")
        return False
    if len(data) > 1000000 or b"PortalInitialize" not in data or b"DllMain" not in data:
        emit("The downloaded file does not look like portal.c; not using it.")
        return False
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)
    emit("Saved %s (%d bytes, sha256 %s)" % (dest, len(data), hashlib.sha256(data).hexdigest()[:16]))
    return True


def find_vcvars32():
    """Path of vcvars32.bat from the newest Visual Studio with the C++ tools, or None."""
    base = os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")
    vswhere = Path(base) / "Microsoft Visual Studio" / "Installer" / "vswhere.exe"
    if not vswhere.is_file():
        return None
    try:
        done = subprocess.run(
            [str(vswhere), "-latest", "-products", "*", "-requires",
             "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"],
            capture_output=True, text=True, timeout=30, creationflags=0x08000000)
    except (OSError, subprocess.TimeoutExpired):
        return None
    install = done.stdout.strip().splitlines()
    if not install:
        return None
    candidate = Path(install[0].strip()) / "VC" / "Auxiliary" / "Build" / "vcvars32.bat"
    return candidate if candidate.is_file() else None


def compile_dll(vcvars, emit):
    """Run cl in the x86 environment inside BUILD_DIR. Returns True when portal.dll was written."""
    out = BUILD_DIR / "portal.dll"
    if out.exists():
        out.unlink()
    line = 'cmd /s /c ""%s" >nul && cl /nologo /LD /O2 /MT portal.c /Fe:portal.dll"' % vcvars
    emit("> cl /nologo /LD /O2 /MT portal.c /Fe:portal.dll   (x86 environment from %s)" % vcvars.name)
    try:
        proc = subprocess.Popen(line, cwd=str(BUILD_DIR), stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                stdin=subprocess.DEVNULL, text=True, encoding="utf-8", errors="replace",
                                creationflags=0x08000000)
    except OSError as err:
        emit("Cannot run the compiler: %s" % err)
        return False
    for text in proc.stdout:
        emit(text.rstrip("\r\n"))
    code = proc.wait()
    if code != 0 or not out.is_file():
        emit("The compiler failed (exit code %s). The lines above say why." % code)
        return False
    return True


def check_dll(path, emit):
    """The DLL must be 32-bit (the client is) and export the portal functions."""
    data = Path(path).read_bytes()
    if len(data) < 0x100 or data[:2] != b"MZ":
        emit("Not a Windows program file.")
        return False
    offset = int.from_bytes(data[0x3C:0x40], "little")
    if data[offset:offset + 4] != b"PE\0\0":
        emit("Not a valid PE file.")
        return False
    machine = int.from_bytes(data[offset + 4:offset + 6], "little")
    if machine != 0x14C:
        emit("The DLL is not 32-bit (machine 0x%X). The client is 32-bit, so this would not load." % machine)
        return False
    missing = [name.decode() for name in REQUIRED_EXPORTS if name not in data]
    if missing:
        emit("The DLL is missing these functions: " + ", ".join(missing))
        return False
    emit("Checked: 32-bit DLL, all portal functions present.")
    return True


def install_dll(client_folder, emit):
    folder = Path(client_folder)
    if not folder.is_dir():
        emit("The client folder does not exist: %s" % folder)
        return False
    if not (folder / "Gw.custom.exe").is_file():
        emit("Warning: there is no Gw.custom.exe in that folder. Copying anyway.")
    target = folder / "portal.dll"
    try:
        if target.exists():
            shutil.copy2(target, folder / "portal.dll.bak")
            emit("Kept the old file as portal.dll.bak")
        shutil.copy2(BUILD_DIR / "portal.dll", target)
    except OSError as err:
        emit("Cannot write to %s: %s" % (folder, err))
        emit("Close the game if it is running, or pick a folder you may write to.")
        return False
    emit("Copied portal.dll to %s" % target)
    try:
        LAST_FOLDER.write_text(str(folder), encoding="utf-8")
    except OSError:
        pass
    return True


def build(emit, client_folder=None, own_source=None):
    """The whole sequence. Returns True when portal.dll is built, checked and (if asked) installed."""
    if not IS_WIN:
        emit("Building the DLL needs Windows and the Visual Studio C++ tools. Run this on your Windows PC.")
        return False
    emit("== 1/5 Getting portal.c")
    BUILD_DIR.mkdir(exist_ok=True)
    target = BUILD_DIR / "portal.c"
    if own_source:
        source = Path(own_source)
        if not source.is_file():
            emit("File not found: %s" % source)
            return False
        if source.resolve() != target.resolve():
            shutil.copy2(source, target)
        emit("Using %s" % source)
    elif not download_source(target, emit):
        return False
    emit("== 2/5 Finding the Visual Studio C++ tools")
    vcvars = find_vcvars32()
    if not vcvars:
        emit(TOOLS_HINT)
        return False
    emit("Found %s" % vcvars)
    emit("== 3/5 Compiling (32-bit)")
    if not compile_dll(vcvars, emit):
        return False
    emit("== 4/5 Checking the DLL")
    if not check_dll(BUILD_DIR / "portal.dll", emit):
        return False
    if client_folder:
        emit("== 5/5 Copying to the client folder")
        if not install_dll(client_folder, emit):
            return False
    else:
        emit("== 5/5 No client folder given; the DLL is at %s" % (BUILD_DIR / "portal.dll"))
    emit("== Done. Start the server, then launch: Gw.custom.exe -authsrv 127.0.0.1 -portal 127.0.0.1")
    return True


# ---------------------------------------------------------------------------------------------
# The window
# ---------------------------------------------------------------------------------------------

def run_gui():
    import tkinter as tk
    import tkinter.font as tkfont
    from tkinter import filedialog, ttk

    class App:
        def __init__(self, root):
            self.root = root
            self.events = queue.Queue()
            self.busy = False
            root.title("Build portal.dll")
            root.geometry("860x600")
            root.minsize(680, 460)
            try:
                ttk.Style().theme_use("clam")
            except tk.TclError:
                pass
            base = tkfont.nametofont("TkDefaultFont")
            big = base.copy()
            big.configure(size=base.cget("size") + 5, weight="bold")

            head = ttk.Frame(root, padding=(14, 10, 14, 0))
            head.pack(fill="x")
            ttk.Label(head, text="Build portal.dll", font=big).pack(side="left")
            self.status = ttk.Label(head, text="")
            self.status.pack(side="right")
            ttk.Label(root, padding=(14, 4, 14, 8), wraplength=820, justify="left", foreground="#555", text=(
                "The patched client needs portal.dll next to Gw.custom.exe. This builds it for you: it downloads "
                "portal.c, compiles a 32-bit DLL with the Visual Studio C++ tools, checks it and copies it into "
                "your client folder.")).pack(fill="x")

            form = ttk.LabelFrame(root, text="Settings", padding=10)
            form.pack(fill="x", padx=12)
            ttk.Label(form, text="Client folder (the one with Gw.custom.exe):").grid(row=0, column=0, sticky="w")
            self.folder = tk.StringVar()
            ttk.Entry(form, textvariable=self.folder, width=60).grid(row=1, column=0, sticky="ew", pady=(2, 8))
            ttk.Button(form, text="Browse...", command=self.pick_folder).grid(row=1, column=1, padx=(8, 0))
            self.own_var = tk.BooleanVar(value=False)
            ttk.Checkbutton(form, text="Use my own portal.c instead of downloading it", variable=self.own_var,
                            command=self.toggle_own).grid(row=2, column=0, sticky="w")
            self.own_path = tk.StringVar()
            self.own_entry = ttk.Entry(form, textvariable=self.own_path, width=60, state="disabled")
            self.own_entry.grid(row=3, column=0, sticky="ew", pady=(2, 0))
            self.own_button = ttk.Button(form, text="Browse...", command=self.pick_source, state="disabled")
            self.own_button.grid(row=3, column=1, padx=(8, 0))
            form.columnconfigure(0, weight=1)

            row = ttk.Frame(root, padding=(12, 10, 12, 4))
            row.pack(fill="x")
            self.build_button = ttk.Button(row, text="Build portal.dll", command=self.on_build)
            self.build_button.pack(side="left", ipadx=14, ipady=5)
            ttk.Button(row, text="Open build folder", command=self.open_build).pack(side="left", padx=8)
            ttk.Button(row, text="Open client folder", command=self.open_client).pack(side="left")

            frame = ttk.Frame(root, padding=(12, 4, 12, 12))
            frame.pack(fill="both", expand=True)
            self.output = tk.Text(frame, wrap="word", bg="#1e1f22", fg="#d7d9dc", relief="flat",
                                  font=tkfont.nametofont("TkFixedFont"), state="disabled")
            scroll = ttk.Scrollbar(frame, command=self.output.yview)
            self.output.configure(yscrollcommand=scroll.set)
            scroll.pack(side="right", fill="y")
            self.output.pack(side="left", fill="both", expand=True)
            self.output.tag_configure("head", foreground="#7ee787")
            self.output.tag_configure("err", foreground="#ff7b72")
            self.output.tag_configure("cmd", foreground="#79c0ff")

            try:
                if LAST_FOLDER.is_file():
                    self.folder.set(LAST_FOLDER.read_text(encoding="utf-8").strip())
            except OSError:
                pass
            if not IS_WIN:
                self.emit("This tool builds a Windows DLL, so it only works on Windows.")
            root.after(80, self.pump)

        def emit(self, text):
            self.events.put(("line", text))

        def pump(self):
            try:
                while True:
                    kind, payload = self.events.get_nowait()
                    if kind == "line":
                        tag = ()
                        if payload.startswith("== "):
                            tag = ("head",)
                        elif payload.startswith("> "):
                            tag = ("cmd",)
                        elif re.search(r"\b(error|failed|cannot|not found|not installed|missing)\b", payload, re.I):
                            tag = ("err",)
                        self.output.configure(state="normal")
                        self.output.insert("end", payload + "\n", tag)
                        self.output.configure(state="disabled")
                        self.output.see("end")
                    elif kind == "done":
                        self.busy = False
                        self.build_button.state(["!disabled"])
                        self.status.configure(text="Built OK" if payload else "Failed - see the output",
                                              foreground="#2e9e4f" if payload else "#d64545")
            except queue.Empty:
                pass
            self.root.after(80, self.pump)

        def pick_folder(self):
            chosen = filedialog.askdirectory(title="Pick the folder that contains Gw.custom.exe")
            if chosen:
                self.folder.set(chosen)

        def pick_source(self):
            chosen = filedialog.askopenfilename(title="Pick portal.c", filetypes=[("C source", "*.c"), ("All", "*.*")])
            if chosen:
                self.own_path.set(chosen)

        def toggle_own(self):
            state = "normal" if self.own_var.get() else "disabled"
            self.own_entry.configure(state=state)
            self.own_button.configure(state=state)

        def open_build(self):
            BUILD_DIR.mkdir(exist_ok=True)
            self.open_folder(BUILD_DIR)

        def open_client(self):
            folder = self.folder.get().strip()
            if folder and Path(folder).is_dir():
                self.open_folder(Path(folder))

        def open_folder(self, path):
            try:
                if IS_WIN:
                    os.startfile(str(path))  # noqa
                else:
                    subprocess.Popen(["xdg-open", str(path)])
            except OSError:
                pass

        def on_build(self):
            if self.busy:
                return
            folder = self.folder.get().strip() or None
            own = self.own_path.get().strip() if self.own_var.get() else None
            if self.own_var.get() and not own:
                self.emit("Pick your portal.c first, or untick the box.")
                return
            if not folder:
                self.emit("No client folder chosen: the DLL will only be built into %s" % BUILD_DIR)
            self.busy = True
            self.build_button.state(["disabled"])
            self.status.configure(text="Working...", foreground="#a15c00")

            def work():
                ok = False
                try:
                    ok = build(self.emit, folder, own)
                except Exception as err:  # keep the window alive
                    self.emit("Unexpected error: %r" % (err,))
                self.events.put(("done", ok))

            threading.Thread(target=work, daemon=True).start()

    root = tk.Tk()
    App(root)
    root.mainloop()


def main(argv):
    parser = argparse.ArgumentParser(prog="build_portal.py", description="Build portal.dll for the patched client.")
    parser.add_argument("--client", help="folder with Gw.custom.exe; portal.dll is copied there")
    parser.add_argument("--source", help="use this portal.c instead of downloading it")
    parser.add_argument("--no-gui", action="store_true", help="build in this console")
    args = parser.parse_args(argv)
    if args.client or args.source or args.no_gui:
        return 0 if build(print, args.client, args.source) else 1
    try:
        import tkinter  # noqa: F401
        run_gui()
        return 0
    except ImportError:
        print("The window needs tkinter, which this Python does not have. Use: python build_portal.py --no-gui")
    except Exception as err:
        if err.__class__.__name__ != "TclError":
            raise
        print("Could not open a window (%s). Use: python build_portal.py --no-gui" % err)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
