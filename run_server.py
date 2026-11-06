#!/usr/bin/env python3
"""run_server.py - set up, start, stop and watch the A.S.U.R.A. server from one window.

Double-click it, or run:

    python run_server.py            open the window (falls back to the console help if there is no display)
    python run_server.py check      list what is installed and what is missing
    python run_server.py setup      first-time setup: .env, database, build, schema, seed
    python run_server.py start      start the server in this console (Ctrl+C stops it)
    python run_server.py stop       stop a running server
    python run_server.py status     show what is running

This file adds no server behaviour. It only runs the commands docs/19-running.md already lists
(cargo build, asura-supervisor schema / seed / start / stop / status / restart) and writes the .env
file. It needs Python 3.8+ and nothing from pip; the window uses tkinter, which ships with Python
(on Debian/Ubuntu: sudo apt install python3-tk).

What it cannot install for you: Rust (https://rustup.rs) and PostgreSQL (https://www.postgresql.org/download/).
The Setup tab tells you which of the two is missing.
"""

import argparse
import getpass
import os
import queue
import re
import shutil
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path
from urllib.parse import quote, unquote, urlsplit

ROOT = Path(__file__).resolve().parent
IS_WIN = os.name == "nt"
EXE_SUFFIX = ".exe" if IS_WIN else ""
CONFIG_FILE = ROOT / "config" / "services.toml"
ENV_FILE = ROOT / ".env"
LOG_DIR = ROOT / "logs"
SUPERVISOR_EXE = ROOT / "target" / "debug" / ("asura-supervisor" + EXE_SUFFIX)
BUILD_PACKAGES = ["asura-supervisor", "asura-gateway", "asura-world", "asura-webgate"]
SERVICE_BINARIES = ["asura-gateway", "asura-world", "asura-webgate"]

DEFAULT_CONFIG = {
    "role": "asura",
    "password": "asura-dev",
    "name": "asura",
    "host": "127.0.0.1",
    "port": "5432",
}

CLIENT_ARGS = "Gw.custom.exe -authsrv 127.0.0.1 -portal 127.0.0.1"

ACCOUNTS = [
    "user1@example.com",
    "user2@example.com",
    "user3@example.com",
]

IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]{0,62}")


# ---------------------------------------------------------------------------------------------
# Finding things
# ---------------------------------------------------------------------------------------------

def find_cargo():
    found = shutil.which("cargo")
    if found:
        return found
    candidate = Path.home() / ".cargo" / "bin" / ("cargo" + EXE_SUFFIX)
    return str(candidate) if candidate.is_file() else None


def find_psql():
    explicit = os.environ.get("ASURA_PSQL")
    if explicit and Path(explicit).is_file():
        return explicit
    found = shutil.which("psql")
    if found:
        return found
    if IS_WIN:
        # ProgramW6432 is the real 64-bit Program Files even inside a 32-bit Python,
        # where ProgramFiles points at "Program Files (x86)". Check every spelling.
        bases = []
        for base in (os.environ.get("ProgramW6432"), os.environ.get("ProgramFiles"),
                     r"C:\Program Files", os.environ.get("ProgramFiles(x86)")):
            if base and base not in bases:
                bases.append(base)

        def version_key(exe):
            # Folder names like "9.6" and "18": sort numerically so 18 beats 9.6.
            nums = re.findall(r"\d+", exe.parent.parent.name)
            return tuple(int(n) for n in nums)

        for base in bases:
            root = Path(base) / "PostgreSQL"
            if root.is_dir():
                candidates = sorted(root.glob("*/bin/psql.exe"), key=version_key)
                if candidates:
                    return str(candidates[-1])
    return None


MSVC_HINT = ("Rust on Windows needs the Visual Studio C++ build tools (for link.exe). Install them from "
             "https://visualstudio.microsoft.com/visual-cpp-build-tools/ and tick 'Desktop development with C++', "
             "or in PowerShell: winget install Microsoft.VisualStudio.2022.BuildTools --override "
             "\"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended\" . "
             "Then restart this program and run the setup again.")


def msvc_linker_ok():
    """True when the C++ build tools are present (always True off Windows)."""
    if not IS_WIN:
        return True
    base = os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")
    vswhere = Path(base) / "Microsoft Visual Studio" / "Installer" / "vswhere.exe"
    if not vswhere.is_file():
        return False
    try:
        done = subprocess.run(
            [str(vswhere), "-latest", "-products", "*", "-requires",
             "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"],
            capture_output=True, text=True, timeout=20, creationflags=0x08000000)
    except (OSError, subprocess.TimeoutExpired):
        return False
    return bool(done.stdout.strip())


def child_env(trace=False):
    """The environment every child process gets: cargo and psql findable even off PATH."""
    env = dict(os.environ)
    cargo = find_cargo()
    if cargo:
        bin_dir = str(Path(cargo).parent)
        parts = env.get("PATH", "").split(os.pathsep)
        if bin_dir not in parts:
            env["PATH"] = bin_dir + os.pathsep + env.get("PATH", "")
        env.setdefault("CARGO", cargo)
    psql = find_psql()
    if psql:
        env.setdefault("ASURA_PSQL", psql)
    if trace:
        env["ASURA_TRACE"] = "1"
    return env


def tcp_open(host, port, timeout=0.4):
    try:
        with socket.create_connection((host, int(port)), timeout=timeout):
            return True
    except (OSError, ValueError):
        return False


def read_services():
    """[(name, port)] for every active [services.NAME] block, plus the control address."""
    try:
        text = CONFIG_FILE.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return [], "127.0.0.1:7711"
    control = "127.0.0.1:7711"
    found = re.search(r'(?m)^control\s*=\s*"([^"]+)"', text)
    if found:
        control = found.group(1)
    parts = re.split(r"(?m)^\[services\.([\w.\-]+)\]\s*$", text)
    services = []
    for i in range(1, len(parts), 2):
        port = re.search(r"(?m)^port\s*=\s*(\d+)", parts[i + 1])
        services.append((parts[i], int(port.group(1)) if port else None))
    return services, control


