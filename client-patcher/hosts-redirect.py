#!/usr/bin/env python3
"""Send the client's ArenaNet hostnames to this machine, or put them back.

A plain patched client is live: patch-gw.py rewrites the DH keys and the mutex and leaves every
hostname alone, so the game resolves ArenaNet's servers and only the auth and portal addresses come
from its command line. This script is the opt-in redirect, and it is the only thing that sends a name
to this machine. A client built with `--detach` needs this block plus a file server answering File1's
content; without both it cannot connect.

  python client-patcher/hosts-redirect.py status              what is on now
  python client-patcher/hosts-redirect.py enable              the six names, File1 staying live
  python client-patcher/hosts-redirect.py enable --file-server   all seven, the content address included
  python client-patcher/hosts-redirect.py disable             remove the block, everything else untouched

Options: --hosts PATH (default: the operating system's hosts file), --target ADDRESS (default
127.0.0.1 -- an address our server answers on).

Turning it on is a deliberate, machine-wide test setting: while the block is present, ANY client on
this machine that resolves these names is redirected to the target address, including a retail game.
That is why enable/disable/status exist and why every other line of the file is left byte for byte
alone. Off (the default), the names resolve normally and the client connects live.

Ported from GWPS `tools/hosts-gwps.ps1` (same block, same reversibility), as Python so it is the same
command on Windows and Linux.
"""

import argparse
import os
import shutil
import sys

BEGIN = '# --- ASURA private server ---'
END = '# --- end ASURA ---'

# The baked-in hostnames, the same set the patcher repoints. `bug` carries a port in the executable's
# string; the hosts file takes the name only.
HOSTNAMES = (
    'Auth1.ArenaNetworks.com',
    'File1.ArenaNetworks.com',
    'Diag.ArenaNetworks.com',
    'bug.arenanetworks.com',
    'icmp.arenanetworks.com',
    'ArenaNetworks.com',
    'account.arena.net',
)

# The one optional address: the content/file server. It is redirected only when asked for, and pairs
# with `patch-gw.py --file-server`.
FILE_SERVER = 'File1.ArenaNetworks.com'


def default_hosts_path() -> str:
    if os.name == 'nt':
        root = os.environ.get('SystemRoot', r'C:\Windows')
        return os.path.join(root, 'System32', 'drivers', 'etc', 'hosts')
    return '/etc/hosts'


def read(path: str) -> str:
    with open(path, 'rb') as f:
        return f.read().decode('utf-8', errors='surrogateescape')


def write(path: str, content: str) -> None:
    with open(path, 'wb') as f:
        f.write(content.encode('utf-8', errors='surrogateescape'))


def block_lines(content: str) -> list:
    """The hostname lines of the block currently in the file (empty when there is no block)."""
    names = []
    inside = False
    for line in content.splitlines():
        stripped = line.strip()
        if stripped == BEGIN:
            inside = True
            continue
        if stripped == END:
            break
        if inside and stripped and not stripped.startswith('#'):
            names.append(stripped)
    return names


def status(path: str) -> None:
    content = read(path)
    names = block_lines(content)
    if not names:
        print('disabled (default): every name resolves normally and the client connects live')
        return
    optional = 'file server (File1) included' if any(FILE_SERVER in n for n in names) else \
        'file server (File1) stays live, the optional address'
    print(f'enabled: {len(names)} hostname(s) resolve to this machine; {optional}')
    for line in names:
        print(f'  {line}')


def enable(path: str, target: str, file_server: bool) -> None:
    content = read(path)
    names = list(HOSTNAMES) if file_server else [n for n in HOSTNAMES if n != FILE_SERVER]
    backup = path + '.asura-backup'
    if not os.path.exists(backup):
        shutil.copyfile(path, backup)
        print(f'backed up the original to {backup}')
    # Replace the block if one is present, so the setting can be changed in place.
    kept = []
    skipping = False
    for line in content.splitlines(keepends=True):
        stripped = line.strip()
        if stripped == BEGIN:
            skipping = True
            continue
        if stripped == END:
            skipping = False
            continue
        if not skipping:
            kept.append(line)
    base = ''.join(kept)
    newline = '\r\n' if '\r\n' in base else '\n'
    if not base.endswith(newline) and base:
        base += newline
    if base:
        base += newline
    lines = [BEGIN, '# client-patcher/hosts-redirect.py - remove with the disable action']
    lines += [f'{target} {name}' for name in names]
    lines.append(END)
    write(path, base + newline.join(lines) + newline)
    print(f'enabled: {len(names)} hostname(s) resolve to {target} (block written to {path})')


def disable(path: str) -> None:
    content = read(path)
    if not block_lines(content):
        print('already disabled')
        return
    kept = []
    skipping = False
    for line in content.splitlines(keepends=True):
        stripped = line.strip()
        if stripped == BEGIN:
            skipping = True
            continue
        if stripped == END:
            skipping = False
            continue
        if not skipping:
            kept.append(line)
    write(path, ''.join(kept))
    print(f'disabled: the block was removed from {path}; every other entry is untouched')


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('action', choices=['status', 'enable', 'disable'])
    parser.add_argument('--hosts', default=default_hosts_path(),
                        help='the hosts file to edit (default: the operating system\'s)')
    parser.add_argument('--target', default='127.0.0.1',
                        help='the address the hostnames resolve to (default 127.0.0.1)')
    parser.add_argument('--file-server', action='store_true',
                        help='also redirect the optional address, File1.ArenaNetworks.com '
                             '(pair with patch-gw.py --file-server)')
    args = parser.parse_args()

    if not os.path.exists(args.hosts):
        print(f'no hosts file at {args.hosts}', file=sys.stderr)
        return 2

    try:
        if args.action == 'status':
            status(args.hosts)
        elif args.action == 'enable':
            enable(args.hosts, args.target, args.file_server)
        else:
            disable(args.hosts)
    except PermissionError:
        print(f'cannot write {args.hosts}: run this from an elevated shell (on Windows, the hosts '
              f'file needs administrator rights)', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
