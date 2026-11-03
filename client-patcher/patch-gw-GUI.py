#!/usr/bin/env python3
"""GUI front end for the ASURA client patcher.

Wraps patch-gw.py and hosts-redirect.py (both in this folder) in one Tkinter window:

  Patch   - pick Gw.exe and keys.txt, choose which ArenaNet hostnames to repoint, write Gw.custom.exe
  Hosts   - show / enable / disable the machine-wide hosts block
  Launch  - start Gw.custom.exe with -authsrv / -portal (Windows only)

The two scripts are loaded as modules and called directly, so the GUI does exactly what the command
line does. Needs Python 3 with tkinter, plus `pip install pefile` for the Patch tab.
"""

import argparse
import contextlib
import ctypes
import importlib.util
import io
import os
import queue
import subprocess
import sys
import threading
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

HERE = os.path.dirname(os.path.abspath(__file__))

# (checkbox label, prefix understood by patch-gw.py --detach-only, on in the default build)
HOST_CHOICES = (
    ('Auth1.ArenaNetworks.com', 'Auth1', True),
    ('File1.ArenaNetworks.com  (file server, optional)', 'File1', False),
    ('Diag.ArenaNetworks.com', 'Diag', True),
    ('bug.arenanetworks.com:6112', 'bug', True),
    ('icmp.arenanetworks.com', 'icmp', True),
    ('ArenaNetworks.com', 'ArenaNetworks.com', True),
    ('account.arena.net', 'account.arena.net', True),
)

_op_lock = threading.Lock()  # stdout redirection is process-wide, so one operation at a time


def load_script(filename):
    """Import a hyphenated script from this folder as a module."""
    path = os.path.join(HERE, filename)
    if not os.path.exists(path):
        raise FileNotFoundError(f'{filename} not found next to GUIapp.py ({HERE})')
    name = os.path.splitext(filename)[0].replace('-', '_')
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def is_admin():
    try:
        if os.name == 'nt':
            return bool(ctypes.windll.shell32.IsUserAnAdmin())
        return os.geteuid() == 0
    except Exception:
        return False