def control_target():
    _, control = read_services()
    host, _, port = control.rpartition(":")
    return host or "127.0.0.1", int(port or 7711)


def open_path(path):
    try:
        if IS_WIN:
            os.startfile(str(path))  # noqa
        elif sys.platform == "darwin":
            subprocess.Popen(["open", str(path)])
        else:
            subprocess.Popen(["xdg-open", str(path)])
    except OSError:
        pass


def postgres_start_hint():
    if IS_WIN:
        return ("Start PostgreSQL: open PowerShell as Administrator and run  Get-Service *postgres*  "
                "to find its name, then  Start-Service 'postgresql-x64-16'  (use your version).")
    return "Start PostgreSQL:  sudo systemctl start postgresql"


# ---------------------------------------------------------------------------------------------
# .env handling
# ---------------------------------------------------------------------------------------------

def database_url(cfg):
    return "postgres://{}:{}@{}:{}/{}".format(
        quote(cfg["role"], safe=""), quote(cfg["password"], safe=""),
        cfg["host"], cfg["port"], quote(cfg["name"], safe=""))


def load_config():
    """The database settings from .env (the URL wins), else the defaults."""
    cfg = dict(DEFAULT_CONFIG)
    values = {}
    if ENV_FILE.is_file():
        for line in ENV_FILE.read_text(encoding="utf-8", errors="replace").splitlines():
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip().strip('"').strip("'")
        mapping = {"ASURA_DB_ROLE": "role", "ASURA_DB_PASSWORD": "password", "ASURA_DB_NAME": "name",
                   "ASURA_DB_HOST": "host", "ASURA_DB_PORT": "port"}
        for key, field in mapping.items():
            if values.get(key):
                cfg[field] = values[key]
        url = values.get("ASURA_DATABASE_URL")
        if url:
            try:
                parts = urlsplit(url)
                if parts.username:
                    cfg["role"] = unquote(parts.username)
                if parts.password is not None:
                    cfg["password"] = unquote(parts.password)
                if parts.hostname:
                    cfg["host"] = parts.hostname
                if parts.port:
                    cfg["port"] = str(parts.port)
                if parts.path.strip("/"):
                    cfg["name"] = unquote(parts.path.strip("/"))
            except ValueError:
                pass
    return cfg


def env_file_has_url():
    if not ENV_FILE.is_file():
        return False
    text = ENV_FILE.read_text(encoding="utf-8", errors="replace")
    return re.search(r"(?m)^\s*ASURA_DATABASE_URL\s*=\s*\S+", text) is not None


def validate_config(cfg):
    problems = []
    for field, label in (("role", "role"), ("name", "database name")):
        if not IDENT.fullmatch(cfg[field]):
            problems.append("The %s must be letters, digits and underscores only (it starts with a letter)." % label)
    if not cfg["password"]:
        problems.append("Pick a database password (any text).")
    if not cfg["host"].strip():
        problems.append("The database host is empty.")
    if not cfg["port"].isdigit():
        problems.append("The database port must be a number (default 5432).")
    return problems


def write_env(cfg):
    """Create or update .env, keeping any lines and comments that are already there."""
    updates = {
        "ASURA_DATABASE_URL": database_url(cfg),
        "ASURA_DB_ROLE": cfg["role"],
        "ASURA_DB_PASSWORD": cfg["password"],
        "ASURA_DB_NAME": cfg["name"],
        "ASURA_DB_HOST": cfg["host"],
        "ASURA_DB_PORT": cfg["port"],
    }
    lines = []
    if ENV_FILE.is_file():
        lines = ENV_FILE.read_text(encoding="utf-8", errors="replace").splitlines()
    else:
        lines = ["# Written by run_server.py. .env is gitignored."]
    done = set()
    for i, line in enumerate(lines):
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key = stripped.split("=", 1)[0].strip()
        if key in updates:
            lines[i] = "%s=%s" % (key, updates[key])
            done.add(key)
    for key, value in updates.items():
        if key not in done:
            lines.append("%s=%s" % (key, value))
    if not any(l.strip().startswith("ASURA_AUTH_BIND") for l in lines):
        lines.append("ASURA_AUTH_BIND=127.0.0.1:6112")
    ENV_FILE.write_text("\n".join(lines) + "\n", encoding="utf-8")


# ---------------------------------------------------------------------------------------------
# Running commands
# ---------------------------------------------------------------------------------------------

