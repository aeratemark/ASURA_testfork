# Recordings — public research evidence

This folder is the project's archive of **faithful recordings of the client and the servers
talking to it**. It is public on purpose: the recordings are the research material that lets anyone
check how the game actually behaves, without needing our servers, the reference, or a client.

They are **evidence, not instructions.** A recording tells you what happened; it never dictates
what to build. The real client remains the acceptance test for any change.

## The layout

```
recordings/
  asura/     recordings made by THIS project's server
  gwps/      recordings of the external reference (historical, frozen)
  client/    client-side recordings (the sniff tool, capturing Gw.exe directly)
```

Each of those contains the **kind**, and every kind has an optional **subject** (only for
`feature/`):

| kind | what it is |
|---|---|
| `login/` | the auth session: handshake, login batch, character list |
| `world/` | one world process's message trace, one file per run |
| `session/` | a whole client session (login -> travel -> maps -> exit) |
| `feature/<subject>/` | one subject per recording: `movement`, `items`, `travel`, `chat` |

## Naming: the subject first, the date only to disambiguate

The name says **what the recording is**, in words a person would use. The date is a disambiguator,
never the name:

```
<subject>.<ext>                      the only recording of its subject in its folder
<subject>-<YYYYMMDD>.<ext>           a second one shares the subject with another day
<subject>-<YYYYMMDD>-<HHMMSS>.<ext>  several share the subject on the same day
```

Examples:

```
live-client-session-20261027-220258.jsonl        one of seven sessions against our world, 27 Oct
live-client-session-20261027-223528.jsonl
load-stream.log                                   the reference's load stream (unique subject)
login-travel-session.log                          the reference's login -> travel session
movement-sniff.log                                the client-side movement sniff
```

- Tools that name their own output (the sniff tool, `play.ps1 -Capture`) include the full
  `YYYYMMDD-HHMMSS`, because they cannot check the folder for collisions.
- **Never edit a recording.** A re-record is a new file.
- When a new recording collides with an undated one, the old one is renamed to the dated form and
  `index.jsonl` is updated — the date appears exactly when it earns its place.
- The full time, timezone, build and tool live in `index.jsonl`; the filename only carries what a
  human needs.
- Client-side captures must be **sanitised** before they land here (the capture tool masks names
  and account data, offsets preserved); the index says what was sanitised.

## The index

`index.jsonl` is one JSON line per recording — the way to search at thousands of files:

```json
{"source":"asura","kind":"world","subject":"live-client-session","path":"asura/world/live-client-session-20261027-220258.jsonl",
 "recorded":"2026-10-27T22:02:58Z","tool":"asura-world --capture","world":8001,
 "client_build":39112,"tags":["load","session"],"note":"..."}
```

- Add a line when you add a recording; the fields are `source`, `kind`, `subject`, `path`,
  `recorded` (ISO-8601), `tool`, and then whatever the recording knows: `world`, `client_build`,
  `maps`, `tags`, `note`.
- The index is the search layer; the folders are for humans.

## The sources, honestly

- `asura/` — made by the server in this repository (`asura-world --capture` and friends). This is
  **our** evidence.
- `gwps/` — made by the external reference server. Historical: the reference is no longer the
  specification; these files are kept as the record of what it did, not as expected answers.
- `client/` — made by looking at the client itself (the sniff tool, `Gw.exe` both directions).
  The server it talked to is not part of the capture; the index note says so when it is unknown.
