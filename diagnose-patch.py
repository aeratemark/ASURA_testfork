#!/usr/bin/env python3
"""Read-only checks of a GUI- or CLI-patched executable against its original.

Requires Python 3 and pefile, like patch-gw.py. No executable is launched or changed.
Exit 0: checked patch bytes match; exit 1: a check failed; exit 2: input/tool error.
Hostname choices and differences from --known-good are reported, not rejected.
This checks file contents, not client login or network reachability.
"""

import argparse
import hashlib
import struct
from pathlib import Path

try:
    import pefile
except ImportError:
    raise SystemExit('Install the dependency: python -m pip install pefile')


HOSTS = (
    b'Auth1.ArenaNetworks.com', b'File1.ArenaNetworks.com',
    b'Diag.ArenaNetworks.com', b'bug.arenanetworks.com:6112',
    b'icmp.arenanetworks.com', b'ArenaNetworks.com', b'account.arena.net',
)


def occurrences(data, name):
    """Find complete C strings, using the patcher's hostname boundary rule."""
    start = 0
    hostname_chars = b'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-'
    while True:
        offset = data.find(name, start)
        if offset < 0:
            return
        start = offset + 1
        end = offset + len(name)
        if offset and data[offset - 1] in hostname_chars:
            continue
        if end < len(data) and data[end] != 0:
            continue
        yield offset


def unique_offset(data, signature, description):
    offset = data.find(signature)
    if offset < 0 or data.find(signature, offset + 1) >= 0:
        raise ValueError(description + ': expected one signature in original executable')
    return offset


def describe(path):
    data = path.read_bytes()
    print(str(path.resolve()))
    print('  size={} SHA256={}'.format(len(data), hashlib.sha256(data).hexdigest()))
    return data


def run(args):
    print('ASURA patch diagnostics: no files or patch settings are changed')
    print('\nOriginal:')
    original = describe(args.original)
    print('Patched:')
    patched = describe(args.patched)
    failures = 0

    def check(passed, message):
        nonlocal failures
        print(('PASS: ' if passed else 'FAIL: ') + message)
        if not passed:
            failures += 1

    check(len(original) == len(patched), 'patched file has original length')
    with pefile.PE(data=original) as image, pefile.PE(data=patched) as output:
        check(image.FILE_HEADER.Machine == output.FILE_HEADER.Machine == 0x14c,
              'original and patched files are x86 PE images')
        sections = {section.Name.rstrip(b'\0'): section for section in image.sections}
        text = sections[b'.text']
        rdata = sections[b'.rdata']
        signature = bytes.fromhex('8B4508C70088000000B8')
        accessor = unique_offset(text.get_data(), signature, 'DH accessor')
        address = struct.unpack_from('<I', text.get_data(), accessor + len(signature))[0]
        key_offset = image.get_offset_from_rva(address - image.OPTIONAL_HEADER.ImageBase) + 4
        mutex = unique_offset(text.get_data(), bytes.fromhex('8BF885FF7411FFD63DB7'),
                              'mutex code')
        mutex_offset = text.PointerToRawData + mutex + 8
        name_offset = rdata.PointerToRawData + unique_offset(
            rdata.get_data(), b'AN-Mutex-Window', 'mutex name')

    keys = {}
    for line in args.keys.read_text(encoding='ascii').splitlines():
        line = line.strip()
        if line and not line.startswith('#'):
            key, value = line.split('=', 1)
            keys[key.strip()] = int(value.strip())
    check(pow(keys['generator'], keys['server_private'], keys['prime']) == keys['server_public'],
          'keys.txt public key agrees with its generator, private key and prime')
    core = (
        (key_offset, keys['generator'].to_bytes(4, 'little'), 'DH generator'),
        (key_offset + 4, keys['prime'].to_bytes(64, 'little'), 'DH prime'),
        (key_offset + 68, keys['server_public'].to_bytes(64, 'little'), 'DH public key'),
        (mutex_offset, bytes.fromhex('31C09090900F84'), 'mutex code'),
        (name_offset, b'AN-Futex', 'mutex name'),
    )
    allowed = bytearray(len(original))
    for offset, expected, name in core:
        check(patched[offset:offset + len(expected)] == expected,
              '{} at file offset 0x{:X}'.format(name, offset))
        allowed[offset:offset + len(expected)] = b'\x01' * len(expected)

    print('\nHostname settings (ORIGINAL and LOOPBACK are supported patch choices):')
    for host in HOSTS:
        offsets = list(occurrences(original, host))
        if not offsets:
            print('INFO: {} not found in original'.format(host.decode('ascii')))
        for offset in offsets:
            actual = patched[offset:offset + len(host)]
            loopback = b'127.0.0.1' + b'\0' * (len(host) - len(b'127.0.0.1'))
            state = 'ORIGINAL' if actual == host else 'LOOPBACK' if actual == loopback else 'OTHER'
            print('  {} at 0x{:X}: {}'.format(host.decode('ascii'), offset, state))
            if state == 'OTHER':
                check(False, 'hostname bytes match neither original nor loopback replacement')
            allowed[offset:offset + len(host)] = b'\x01' * len(host)

    unexpected = [i for i, (a, b) in enumerate(zip(original, patched))
                  if a != b and not allowed[i]]
    check(not unexpected, '{} changed byte(s) outside the patcher\'s known regions'.format(
        len(unexpected)))
    if unexpected:
        print('  First offsets: ' + ', '.join('0x{:X}'.format(i) for i in unexpected[:16]))

    if args.known_good:
        print('\nKnown working executable:')
        working = describe(args.known_good)
        count = sum(a != b for a, b in zip(working, patched)) + abs(len(working) - len(patched))
        print('INFO: {} differing byte(s) from known working executable'.format(count))
        if working == patched:
            print('INFO: patched output is byte-for-byte identical to known working executable')
        else:
            print('INFO: a difference alone does not identify a faulty patch or matching client build')
            for host in HOSTS:
                for offset in occurrences(original, host):
                    end = offset + len(host)
                    if working[offset:end] != patched[offset:end]:
                        print('  Hostname differs: {} at 0x{:X}'.format(host.decode('ascii'), offset))

    print('\n{} failed check(s). No client connection was tested.'.format(failures))
    print('Send this output, the GUI patch log, launch command and exact client error.')
    return 1 if failures else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('original', type=Path, help='unmodified Gw.exe used by the GUI')
    parser.add_argument('patched', type=Path, help='Gw.custom.exe produced by the GUI')
    parser.add_argument('--known-good', type=Path, help='optional executable that logs in successfully')
    parser.add_argument('--keys', type=Path, default=Path(__file__).with_name('keys.txt'))
    args = parser.parse_args()
    try:
        return run(args)
    except (OSError, ValueError, KeyError, OverflowError, struct.error, pefile.PEFormatError) as error:
        print('ERROR: {}'.format(error))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