def spawn(cmd, env, detached=False):
    kwargs = {}
    if IS_WIN:
        flags = 0x08000000  # CREATE_NO_WINDOW
        if detached:
            flags |= 0x00000200  # CREATE_NEW_PROCESS_GROUP
        kwargs["creationflags"] = flags
    elif detached:
        kwargs["start_new_session"] = True
    return subprocess.Popen(
        cmd, cwd=str(ROOT), env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace", bufsize=1, **kwargs)


def psql_capture(psql, cfg, user, password, database, sql):
    """Run one SQL statement through psql. Returns (exit code, output)."""
    env = child_env()
    env["PGPASSWORD"] = password
    env["PGCONNECT_TIMEOUT"] = "8"
    cmd = [psql, "-X", "-A", "-t", "-v", "ON_ERROR_STOP=1", "-h", cfg["host"], "-p", cfg["port"],
           "-U", user, "-d", database, "-c", sql]
    kwargs = {"creationflags": 0x08000000} if IS_WIN else {}
    try:
        done = subprocess.run(cmd, cwd=str(ROOT), env=env, capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=30, **kwargs)
    except (OSError, subprocess.TimeoutExpired) as err:
        return 1, str(err)
    return done.returncode, (done.stdout + done.stderr).strip()


def can_login(cfg):
    psql = find_psql()
    if not psql:
        return False
    code, _ = psql_capture(psql, cfg, cfg["role"], cfg["password"], cfg["name"], "SELECT 1")
    return code == 0


def service_binaries_built():
    return all((ROOT / "target" / "debug" / (name + EXE_SUFFIX)).is_file() for name in SERVICE_BINARIES)


def collect_checks(cfg):
    """[(label, state, detail, hint)] with state True / False / None (not checked yet)."""
    checks = []
    repo_ok = (ROOT / "Cargo.toml").is_file() and CONFIG_FILE.is_file()
    checks.append(("Server files", repo_ok, str(ROOT) if repo_ok else "Cargo.toml or config/services.toml missing",
                   "Put run_server.py in the top folder of the ASURA repository (next to Cargo.toml)."))
    cargo = find_cargo()
    checks.append(("Rust (cargo)", bool(cargo), cargo or "not found",
                   "Install Rust from https://rustup.rs , then close and reopen this program."))
    if IS_WIN:
        linker = msvc_linker_ok()
        checks.append(("C++ build tools", linker, "found" if linker else "not found (Rust cannot build without them)",
                       MSVC_HINT))
    psql = find_psql()
    checks.append(("PostgreSQL tools (psql)", bool(psql), psql or "not found",
                   "Install PostgreSQL from https://www.postgresql.org/download/ (it includes psql). "
                   "On Windows, tick 'Command Line Tools' in the installer."))
    db_up = tcp_open(cfg["host"], cfg["port"], 1.0)
    checks.append(("PostgreSQL running", db_up, "%s:%s answers" % (cfg["host"], cfg["port"]) if db_up
                   else "nothing answers on %s:%s" % (cfg["host"], cfg["port"]), postgres_start_hint()))
    has_env = env_file_has_url()
    checks.append((".env file", has_env, "present" if has_env else "missing",
                   "Fill in the form on the Setup tab and press 'Save .env'."))
    if psql and db_up and has_env:
        login = can_login(cfg)
        checks.append(("Database login", login, "role '%s' can open '%s'" % (cfg["role"], cfg["name"]) if login
                       else "role or database does not exist yet, or the password is wrong",
                       "Press 'Create database and role...' on the Setup tab."))
    else:
        checks.append(("Database login", None, "checked once the three items above are fine", ""))
    built = SUPERVISOR_EXE.is_file() and service_binaries_built()
    checks.append(("Server built", built, "target/debug is ready" if built else "not built yet",
                   "Press 'Build' on the Setup tab (the first build takes several minutes)."))
    graphs = ROOT / "data" / "path-graphs.bin"
    graphs_ok = graphs.is_file() and graphs.stat().st_size > 1000000
    checks.append(("Map data", graphs_ok, "data/path-graphs.bin ok" if graphs_ok
                   else "data/path-graphs.bin is missing or only a stub",
                   "Download the full repository again (git clone / Download ZIP), without skipping large files."))
    return checks


def next_step(checks):
    """One plain sentence telling a first-time user what to do now."""
    state = {label: ok for label, ok, _, _ in checks}
    if state.get("Server files") is False:
        return "run_server.py is not inside the ASURA folder. Move it next to Cargo.toml."
    if state.get("Rust (cargo)") is False:
        return "Install Rust first: https://rustup.rs  (then restart this program)."
    if state.get("C++ build tools") is False:
        return "Install the Visual Studio C++ build tools first (Rust needs them): see the red item on the Setup tab."
    if state.get("PostgreSQL tools (psql)") is False:
        return "Install PostgreSQL first: https://www.postgresql.org/download/  (then restart this program)."
    if state.get("PostgreSQL running") is False:
        return "PostgreSQL is not running. " + postgres_start_hint()
    if state.get(".env file") is False or state.get("Database login") is False or state.get("Server built") is False:
        return "First time here: open the Setup tab and press 'Run all setup steps'."
    return ""


class Ops:
    """The blocking operations. `emit` receives one line of text at a time."""

    def __init__(self, emit):
        self.emit = emit

    def say(self, text):
        self.emit("== " + text)

    def run(self, cmd):
        self.emit("$ " + subprocess.list2cmdline([str(c) for c in cmd]))
        try:
            proc = spawn([str(c) for c in cmd], child_env())
        except OSError as err:
            self.emit("cannot run %s: %s" % (cmd[0], err))
            return 127
        for line in proc.stdout:
            self.emit(line.rstrip("\r\n"))
        return proc.wait()

    def build(self, packages=BUILD_PACKAGES):
        cargo = find_cargo()
        if not cargo:
            self.emit("Rust is not installed. Get it from https://rustup.rs")
            return False
        cmd = [cargo, "build"]
        for package in packages:
            cmd += ["-p", package]
        if not msvc_linker_ok():
            self.emit("Cannot build: the C++ build tools are not installed.")
            self.emit(MSVC_HINT)
            return False
        return self.run(cmd) == 0

    def ensure_supervisor(self):
        if not self.build(["asura-supervisor"]):
            self.emit("The supervisor did not build; see the messages above.")
            return False
        if not SUPERVISOR_EXE.is_file():
            self.emit("Expected %s after the build, but it is not there." % SUPERVISOR_EXE)
            return False
        return True

    def supervisor(self, *args):
        if not SUPERVISOR_EXE.is_file():
            self.emit("The server is not built yet. Use 'Run all setup steps' on the Setup tab "
                      "(console: python run_server.py setup).")
            return 1
        return self.run([str(SUPERVISOR_EXE)] + list(args))

    def create_database(self, cfg, superuser, super_password):
        psql = find_psql()
        if not psql:
            self.emit("psql not found. Install PostgreSQL first.")
            return False
        role, name = cfg["role"], cfg["name"]
        literal = cfg["password"].replace("'", "''")

        def sql(statement):
            return psql_capture(psql, cfg, superuser, super_password, "postgres", statement)

        code, out = sql("SELECT 1 FROM pg_roles WHERE rolname = '%s'" % role)
        if code != 0:
            self.emit("Could not log in as '%s': %s" % (superuser, out))
            self.emit("Check the PostgreSQL admin name and password (the one chosen when PostgreSQL was installed).")
            return False
        if out.strip() == "1":
            code, out = sql("ALTER ROLE %s LOGIN PASSWORD '%s'" % (role, literal))
            self.emit("Role '%s' already exists; its password now matches the form." % role)
        else:
            code, out = sql("CREATE ROLE %s LOGIN PASSWORD '%s'" % (role, literal))
            self.emit("Created role '%s'." % role)
        if code != 0:
            self.emit(out)
            return False
        code, out = sql("SELECT 1 FROM pg_database WHERE datname = '%s'" % name)
        if code == 0 and out.strip() == "1":
            self.emit("Database '%s' already exists; left alone." % name)
            return True
        code, out = sql("CREATE DATABASE %s OWNER %s" % (name, role))
        if code != 0:
            self.emit(out)
            return False
        self.emit("Created database '%s' owned by '%s'." % (name, role))
        return True

    def setup_all(self, cfg, superuser, super_password):
        """First-time setup. Every step can be repeated safely."""
        problems = validate_config(cfg)
        if problems:
            for problem in problems:
                self.emit(problem)
            return False
        self.say("1/6 Checking Rust and PostgreSQL")
        if not find_cargo():
            self.emit("Rust is missing. Install it from https://rustup.rs and start this program again.")
            return False
        if not find_psql():
            self.emit("PostgreSQL tools are missing. Install PostgreSQL from https://www.postgresql.org/download/")
            return False
        if not tcp_open(cfg["host"], cfg["port"], 1.5):
            self.emit("PostgreSQL does not answer on %s:%s." % (cfg["host"], cfg["port"]))
            self.emit(postgres_start_hint())
            return False
        self.say("2/6 Writing .env")
        write_env(cfg)
        self.emit("Saved %s" % ENV_FILE)
        self.say("3/6 Database and role")
        if can_login(cfg):
            self.emit("Role '%s' can already open database '%s'; nothing to create." % (cfg["role"], cfg["name"]))
        elif super_password is None:
            self.emit("The database or role does not exist yet. Run setup again and give the PostgreSQL admin password.")
            return False
        elif not self.create_database(cfg, superuser, super_password):
            return False
        self.say("4/6 Building the server (the first time takes several minutes)")
        if not self.build():
            self.emit("The build failed; see the messages above.")
            return False
        if not SUPERVISOR_EXE.is_file():
            self.emit("Build finished but %s is missing." % SUPERVISOR_EXE)
            return False
        self.say("5/6 Creating the database tables")
        if self.supervisor("schema") != 0:
            return False
        self.say("6/6 Adding the test accounts and maps")
        if self.supervisor("seed") != 0:
            return False
        self.say("Setup finished. Press Start server.")
        return True

    def start_checks(self, cfg):
        """Why the server cannot start yet, or None when it can."""
        if not env_file_has_url():
            return "There is no .env yet. Use the Setup tab first."
        if not tcp_open(cfg["host"], cfg["port"], 1.5):
            return "PostgreSQL does not answer on %s:%s. %s" % (cfg["host"], cfg["port"], postgres_start_hint())
        host, port = control_target()
        if tcp_open(host, port):
            return "A server is already running (control port %d answers)." % port
        return None


def check_report(cfg):
    lines = []
    for label, ok, detail, hint in collect_checks(cfg):
        mark = "ok     " if ok else ("MISSING" if ok is False else "...    ")
        lines.append("[%s] %-24s %s" % (mark, label, detail))
        if ok is False and hint:
            lines.append("          -> " + hint)
    return lines


# ---------------------------------------------------------------------------------------------
# The window
# ---------------------------------------------------------------------------------------------

def run_gui():
    import tkinter as tk
    import tkinter.font as tkfont
    from tkinter import messagebox, simpledialog, ttk

    GREEN, GREY, RED = "#2e9e4f", "#9aa0a6", "#d64545"

    class Light(ttk.Frame):
        def __init__(self, parent, label, background):
            ttk.Frame.__init__(self, parent)
            self.canvas = tk.Canvas(self, width=14, height=14, highlightthickness=0, bg=background)
            self.dot = self.canvas.create_oval(2, 2, 12, 12, fill=GREY, outline="")
            self.canvas.pack(side="left", padx=(0, 5))
            ttk.Label(self, text=label).pack(side="left")

        def set(self, color):
            self.canvas.itemconfigure(self.dot, fill=color)

    class App:
        def __init__(self, root):
            self.root = root
            self.events = queue.Queue()
            self.supervisor_proc = None
            self.busy = False
            self.closing = False
            self.port_state = {}
            self.checks = []
            self.log_pos = {}
            self.service_lights = {}
            root.title("A.S.U.R.A. Server")
            root.geometry("1020x760")
            root.minsize(820, 560)
            try:
                ttk.Style().theme_use("clam")
            except tk.TclError:
                pass
            self.build_ui()
            self.fill_fields(load_config())
            root.protocol("WM_DELETE_WINDOW", self.on_close)
            root.after(80, self.pump)
            root.after(300, self.poll_ports)
            root.after(1000, self.tail_log)
            self.refresh_checks()

        # ---- layout ----
        def build_ui(self):
            root = self.root
            base = tkfont.nametofont("TkDefaultFont")
            big = base.copy()
            big.configure(size=base.cget("size") + 6, weight="bold")
            mid = base.copy()
            mid.configure(size=base.cget("size") + 2, weight="bold")
            bg = ttk.Style().lookup("TFrame", "background") or "#ececec"

            head = ttk.Frame(root, padding=(14, 10, 14, 2))
            head.pack(fill="x")
            ttk.Label(head, text="A.S.U.R.A. Server", font=big).pack(side="left")
            self.state_label = ttk.Label(head, text="STOPPED", font=mid)
            self.state_label.pack(side="right")

            self.hint_label = ttk.Label(root, text="", foreground="#a15c00", padding=(14, 0, 14, 4),
                                        wraplength=980, justify="left")
            self.hint_label.pack(fill="x")

            self.light_row = ttk.Frame(root, padding=(14, 2, 14, 8))
            self.light_row.pack(fill="x")
            self.light_bg = bg
            self.lights = {}
            self.rebuild_lights()

            paned = ttk.PanedWindow(root, orient="vertical")
            paned.pack(fill="both", expand=True, padx=10, pady=(0, 10))
            self.notebook = ttk.Notebook(paned)
            paned.add(self.notebook, weight=3)
            out_frame = ttk.LabelFrame(paned, text="Output", padding=4)
            paned.add(out_frame, weight=2)
            self.output = self.make_text(out_frame, wrap="none")

            self.build_server_tab()
            self.build_setup_tab()
            self.build_logs_tab()
            self.build_connect_tab()

        def make_text(self, parent, wrap="word", height=10):
            frame = ttk.Frame(parent)
            frame.pack(fill="both", expand=True)
            text = tk.Text(frame, wrap=wrap, height=height, bg="#1e1f22", fg="#d7d9dc", insertbackground="#d7d9dc",
                           relief="flat", font=tkfont.nametofont("TkFixedFont"), state="disabled")
            y = ttk.Scrollbar(frame, orient="vertical", command=text.yview)
            text.configure(yscrollcommand=y.set)
            y.pack(side="right", fill="y")
            text.pack(side="left", fill="both", expand=True)
            text.tag_configure("err", foreground="#ff7b72")
            text.tag_configure("head", foreground="#7ee787")
            text.tag_configure("cmd", foreground="#79c0ff")
            return text

        def build_server_tab(self):
            tab = ttk.Frame(self.notebook, padding=14)
            self.notebook.add(tab, text="  Server  ")
            row = ttk.Frame(tab)
            row.pack(fill="x")
            self.start_button = ttk.Button(row, text="Start server", command=self.on_start)
            self.start_button.pack(side="left", ipadx=14, ipady=6)
            self.stop_button = ttk.Button(row, text="Stop server", command=self.on_stop)
            self.stop_button.pack(side="left", padx=8, ipadx=14, ipady=6)
            self.status_button = ttk.Button(row, text="Show status", command=self.on_status)
            self.status_button.pack(side="left", ipadx=8, ipady=6)

            self.trace_var = tk.BooleanVar(value=False)
            ttk.Checkbutton(tab, text="Trace every packet into the logs (ASURA_TRACE=1, applies on the next start)",
                            variable=self.trace_var).pack(anchor="w", pady=(14, 0))

            restart = ttk.Frame(tab)
            restart.pack(fill="x", pady=(14, 0))
            ttk.Label(restart, text="Restart one service:").pack(side="left")
            names = [name for name, _ in read_services()[0]]
            self.restart_var = tk.StringVar(value=names[0] if names else "")
            self.restart_box = ttk.Combobox(restart, textvariable=self.restart_var, values=names, width=18,
                                            state="readonly")
            self.restart_box.pack(side="left", padx=8)
            self.restart_button = ttk.Button(restart, text="Restart", command=self.on_restart)
            self.restart_button.pack(side="left")

            ttk.Label(tab, justify="left", foreground="#555", wraplength=900, text=(
                "Start runs the server's own supervisor: it checks the database, builds anything out of date, "
                "adds the test accounts, then starts the web gate (6601), the login service (6112) and the "
                "world (6113) and keeps them alive. A service that keeps crashing is retried 5 times and then "
                "shown as failed. Stop shuts them all down cleanly.")).pack(anchor="w", pady=(18, 0))
            self.action_buttons = [self.start_button, self.stop_button, self.status_button, self.restart_button]

        def build_setup_tab(self):
            tab = ttk.Frame(self.notebook, padding=14)
            self.notebook.add(tab, text="  Setup  ")
            left = ttk.LabelFrame(tab, text="Database settings (saved to .env)", padding=10)
            left.pack(side="left", fill="y", padx=(0, 14))
            self.vars = {}
            fields = [("role", "Role (user)"), ("password", "Password"), ("name", "Database name"),
                      ("host", "Host"), ("port", "Port")]
            for i, (key, label) in enumerate(fields):
                ttk.Label(left, text=label).grid(row=i, column=0, sticky="w", pady=3)
                var = tk.StringVar()
                self.vars[key] = var
                entry = ttk.Entry(left, textvariable=var, width=26, show="*" if key == "password" else "")
                entry.grid(row=i, column=1, sticky="w", padx=(8, 0), pady=3)
                if key == "password":
                    self.password_entry = entry
            self.show_var = tk.BooleanVar(value=False)
            ttk.Checkbutton(left, text="Show password", variable=self.show_var,
                            command=lambda: self.password_entry.configure(show="" if self.show_var.get() else "*")
                            ).grid(row=len(fields), column=1, sticky="w", pady=(2, 8))
            buttons = [
                ("Run all setup steps", self.on_setup_all),
                ("Save .env", self.on_save_env),
                ("Create database and role...", self.on_create_db),
                ("Build", self.on_build),
                ("Create tables (schema)", self.on_schema),
                ("Add test accounts and maps (seed)", self.on_seed),
            ]
            self.setup_buttons = []
            for i, (label, command) in enumerate(buttons):
                button = ttk.Button(left, text=label, command=command)
                button.grid(row=len(fields) + 1 + i, column=0, columnspan=2, sticky="ew", pady=2,
                            ipady=3 if i == 0 else 0)
                self.setup_buttons.append(button)
            self.action_buttons += self.setup_buttons

            right = ttk.LabelFrame(tab, text="What is installed", padding=10)
            right.pack(side="left", fill="both", expand=True)
            self.check_frame = ttk.Frame(right)
            self.check_frame.pack(fill="both", expand=True)
            ttk.Button(right, text="Re-check", command=self.refresh_checks).pack(anchor="e", pady=(8, 0))

        def build_logs_tab(self):
            tab = ttk.Frame(self.notebook, padding=10)
            self.notebook.add(tab, text="  Logs  ")
            row = ttk.Frame(tab)
            row.pack(fill="x", pady=(0, 6))
            ttk.Label(row, text="Log file:").pack(side="left")
            self.log_var = tk.StringVar()
            self.log_box = ttk.Combobox(row, textvariable=self.log_var, width=26, state="readonly")
            self.log_box.pack(side="left", padx=8)
            self.log_box.bind("<<ComboboxSelected>>", lambda _e: self.load_log(force=True))
            ttk.Button(row, text="Refresh list", command=self.refresh_log_list).pack(side="left")
            ttk.Button(row, text="Open logs folder", command=lambda: (LOG_DIR.mkdir(exist_ok=True), open_path(LOG_DIR))
                       ).pack(side="left", padx=8)
            self.follow_var = tk.BooleanVar(value=True)
            ttk.Checkbutton(row, text="Follow", variable=self.follow_var).pack(side="left")
            self.log_text = self.make_text(tab, wrap="none")
            self.refresh_log_list()

        def build_connect_tab(self):
            tab = ttk.Frame(self.notebook, padding=14)
            self.notebook.add(tab, text="  Connect a client  ")
            text = (
                "The game client is Windows-only and must be the patched one (Gw.custom.exe, built with\n"
                "client-patcher/patch-gw.py - see client-patcher/README.md).\n\n"
                "Launch it against this server with:\n")
            ttk.Label(tab, text=text, justify="left").pack(anchor="w")
            row = ttk.Frame(tab)
            row.pack(anchor="w", fill="x")
            entry = ttk.Entry(row, width=60)
            entry.insert(0, CLIENT_ARGS)
            entry.configure(state="readonly")
            entry.pack(side="left")
            ttk.Button(row, text="Copy", command=lambda: self.copy_text(CLIENT_ARGS)).pack(side="left", padx=8)
            more = (
                "\nLog in as one of the test accounts (the web gate does not check the password):\n    "
                + "\n    ".join(ACCOUNTS) + "\n\n"
                "Ports: web gate 6601, login 6112 (the client plays through it), world 6113, database 5432.\n"
                "Everything listens on 127.0.0.1, so the client must run on this same computer. To let another\n"
                "computer connect, change the bind addresses in config/services.toml and allow the ports through\n"
                "the firewall (docs/17-running-on-linux.md, 'Reaching it from a Windows client').\n")
            ttk.Label(tab, text=more, justify="left").pack(anchor="w")
            row = ttk.Frame(tab)
            row.pack(anchor="w")
            ttk.Button(row, text="Open server folder", command=lambda: open_path(ROOT)).pack(side="left")
            ttk.Button(row, text="Open client-patcher folder",
                       command=lambda: open_path(ROOT / "client-patcher")).pack(side="left", padx=8)

        # ---- plumbing ----
        def copy_text(self, value):
            self.root.clipboard_clear()
            self.root.clipboard_append(value)
            self.emit("Copied: " + value)

        def emit(self, line):
            self.events.put(("line", line))

        def append(self, line):
            tag = ()
            if line.startswith("== "):
                tag = ("head",)
            elif line.startswith("$ "):
                tag = ("cmd",)
            elif re.search(r"\b(error|failed|cannot|could not|FAILED|missing)\b", line, re.I):
                tag = ("err",)
            self.output.configure(state="normal")
            self.output.insert("end", line + "\n", tag)
            if int(self.output.index("end-1c").split(".")[0]) > 6000:
                self.output.delete("1.0", "1000.0")
            self.output.configure(state="disabled")
            self.output.see("end")

        def pump(self):
            try:
                while True:
                    kind, payload = self.events.get_nowait()
                    if kind == "line":
                        self.append(payload)
                    elif kind == "done":
                        self.busy = False
                        self.refresh_buttons()
                        self.refresh_checks()
                        if payload:
                            payload()
                    elif kind == "ports":
                        self.port_state = payload
                        self.update_lights()
                    elif kind == "checks":
                        self.checks = payload
                        self.show_checks()
            except queue.Empty:
                pass
            if self.supervisor_proc is not None and self.supervisor_proc.poll() is not None:
                self.append("== The supervisor stopped (exit code %s)." % self.supervisor_proc.returncode)
                self.supervisor_proc = None
                self.refresh_buttons()
            if not self.closing:
                self.root.after(80, self.pump)

        def run_bg(self, work, after=None):
            """Run work(ops) on a thread; buttons stay disabled until it ends."""
            if self.busy:
                return
            self.busy = True
            self.refresh_buttons()

            def target():
                ops = Ops(self.emit)
                try:
                    work(ops)
                except Exception as err:  # keep the window alive whatever happens
                    self.emit("Unexpected error: %r" % (err,))
                self.events.put(("done", after))

            threading.Thread(target=target, daemon=True).start()

        def running(self):
            host, port = control_target()
            return self.port_state.get("Supervisor", tcp_open(host, port))

        def refresh_buttons(self):
            running = self.running() or self.supervisor_proc is not None
            for button in self.action_buttons:
                button.state(["disabled"] if self.busy else ["!disabled"])
            if not self.busy:
                self.start_button.state(["disabled"] if running else ["!disabled"])
                self.stop_button.state(["!disabled"] if running else ["disabled"])
                self.restart_button.state(["!disabled"] if running else ["disabled"])
                self.status_button.state(["!disabled"] if running else ["disabled"])

        # ---- status lights ----
        def rebuild_lights(self):
            for child in self.light_row.winfo_children():
                child.destroy()
            self.lights = {}
            names = ["Database", "Supervisor"] + [name for name, _ in read_services()[0]]
            for name in names:
                light = Light(self.light_row, name, self.light_bg)
                light.pack(side="left", padx=(0, 18))
                self.lights[name] = light

        def poll_ports(self):
            cfg = self.read_fields()
            services = read_services()[0]
            c_host, c_port = control_target()

            def work():
                state = {"Database": tcp_open(cfg["host"], cfg["port"], 0.8) if cfg["port"].isdigit() else False,
                         "Supervisor": tcp_open(c_host, c_port)}
                for name, port in services:
                    state[name] = tcp_open("127.0.0.1", port) if port else False
                if not self.closing:
                    self.events.put(("ports", state))

            threading.Thread(target=work, daemon=True).start()
            if not self.closing:
                self.root.after(1500, self.poll_ports)

        def update_lights(self):
            for name, light in self.lights.items():
                up = self.port_state.get(name)
                if name == "Database":
                    light.set(GREEN if up else RED)
                else:
                    light.set(GREEN if up else GREY)
            supervisor = self.port_state.get("Supervisor")
            services = [self.port_state.get(name) for name, _ in read_services()[0]]
            if supervisor and services and all(services):
                self.state_label.configure(text="RUNNING", foreground=GREEN)
            elif supervisor:
                self.state_label.configure(text="STARTING...", foreground="#a15c00")
            else:
                self.state_label.configure(text="STOPPED", foreground="#555555")
            if not self.busy:
                self.refresh_buttons()

        # ---- requirements ----
        def refresh_checks(self):
            cfg = self.read_fields()

            def work():
                result = collect_checks(cfg)
                if not self.closing:
                    self.events.put(("checks", result))

            threading.Thread(target=work, daemon=True).start()

        def show_checks(self):
            for child in self.check_frame.winfo_children():
                child.destroy()
            for i, (label, ok, detail, hint) in enumerate(self.checks):
                color = GREEN if ok else (RED if ok is False else GREY)
                canvas = tk.Canvas(self.check_frame, width=14, height=14, highlightthickness=0, bg=self.light_bg)
                canvas.create_oval(2, 2, 12, 12, fill=color, outline="")
                canvas.grid(row=i * 2, column=0, sticky="w", padx=(0, 6))
                ttk.Label(self.check_frame, text=label).grid(row=i * 2, column=1, sticky="w", padx=(0, 12))
                ttk.Label(self.check_frame, text=detail, foreground="#555").grid(row=i * 2, column=2, sticky="w")
                if ok is False and hint:
                    ttk.Label(self.check_frame, text=hint, foreground="#a15c00", wraplength=420, justify="left"
                              ).grid(row=i * 2 + 1, column=1, columnspan=2, sticky="w", pady=(0, 4))
            self.hint_label.configure(text=next_step(self.checks))

        # ---- settings ----
        def fill_fields(self, cfg):
            for key, var in self.vars.items():
                var.set(cfg[key])

        def read_fields(self):
            if not hasattr(self, "vars"):
                return dict(DEFAULT_CONFIG)
            return {key: var.get().strip() if key != "password" else var.get() for key, var in self.vars.items()}

        def valid_fields(self):
            cfg = self.read_fields()
            problems = validate_config(cfg)
            if problems:
                messagebox.showwarning("Check the settings", "\n".join(problems))
                return None
            return cfg

        # ---- button handlers ----
        def on_save_env(self):
            cfg = self.valid_fields()
            if cfg:
                write_env(cfg)
                self.emit("Saved %s" % ENV_FILE)
                self.refresh_checks()

        def ask_superuser(self):
            """(user, password), or None when the person cancels."""
            user = simpledialog.askstring(
                "PostgreSQL administrator",
                "To create the database, enter the PostgreSQL admin name.\n(The default is 'postgres'.)",
                initialvalue="postgres", parent=self.root)
            if not user:
                return None
            password = simpledialog.askstring(
                "PostgreSQL administrator",
                "Password of '%s'\n(the one you chose when installing PostgreSQL).\n"
                "It is used once and not saved." % user, show="*", parent=self.root)
            if password is None:
                return None
            return user, password

        def on_create_db(self):
            cfg = self.valid_fields()
            if not cfg:
                return
            creds = self.ask_superuser()
            if not creds:
                return

            def work(ops):
                write_env(cfg)
                ops.say("Creating the database and role")
                ops.create_database(cfg, creds[0], creds[1])

            self.run_bg(work)

        def on_setup_all(self):
            cfg = self.valid_fields()
            if not cfg:
                return
            creds = None
            if find_psql() and tcp_open(cfg["host"], cfg["port"], 1.5) and not can_login(cfg):
                creds = self.ask_superuser()
                if not creds:
                    return

            def work(ops):
                ops.setup_all(cfg, creds[0] if creds else "postgres", creds[1] if creds else None)

            self.run_bg(work)

        def on_build(self):
            self.run_bg(lambda ops: (ops.say("Building"), ops.build(),
                                     ops.say("Build finished; see the lines above for errors.")))

        def on_schema(self):
            def work(ops):
                if ops.ensure_supervisor():
                    ops.say("Creating the database tables")
                    ops.supervisor("schema")
            self.run_bg(work)

        def on_seed(self):
            def work(ops):
                if ops.ensure_supervisor():
                    ops.say("Adding the test accounts and maps")
                    ops.supervisor("seed")
            self.run_bg(work)

        def on_start(self):
            cfg = self.read_fields()
            trace = self.trace_var.get()

            def work(ops):
                why = ops.start_checks(cfg)
                if why:
                    ops.emit(why)
                    return
                ops.say("Starting the server")
                if not ops.ensure_supervisor():
                    return
                env = child_env(trace)
                ops.emit("$ %s start" % SUPERVISOR_EXE)
                try:
                    proc = spawn([str(SUPERVISOR_EXE), "start"], env, detached=True)
                except OSError as err:
                    ops.emit("cannot start the supervisor: %s" % err)
                    return
                self.supervisor_proc = proc

                def reader():
                    for line in proc.stdout:
                        self.emit(line.rstrip("\r\n"))

                threading.Thread(target=reader, daemon=True).start()
                ops.emit("The supervisor is starting; the lights turn green as each service is ready.")

            self.run_bg(work)

        def stop_work(self, ops):
            ops.say("Stopping the server")
            if not SUPERVISOR_EXE.is_file():
                ops.emit("Nothing to stop: the supervisor is not built.")
                return
            ops.supervisor("stop")
            proc = self.supervisor_proc
            if proc is not None:
                for _ in range(300):
                    if proc.poll() is not None:
                        break
                    time.sleep(0.1)
                else:
                    ops.emit("The supervisor is still shutting down; give it a few more seconds.")

        def on_stop(self):
            self.run_bg(self.stop_work)

        def on_status(self):
            def work(ops):
                ops.say("Status")
                if SUPERVISOR_EXE.is_file():
                    ops.supervisor("status")
            self.run_bg(work)

        def on_restart(self):
            name = self.restart_var.get()
            if not name:
                return

            def work(ops):
                ops.say("Restarting %s" % name)
                ops.supervisor("restart", name)

            self.run_bg(work)

        # ---- logs ----
        def refresh_log_list(self):
            files = sorted(p.name for p in LOG_DIR.glob("*.log")) if LOG_DIR.is_dir() else []
            self.log_box.configure(values=files)
            if files and self.log_var.get() not in files:
                self.log_var.set(files[0])
                self.load_log(force=True)

        def load_log(self, force=False):
            name = self.log_var.get()
            if not name:
                return
            path = LOG_DIR / name
            try:
                size = path.stat().st_size
            except OSError:
                return
            if not force and self.log_pos.get(name) == size:
                return
            self.log_pos[name] = size
            try:
                with open(path, "rb") as handle:
                    if size > 200000:
                        handle.seek(size - 200000)
                    data = handle.read().decode("utf-8", errors="replace")
            except OSError:
                return
            self.log_text.configure(state="normal")
            self.log_text.delete("1.0", "end")
            self.log_text.insert("end", data)
            self.log_text.configure(state="disabled")
            if self.follow_var.get():
                self.log_text.see("end")

        def tail_log(self):
            if self.notebook.index(self.notebook.select()) == 2:
                if not self.log_box.cget("values"):
                    self.refresh_log_list()
                self.load_log()
            if not self.closing:
                self.root.after(1000, self.tail_log)

        # ---- closing ----
        def on_close(self):
            if self.busy:
                if not messagebox.askyesno("Still working", "A task is still running. Close anyway?"):
                    return
            if self.running() or self.supervisor_proc is not None:
                answer = messagebox.askyesnocancel(
                    "Server is running",
                    "The server is still running.\n\nYes = stop it, then close\nNo = close and leave it running")
                if answer is None:
                    return
                if answer:
                    self.busy = False
                    self.closing = True
                    done = threading.Event()

                    def work():
                        self.stop_work(Ops(lambda line: None))
                        done.set()

                    threading.Thread(target=work, daemon=True).start()
                    done.wait(40)
            self.closing = True
            self.root.destroy()

    root = tk.Tk()
    App(root)
    root.mainloop()


# ---------------------------------------------------------------------------------------------
# Console mode
# ---------------------------------------------------------------------------------------------

def console_help():
    print(__doc__.split("What it cannot install")[0].strip())


def console_main(args):
    cfg = load_config()
    ops = Ops(print)
    if args.command == "check":
        for line in check_report(cfg):
            print(line)
        hint = next_step(collect_checks(cfg))
        if hint:
            print("\nNext: " + hint)
        return 0
    if args.command == "setup":
        if not ENV_FILE.is_file():
            cfg["password"] = args.password or DEFAULT_CONFIG["password"]
        for field in ("role", "name", "host", "port"):
            value = getattr(args, "db_" + field, None)
            if value:
                cfg[field] = str(value)
        if args.password:
            cfg["password"] = args.password
        super_password = None
        if find_psql() and tcp_open(cfg["host"], cfg["port"], 1.5) and not can_login(cfg):
            super_password = os.environ.get("PGPASSWORD")
            if super_password is None:
                super_password = getpass.getpass("PostgreSQL admin password for '%s' (not saved): " % args.superuser)
        return 0 if ops.setup_all(cfg, args.superuser, super_password) else 1
    if args.command in ("stop", "status"):
        if not SUPERVISOR_EXE.is_file():
            print("The server is not built yet. Run: python run_server.py setup")
            return 1
        return ops.supervisor(args.command)
    if args.command == "start":
        why = ops.start_checks(cfg)
        if why:
            print(why)
            return 1
        if not ops.ensure_supervisor():
            return 1
        return subprocess.call([str(SUPERVISOR_EXE), "start"], cwd=str(ROOT), env=child_env(args.trace))
    return 2


def main(argv):
    if not (ROOT / "Cargo.toml").is_file():
        print("run_server.py must sit in the top folder of the ASURA repository (next to Cargo.toml).")
        return 1
    parser = argparse.ArgumentParser(prog="run_server.py", description="Set up and run the A.S.U.R.A. server.")
    sub = parser.add_subparsers(dest="command")
    sub.add_parser("gui", help="open the window (default)")
    sub.add_parser("check", help="show what is installed and what is missing")
    setup = sub.add_parser("setup", help="first-time setup: .env, database, build, schema, seed")
    setup.add_argument("--password", help="password for the database role (default: asura-dev)")
    setup.add_argument("--superuser", default="postgres", help="PostgreSQL admin name (default: postgres)")
    setup.add_argument("--db-role", dest="db_role")
    setup.add_argument("--db-name", dest="db_name")
    setup.add_argument("--db-host", dest="db_host")
    setup.add_argument("--db-port", dest="db_port")
    start = sub.add_parser("start", help="start the server in this console")
    start.add_argument("--trace", action="store_true", help="log every packet (ASURA_TRACE=1)")
    sub.add_parser("stop", help="stop the server")
    sub.add_parser("status", help="show what is running")
    args = parser.parse_args(argv)

    if args.command in (None, "gui"):
        try:
            import tkinter  # noqa: F401
            run_gui()
            return 0
        except ImportError:
            print("The window needs tkinter, which this Python does not have.")
            print("  Debian/Ubuntu: sudo apt install python3-tk     Windows: reinstall Python with 'tcl/tk' ticked.\n")
        except Exception as err:  # no display, for example on a Linux server over ssh
            if err.__class__.__name__ != "TclError":
                raise
            print("Could not open a window (%s).\n" % err)
        console_help()
        return 1
    return console_main(args)


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except KeyboardInterrupt:
        sys.exit(130)