def relaunch_elevated():
    """Windows only: restart this program with the UAC prompt."""
    params = ' '.join(f'"{a}"' for a in sys.argv)
    ctypes.windll.shell32.ShellExecuteW(None, 'runas', sys.executable, params, None, 1)


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title('ASURA client patcher')
        self.geometry('780x720')
        self.minsize(700, 620)
        self.log_queue = queue.Queue()
        self.busy = False

        self._build_widgets()
        self.after(100, self._drain_log)
        self._log(f'Folder: {HERE}')
        self._log(f'Administrator: {"yes" if is_admin() else "no (needed for the Hosts tab)"}')

    # ------------------------------------------------------------------ layout

    def _build_widgets(self):
        pad = {'padx': 8, 'pady': 4}
        notebook = ttk.Notebook(self)
        notebook.pack(fill='x', padx=8, pady=(8, 4))

        self.patch_tab = ttk.Frame(notebook)
        self.hosts_tab = ttk.Frame(notebook)
        self.launch_tab = ttk.Frame(notebook)
        notebook.add(self.patch_tab, text='Patch')
        notebook.add(self.hosts_tab, text='Hosts')
        notebook.add(self.launch_tab, text='Launch')

        self._build_patch_tab(pad)
        self._build_hosts_tab(pad)
        self._build_launch_tab(pad)

        log_frame = ttk.LabelFrame(self, text='Log')
        log_frame.pack(fill='both', expand=True, padx=8, pady=(4, 8))
        self.log_text = tk.Text(log_frame, height=14, wrap='word', state='disabled',
                                font=('Consolas' if os.name == 'nt' else 'monospace', 9))
        scroll = ttk.Scrollbar(log_frame, command=self.log_text.yview)
        self.log_text.configure(yscrollcommand=scroll.set)
        self.log_text.pack(side='left', fill='both', expand=True)
        scroll.pack(side='right', fill='y')
        ttk.Button(self, text='Clear log', command=self._clear_log).pack(anchor='e', padx=8, pady=(0, 8))

    def _file_row(self, parent, row, label, var, browse):
        ttk.Label(parent, text=label).grid(row=row, column=0, sticky='w', padx=8, pady=4)
        ttk.Entry(parent, textvariable=var).grid(row=row, column=1, sticky='ew', padx=4, pady=4)
        ttk.Button(parent, text='Browse...', command=browse).grid(row=row, column=2, padx=8, pady=4)

    def _build_patch_tab(self, pad):
        tab = self.patch_tab
        tab.columnconfigure(1, weight=1)

        self.gw_var = tk.StringVar()
        self.keys_var = tk.StringVar(value=os.path.join(HERE, 'keys.txt'))
        self.out_var = tk.StringVar()

        self._file_row(tab, 0, 'Gw.exe', self.gw_var, self._browse_gw)
        self._file_row(tab, 1, 'Keys file', self.keys_var, self._browse_keys)
        self._file_row(tab, 2, 'Output', self.out_var, self._browse_out)

        hosts = ttk.LabelFrame(tab, text='Hostnames to repoint at 127.0.0.1')
        hosts.grid(row=3, column=0, columnspan=3, sticky='ew', padx=8, pady=8)
        self.host_vars = {}
        for label, prefix, default_on in HOST_CHOICES:
            var = tk.BooleanVar(value=default_on)
            self.host_vars[prefix] = var
            ttk.Checkbutton(hosts, text=label, variable=var,
                            command=self._sync_file_server_flag).pack(anchor='w', padx=8, pady=1)

        presets = ttk.Frame(hosts)
        presets.pack(anchor='w', padx=8, pady=6)
        ttk.Button(presets, text='Default (all but File1)',
                   command=lambda: self._preset(False)).pack(side='left', padx=(0, 6))
        ttk.Button(presets, text='All seven (--file-server)',
                   command=lambda: self._preset(True)).pack(side='left')

        ttk.Label(tab, text='If File1 is on, enable it on the Hosts tab too so the names agree.',
                  foreground='#555').grid(row=4, column=0, columnspan=3, sticky='w', padx=8)

        self.patch_btn = ttk.Button(tab, text='Patch client', command=self._do_patch)
        self.patch_btn.grid(row=5, column=0, columnspan=3, pady=10)

    def _build_hosts_tab(self, pad):
        tab = self.hosts_tab
        tab.columnconfigure(1, weight=1)

        self.hosts_path_var = tk.StringVar()
        self.target_var = tk.StringVar(value='127.0.0.1')
        self.hosts_file_server = tk.BooleanVar(value=False)

        self._file_row(tab, 0, 'Hosts file', self.hosts_path_var, self._browse_hosts)
        ttk.Label(tab, text='Target address').grid(row=1, column=0, sticky='w', padx=8, pady=4)
        ttk.Entry(tab, textvariable=self.target_var, width=20).grid(row=1, column=1, sticky='w', padx=4, pady=4)
        ttk.Checkbutton(tab, text='Also redirect File1.ArenaNetworks.com (--file-server)',
                        variable=self.hosts_file_server).grid(row=2, column=0, columnspan=3,
                                                              sticky='w', padx=8, pady=4)

        ttk.Label(tab, text='The block is machine-wide: a retail client on this PC is redirected too '
                            'while it is enabled.', foreground='#a33').grid(
            row=3, column=0, columnspan=3, sticky='w', padx=8, pady=4)

        buttons = ttk.Frame(tab)
        buttons.grid(row=4, column=0, columnspan=3, pady=10)
        self.hosts_buttons = []
        for text, action in (('Status', 'status'), ('Enable', 'enable'), ('Disable', 'disable')):
            b = ttk.Button(buttons, text=text, command=lambda a=action: self._do_hosts(a))
            b.pack(side='left', padx=6)
            self.hosts_buttons.append(b)

        if os.name == 'nt' and not is_admin():
            ttk.Button(tab, text='Restart as administrator', command=self._elevate).grid(
                row=5, column=0, columnspan=3, pady=4)
        elif not is_admin():
            ttk.Label(tab, text='Not root: run this program with sudo to change /etc/hosts.',
                      foreground='#555').grid(row=5, column=0, columnspan=3, padx=8, pady=4)

        try:
            self.hosts_path_var.set(load_script('hosts-redirect.py').default_hosts_path())
        except Exception as exc:
            self._log(f'[!] {exc}')

    def _build_launch_tab(self, pad):
        tab = self.launch_tab
        tab.columnconfigure(1, weight=1)

        self.exe_var = tk.StringVar()
        self.auth_var = tk.StringVar(value='127.0.0.1')
        self.portal_var = tk.StringVar(value='127.0.0.1')

        self._file_row(tab, 0, 'Gw.custom.exe', self.exe_var, self._browse_custom)
        ttk.Label(tab, text='-authsrv').grid(row=1, column=0, sticky='w', padx=8, pady=4)
        ttk.Entry(tab, textvariable=self.auth_var, width=24).grid(row=1, column=1, sticky='w', padx=4)
        ttk.Label(tab, text='-portal').grid(row=2, column=0, sticky='w', padx=8, pady=4)
        ttk.Entry(tab, textvariable=self.portal_var, width=24).grid(row=2, column=1, sticky='w', padx=4)

        ttk.Label(tab, text='portal.dll must sit next to Gw.custom.exe or the login will not work.',
                  foreground='#555').grid(row=3, column=0, columnspan=3, sticky='w', padx=8, pady=4)

        self.launch_btn = ttk.Button(tab, text='Launch client', command=self._do_launch)
        self.launch_btn.grid(row=4, column=0, columnspan=3, pady=10)
        if os.name != 'nt':
            self.launch_btn.state(['disabled'])
            ttk.Label(tab, text='The client is Windows-only; copy Gw.custom.exe to a Windows machine.',
                      foreground='#a33').grid(row=5, column=0, columnspan=3, padx=8)

    # ------------------------------------------------------------------ browse helpers

    def _browse_gw(self):
        path = filedialog.askopenfilename(title='Select Gw.exe',
                                          filetypes=[('Executable', '*.exe'), ('All files', '*.*')])
        if path:
            self.gw_var.set(path)
            name, ext = os.path.splitext(path)
            self.out_var.set(name + '.custom' + ext)
            self.exe_var.set(name + '.custom' + ext)

    def _browse_keys(self):
        path = filedialog.askopenfilename(title='Select keys file', initialdir=HERE,
                                          filetypes=[('Text', '*.txt'), ('All files', '*.*')])
        if path:
            self.keys_var.set(path)

    def _browse_out(self):
        path = filedialog.asksaveasfilename(title='Output file', defaultextension='.exe',
                                            filetypes=[('Executable', '*.exe'), ('All files', '*.*')])
        if path:
            self.out_var.set(path)

    def _browse_hosts(self):
        path = filedialog.askopenfilename(title='Select hosts file')
        if path:
            self.hosts_path_var.set(path)

    def _browse_custom(self):
        path = filedialog.askopenfilename(title='Select Gw.custom.exe',
                                          filetypes=[('Executable', '*.exe'), ('All files', '*.*')])
        if path:
            self.exe_var.set(path)

    # ------------------------------------------------------------------ host selection

    def _preset(self, include_file1):
        for _label, prefix, default_on in HOST_CHOICES:
            self.host_vars[prefix].set(True if prefix == 'File1' and include_file1 else default_on)
        self._sync_file_server_flag()

    def _sync_file_server_flag(self):
        self.hosts_file_server.set(self.host_vars['File1'].get())

    # ------------------------------------------------------------------ log

    def _log(self, text):
        self.log_queue.put(text)

    def _drain_log(self):
        try:
            while True:
                text = self.log_queue.get_nowait()
                self.log_text.configure(state='normal')
                self.log_text.insert('end', text.rstrip('\n') + '\n')
                self.log_text.see('end')
                self.log_text.configure(state='disabled')
        except queue.Empty:
            pass
        self.after(100, self._drain_log)

    def _clear_log(self):
        self.log_text.configure(state='normal')
        self.log_text.delete('1.0', 'end')
        self.log_text.configure(state='disabled')

    # ------------------------------------------------------------------ background work

    def _set_busy(self, busy):
        self.busy = busy
        state = ['disabled'] if busy else ['!disabled']
        self.patch_btn.state(state)
        for b in self.hosts_buttons:
            b.state(state)

    def _run_bg(self, title, func):
        """Run func() in a thread, capturing its prints and SystemExit into the log."""
        if self.busy:
            return
        self._set_busy(True)
        self._log(f'--- {title} ---')

        def worker():
            buf = io.StringIO()
            ok = True
            with _op_lock:
                try:
                    with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(buf):
                        func()
                except SystemExit as exc:
                    if exc.code not in (None, 0):
                        ok = False
                        buf.write(f'{exc.code}\n')
                except PermissionError:
                    ok = False
                    buf.write('Permission denied: run as administrator / root for the hosts file.\n')
                except Exception as exc:
                    ok = False
                    buf.write(f'Error: {type(exc).__name__}: {exc}\n')
            output = buf.getvalue()
            if output:
                self._log(output)
            self._log('Done.' if ok else 'FAILED.')
            self.after(0, lambda: self._set_busy(False))

        threading.Thread(target=worker, daemon=True).start()

    # ------------------------------------------------------------------ actions

    def _do_patch(self):
        gw = self.gw_var.get().strip()
        keys = self.keys_var.get().strip()
        out = self.out_var.get().strip()
        if not gw or not os.path.isfile(gw):
            messagebox.showerror('Patch', 'Pick the original Gw.exe first.')
            return
        if not keys or not os.path.isfile(keys):
            messagebox.showerror('Patch', 'The keys file does not exist.')
            return
        prefixes = [p for _l, p, _d in HOST_CHOICES if self.host_vars[p].get()]
        if not prefixes:
            messagebox.showerror('Patch', 'Select at least one hostname to repoint.')
            return
        if not out:
            name, ext = os.path.splitext(gw)
            out = name + '.custom' + ext
            self.out_var.set(out)
        if os.path.abspath(out) == os.path.abspath(gw):
            messagebox.showerror('Patch', 'The output must differ from the original Gw.exe.')
            return
        if os.path.exists(out) and not messagebox.askyesno('Patch', f'{out} exists. Overwrite?'):
            return

        args = argparse.Namespace(input=gw, keys_file=keys, output=out, detach=False,
                                  file_server=False, detach_only=prefixes, detach_keep=None)

        def job():
            module = load_script('patch-gw.py')
            module.main(args)

        self._run_bg('Patch', job)

    def _do_hosts(self, action):
        path = self.hosts_path_var.get().strip()
        target = self.target_var.get().strip() or '127.0.0.1'
        file_server = self.hosts_file_server.get()
        if action == 'enable' and not messagebox.askyesno(
                'Enable hosts block',
                'This redirects the ArenaNet hostnames for every program on this machine, '
                'a retail client included, until you disable it.\n\nContinue?'):
            return

        def job():
            module = load_script('hosts-redirect.py')
            if not os.path.exists(path):
                raise FileNotFoundError(f'no hosts file at {path}')
            if action == 'status':
                module.status(path)
            elif action == 'enable':
                module.enable(path, target, file_server)
            else:
                module.disable(path)

        self._run_bg(f'Hosts: {action}', job)

    def _do_launch(self):
        exe = self.exe_var.get().strip()
        if not exe or not os.path.isfile(exe):
            messagebox.showerror('Launch', 'Pick Gw.custom.exe first.')
            return
        cmd = [exe, '-authsrv', self.auth_var.get().strip() or '127.0.0.1',
               '-portal', self.portal_var.get().strip() or '127.0.0.1']
        if not os.path.exists(os.path.join(os.path.dirname(exe), 'portal.dll')):
            self._log('[!] portal.dll is not next to the client: the auth connection will close '
                      'after the key exchange.')
        self._log('Launching: ' + ' '.join(cmd))
        try:
            subprocess.Popen(cmd, cwd=os.path.dirname(exe))
        except OSError as exc:
            self._log(f'[!] could not start the client: {exc}')

    def _elevate(self):
        if messagebox.askyesno('Administrator', 'Restart this program with administrator rights?'):
            relaunch_elevated()
            self.destroy()


def main():
    App().mainloop()


if __name__ == '__main__':
    main()
