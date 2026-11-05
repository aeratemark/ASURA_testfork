# The client patcher

Commands first, each block run in order from the repository root. The paths assume the client is in
`F:\GW\GW1`; change them if yours is elsewhere.

## 1. Build the client (one command)

```powershell
pip install pefile                                                                          # once per machine
python client-patcher/patch-gw.py "F:\GW\GW1\Gw.exe" client-patcher/keys.txt --output "F:\GW\GW1\Gw.custom.exe"
```

Expected last two lines:

```
[+] default: no hostname is repointed, the client stays live (--detach repoints them all, and needs the hosts block and a file server)
[+] Patched executable written here 'F:\GW\GW1\Gw.custom.exe'
```

## 2. Run the client (one command)

```powershell
F:\GW\GW1\Gw.custom.exe -authsrv 127.0.0.1 -portal 127.0.0.1
```

Log in as `user1@example.com`, `user2@example.com` or `user3@example.com`; the web gate does not check
the password. `portal.dll` must sit next to the executable, and it is already staged in `F:\GW\GW1`.

Success: the character list appears, a character is picked, and an outpost loads.

## 3. Check the session (one command)

```powershell
Get-Content .\logs\auth.log -Tail 40 | Select-String '0x8038|0x8029|0x0009'
```

```
auth <- 0x8038 AUTH_CMSG_PORTAL_ACCOUNT_LOGIN    login accepted
auth <- 0x8029 AUTH_CMSG_REQUEST_GAME_INSTANCE   map requested
auth -> 0x0009 AUTH_SMSG_GAME_SERVER_INFO        world address handed over
```

All three lines means the client is in the game. No `0x8038`, with the session ending after `0x8023`,
means it stopped at the portal step: read "Redirected build" below.

## 4. Check which build you are running (one command)

```powershell
python client-patcher/hosts-redirect.py status
```

```
disabled (default): every name resolves normally and the client connects live
```

That is the working build. Anything else on that line means the machine is redirected, and it is
machine-wide.

## Redirected build: three commands, all three or it cannot connect

Only for a machine that also runs the content file server. Step 3 without steps 1 and 2 is the build
that stops before `0x8038`.

```powershell
# 1. elevated shell: write the machine-wide hosts block
python client-patcher/hosts-redirect.py enable --file-server

# 2. install the file server that answers the content the client streams from File1,
#    and the 21-byte file check - see the section on the file check below

# 3. build the matching client
python client-patcher/patch-gw.py "F:\GW\GW1\Gw.exe" client-patcher/keys.txt --output "F:\GW\GW1\Gw.custom.exe" --detach
```

Undo step 1 with `python client-patcher/hosts-redirect.py disable`, then rebuild with the command in
step 1 above.

## Diagnose a patch (reads files, launches nothing)

```powershell
python .\client-patcher\diagnose-patch.py "F:\GW\GW1\Gw.exe" "F:\GW\GW1\Gw.custom.exe" --known-good "F:\GW\GW1\Gw.custom.known-good.exe"
```

Exit 0 means the patch bytes match, 1 means a check failed, 2 means an input or tool error. It reports
SHA-256, the DH keys against `keys.txt`, both mutex patches, every hostname as `ORIGINAL` or
`LOOPBACK`, and bytes changed outside those regions. A difference from a known-good file is not by
itself a bad patch.

## The file check, and the numbers that keep moving

Before anything else on a gameplay-world socket the client asks which game files it should have: 21 bytes
in, 32 bytes out, six `Gw.dat` file ids and a zero (`docs/03-protocol.md` 7). Three things about it belong
in this folder because they decide whether a patched client can log in.

- **The patch does not answer it.** `patch-gw.py` rewrites the DH constants, the mutex and - only with
  `--detach` - the seven hostnames. The check is answered on the wire, so what has to be current is the
  answer, not the executable.
