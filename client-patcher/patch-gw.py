#!/usr/bin/env python3
# Patches a Windows Gw.exe into Gw.custom.exe. Runs on Windows and Linux: it reads and writes a PE
# image and calls no Windows API, so the platform only matters for where the file lives. The client
# itself still runs on Windows; this script does not.
import os
import sys
import struct
import argparse

try:
    import pefile
except ImportError:
    sys.exit(
        'patch-gw.py needs the pefile module: install it with `pip install pefile` '
        '(or `pip3 install pefile` on Linux)'
    )

def reverse_endian(data):
    return data[::-1]

def parse_key_file(file_path):
    lines = open(file_path, 'r').read().splitlines()
    for line in lines:
        key, val = tuple(map(lambda val: val.strip(), line.split('=')))
        if key == 'prime':
            prime_mod = int(val)
        elif key == 'server_public':
            public_key = int(val)
        elif key == 'generator':
            prim_root = int(val)
    return prim_root, prime_mod, public_key

def main(args):
    input_path = args.input
    if args.output:
        out_path = args.output
    else:
        name, ext = os.path.splitext(input_path)
        out_path = name + '.custom' + ext

    print('[+] Input:', input_path)
    print('[+] Output:', out_path)
    print('[+] Key\'s file:', args.keys_file)

    prim_root, prime_mod, public_key = parse_key_file(args.keys_file)

    print('[+] Searching for the keys in the executable...')
    gw_pe = pefile.PE(input_path)
    for section in gw_pe.sections:
        if b'.text' in section.Name:
            text_section = section
        if b'.rdata' in section.Name:
            rdata_section = section

    if not text_section:
        print(f"Couldn't find '.text' section in executable '{input_path}'")
        sys.exit(1)

    if not rdata_section:
        print(f"Couldn't find '.rdata' section in executable '{input_path}'")
        sys.exit(1)

    text_sec_data = text_section.get_data()
    rdata_section_data = rdata_section.get_data()

    found = text_sec_data.find(b'\x8B\x45\x08\xC7\x00\x88\x00\x00\x00\xB8')
    if found < 0:
        print("Couldn't find the accessor of the Diffie-Hellman keys in text section")
        sys.exit(1)

    keys_addr,  = struct.unpack_from('<L', text_sec_data, found + 0xA)
    keys_rva    = keys_addr - gw_pe.OPTIONAL_HEADER.ImageBase
    print('[+] Found DHL keys at address:', hex(keys_addr))
    print('[+] DHL keys are at RVA:', hex(keys_rva))

    # the first 4 bytes are unknown, but always 1
    key_file_pos = gw_pe.get_offset_from_rva(keys_rva) + 4

    print('[+] Searching for mutex patch')
    found = text_sec_data.find(b'\x8B\xF8\x85\xFF\x74\x11\xFF\xD6\x3D\xB7')
    if found < 0:
        print("Couldn't find the code to patch `CreateMutexA`")
        sys.exit(1)

    mutex_patch_rva = found + text_section.VirtualAddress + 0x8
    print('[+] mutex patch rva is:', hex(mutex_patch_rva))
    mutex_patch_file_pos = gw_pe.get_offset_from_rva(mutex_patch_rva)

    print('[+] Searching for mutex name')
    mutex_name_file_pos = rdata_section_data.find(b'AN-Mutex-Window')
    if mutex_name_file_pos < 0:
        print("Couldn't find the mutex name in .rdata")
        sys.exit(1)
    mutex_name_file_pos += rdata_section.PointerToRawData
    print(f'[+] mutex_name_file_pos is 0x{mutex_name_file_pos:X}')

    gw_pe.close()

    print('[+] Patching executable...')
    with open(input_path, 'rb') as f:
        data = f.read()

    # ---- repoint the baked-in ArenaNet hostnames: opt-in only ------------------
    #
    # The address this client actually dials comes from the command line
    # (`Gw.custom.exe -authsrv 127.0.0.1 -portal 127.0.0.1`), not from this patch. So a plain run
    # leaves every hostname alone: the client stays live for everything the command line does not
    # cover, and that build connects (measured 2026-11-02, `Gw.custom.exe`).
    #
    # Repointing is a separate, deliberate step, because only one of these strings changes where the
    # client looks. `ArenaNetworks.com` at file offset 0x64EB40 is element 0 of the client's DNS
    # base-name table (`s_dnsBaseNames`, next to `NCPlatform.net` and the assert
    # `arrsize(s_dnsBaseNames)` named in `P:\Code\Base\Services\DnsName.cpp`). Names are composed at
    # run time as `<leaf>.<base>`, so that one string decides every lookup at once, while the leaf
    # strings below are inert: measured 2026-11-02, a build with the leaves repointed and the base
    # whole connects, a build with the base repointed alone does not.
    #
    # A base-repointed client needs its composed names answered, so `--detach` is a pair with the
    # hosts block (`hosts-redirect.py enable --file-server`) and a file server for the content the
    # client streams. Without that provisioning it cannot connect; that is a provisioning gap, not a
    # patch verdict. The file check itself is answered by the auth service.
    #
    # Each string is rewritten in place as `127.0.0.1` padded with NULs, which keeps every length and
    # section offset intact.
    #
    # Selection:
    #
    #   (default)                 nothing is repointed; the client stays live
    #   --detach                  repoint all seven, the DNS base entry included (needs the hosts
    #                             block and a file server)
    #   --file-server             same set as --detach, named for the optional content address
    #   --detach-only <prefix>    repoint only the matching host(s). Repeatable.
    #   --detach-keep <prefix>    repoint all except the matching host(s). Repeatable.
    #
    # Prefixes: Auth1, File1, Diag, icmp, bug, ArenaNetworks.com, account.arena.net
    ALL_HOSTS = (
        b'Auth1.ArenaNetworks.com',
        b'File1.ArenaNetworks.com',
        b'Diag.ArenaNetworks.com',
        b'bug.arenanetworks.com:6112',
        b'icmp.arenanetworks.com',
        b'ArenaNetworks.com',
        b'account.arena.net',
    )

    def matches(host, selects):
        name = host.decode().lower()
        return any(name.startswith(s.lower()) for s in selects)

    only = getattr(args, 'detach_only', None) or []
    keep = getattr(args, 'detach_keep', None) or []
    if only:
        HOSTS = tuple(h for h in ALL_HOSTS if matches(h, only))
        print(f'[+] --detach-only selected {len(HOSTS)} host(s)')
    elif keep:
        HOSTS = tuple(h for h in ALL_HOSTS if not matches(h, keep))
        print(f'[+] --detach-keep left out {len(ALL_HOSTS) - len(HOSTS)} host(s), repointing {len(HOSTS)}')
    elif getattr(args, 'detach', False):
        HOSTS = ALL_HOSTS
        print('[+] --detach: repointing all seven, the DNS base entry included; this build needs '
              'the hosts block and a file server')
    elif getattr(args, 'file_server', False):
        HOSTS = ALL_HOSTS
        print('[+] --file-server: repointing all seven, the optional content address included')
    else:
        HOSTS = ()
        print('[+] default: no hostname is repointed, the client stays live '
              '(--detach repoints them all, and needs the hosts block and a file server)')
    REPLACEMENT = b'127.0.0.1'

    def is_host_char(b):
        return (0x30 <= b <= 0x39) or (0x41 <= b <= 0x5a) or (0x61 <= b <= 0x7a) or b in (0x2e, 0x2d)

    print('[+] Searching for baked-in ArenaNet hostnames')
    host_patches = []
    for host in HOSTS:
        start = 0
        while True:
            idx = data.find(host, start)
            if idx < 0:
                break
            start = idx + 1
            # Only complete C strings: not part of a longer hostname, and NUL-terminated.
            if idx > 0 and is_host_char(data[idx - 1]):
                continue
            if idx + len(host) < len(data) and data[idx + len(host)] != 0:
                continue
            host_patches.append((idx, host))
    for (idx, host) in host_patches:
        print(f'[+] {host.decode():<26} at 0x{idx:X} -> {REPLACEMENT.decode()}')

    def patch(data, offset, replace):
        data[offset:offset+len(replace)] = replace
        return data

    data_mut = list(data)
    data_mut = patch(data_mut, key_file_pos, prim_root.to_bytes(4, byteorder='little'))
    data_mut = patch(data_mut, key_file_pos + 4, prime_mod.to_bytes(64, byteorder='little'))
    data_mut = patch(data_mut, key_file_pos + 68, public_key.to_bytes(64, byteorder='little'))
    data_mut = patch(data_mut, mutex_patch_file_pos, b'\x31\xC0\x90\x90\x90\x0F\x84')
    data_mut = patch(data_mut, mutex_name_file_pos, b'AN-Futex')
    for (idx, host) in host_patches:
        data_mut = patch(data_mut, idx, REPLACEMENT + b'\x00' * (len(host) - len(REPLACEMENT)))

    data = bytes(data_mut)
    with open(out_path, 'wb') as f:
        f.write(data)

    print("[+] Patched executable written here '%s'" % out_path)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description='Patch Gw executable with custom DHL keys', add_help=True)
    parser.add_argument('input')
    parser.add_argument('keys_file')
    parser.add_argument('--output', metavar='output', type=str, help='The output file', required=False)
    parser.add_argument('--detach', action='store_true',
                        help='Repoint all seven baked-in ArenaNet hostnames, the DNS base entry '
                             'included. Needs client-patcher/hosts-redirect.py enable '
                             '--file-server and a file server; without that provisioning the '
                             'client cannot connect.')
    parser.add_argument('--file-server', action='store_true',
                        help='The same set as --detach, named for the optional content address '
                             'File1.ArenaNetworks.com. Off by default: the client stays live.')
    parser.add_argument('--detach-only', action='append', metavar='PREFIX',
                        help='Repoint only the hostname(s) starting with PREFIX. Repeatable.')
    parser.add_argument('--detach-keep', action='append', metavar='PREFIX',
                        help='Repoint all hostnames except those starting with PREFIX. Repeatable.')

    args = parser.parse_args()
    main(args)