- **Who answers depends on the build.** A plain build asks ArenaNet and never involves us. A `--detach`
  build sends the file names to this machine, so whatever listens there answers - the socket layer
  (`crates/asura-gateway/src/lib.rs`, step 0) or the content file server you installed on that address. If
  your file server answers the check itself, its seven words need the same refresh as ours.
- **The seven words are the running client's build**, not a fixed constant: a new `Gw.exe`/`Gw.dat` brings
  a new set, and a stale set is a question answered in another build's terms. Refreshing it is a
  measurement - the commands, and what to record beside the set, are in `docs/19-running.md` ("Refreshing
  the file check's seven words"); the bytes live in `crates/asura-protocol/src/client/filecheck.rs`.
  Do not shift the old ids by arithmetic.
- **What it costs.** No answer at all is fatal - the client refuses with "Guild Wars is unable to connect
  to the ArenaNet network". Whether a stale answer is also fatal is not measured, so treat the set as
  required.

## Reference

### What each patch changes

| patch | why |
|---|---|
| Diffie-Hellman constants in `.text`/`.rdata` | the client and the server must run the same key exchange; the client carries a copy of the parameters |
| `CreateMutexA` code and mutex name (`AN-Mutex-Window` -> `AN-Futex`) | without it the client refuses to start when another copy is running, which kills every two-client test |
| the seven ArenaNet hostnames | **off by default**: a plain run leaves them alone and the client stays live. `--detach` repoints them at `127.0.0.1`, which is a pair with the hosts block and a file server |

The patcher does not touch the auth address: the client takes that from its command line. It does not
decompile anything; it finds byte signatures and overwrites the bytes in place.

### Why a plain run leaves the hostnames live

`ArenaNetworks.com` in the executable is not a name the client looks up, it is element 0 of its DNS
base-name table (`s_dnsBaseNames`, beside `NCPlatform.net` and the `DnsName.cpp` assert). Names are
composed at run time as `<leaf>.<base>`, so repointing it sends every lookup to `127.0.0.1` at once,
and the leaf strings are inert. Measured 2026-11-02: a client with the leaves repointed and the base
whole connects, a client with the base repointed alone cannot connect until its composed names are
answered by the hosts block and a file server.

### The hosts switch

The block is machine-wide: while it is in, any client on this machine resolving those names is
redirected, a retail one included.

```powershell
python client-patcher/hosts-redirect.py status     # the names in the block, or "disabled"
python client-patcher/hosts-redirect.py enable     # the six names, File1 staying live
python client-patcher/hosts-redirect.py disable    # remove only that block
```

The first `enable` copies the hosts file to `<hosts>.asura-backup`; `disable` leaves every other entry
alone.

### keys.txt and the server

`keys.txt` is not a setting to tune: `prime` and `server_private` are the two constants in
`crates/asura-protocol/src/client/handshake.rs` (`PRIME_MODULUS_DECIMAL`, `SERVER_PRIVATE_DECIMAL`). A
client patched with this file and this server agree on the handshake; if the server's constants ever
change, this file changes with them.

### Requirements, and building on Linux

- **Python 3** and **`pefile`** (`pip install pefile`); the game's own `Gw.exe`, which the game installs
  and this repository does not carry.
- The patcher is pure Python and `pefile`, with no Windows API calls, so it runs on Linux too and was
  run in a `python:3-slim` container against the rig's `Gw.exe`, producing a byte-identical
  `Gw.custom.exe`. The client itself is Windows-only.

```bash
pip3 install pefile
python3 client-patcher/patch-gw.py /path/to/Gw.exe client-patcher/keys.txt --output /path/to/Gw.custom.exe
```

### Provenance

`patch-gw.py` is the patcher from the GWPS project (`tools/patch-gw.py`), and `keys.txt` is that
project's keys file, whose parameters this server copied (`docs/04-from-gwps.md`). Both are kept here
verbatim so a clean checkout can produce the client the server expects; when the reference disappears
from disk, this folder is the copy that matters.
