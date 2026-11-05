# AGENTS.md

Binding rules for any agent working in this repository. Read this file before the first edit of a
session. It is deliberately short: the detail, the evidence and the incident ledger live in
`docs/HANDOFF.md`, which is read on demand and never duplicated here.

The owner's words, 2026-10-28: "reproduce faithfully the packet stream of the live server/client".
Every rule below serves that one sentence.

## The sources, in the order they answer

Five projects outside this repository are read here, and none is a dependency (`# 6`). The `first` column
is the source that answers the question; the `then` column is the cross-check.

| the question | first | then | governed by |
|---|---|---|---|
| what the **running client** does: its state, its archive, what it sends and receives, **and its own catalogs** - every map's id, name, region, campaign and blob id | **Py4GW Stealth**, by hand - 32-bit Python, elevated, `tools/sniff-capture.py`, `tools/gen-portal-table.py` | a client-side capture under `recordings/client/` | `docs/adr/0013-py4gw-stealth-is-our-link-to-the-client.md` |
| what the **client does with** what we send: asserts, codecs, enums, map dimensions | Eden | a crash log read by hand | `docs/adr/0012-eden-is-a-source.md` |
| what our **server must send**, byte for byte | a wire capture: `recordings/gwps/`, `recordings/asura/` | GWPS's own C source | `docs/03-protocol.md`, `docs/adr/0010-gwps-is-the-floor.md` |
| what **Rurik** measured, and what is left to port | `docs/20-rurik-gap.md` | Rurik's `studies/` | `docs/adr/0011-rurik-is-a-source.md` |
| **structure**: processes, where state lives, instances, the database | AzerothCore's tree | `docs/02-architecture.md` | `docs/adr/0006-azerothcore-is-the-reference.md` |

- **Py4GW Stealth comes first for anything about the client.** It reads the archive through the client's
  own open-file, image and `AMET` code, and it reads what the client sends and receives at the handler
  table pointer and the send wrapper the client itself uses. No other source here sees the client
  running, so a question that needs the client running is answered there and not by a document.
- **Its catalogs are complete, and the naming is already solved.** A map name is not a per-map lookup:
  the client's own tables name every map - 278 outposts and 378 explorables, id to name and name to id
  (`name_to_map_id`). Beside them one `AreaInfo` record per map id carries that map's campaign, region,
  type, party sizes and `Gw.dat` blob id. Py4GW carries both, read-only
  (`py4gw.enums_src.map_enums`, `py4gw.context.instance_info_context.AreaInfoStruct`; Reforged ships the
  same tables plus `GetMapInfo(map_id)`). So no map fact is guessed, hand-listed or scraped from a wiki:
  a name comes from the enum, the rest from the record. Reforged is the client patch this server's keys
  and mutex patches pair with; Stealth is the read-only link to it. The names are copied into this
  repository as a reference (`docs/39-map-list.md`), so the table is not read out of a checkout twice.
- Its packages carry no licence grant, so what crosses back is a measurement in our words with our
  recording named beside it - never a file, an offset, a byte pattern or a table.
- A wire size comes from a wire capture, never from the sniffer: the sniffer reports the client's message
  struct, which is the client's own bookkeeping (`docs/03-protocol.md` 5e).
- **The real client in the owner's hands outranks all five.** Where a source and the live client
  disagree, the client wins and the disagreement is reported, not split.
- The table is a reading order, not a licence: every fact taken from a source is restated in our words
  with our own recording, crash log or test named beside it. Where a source is the only witness, the
  sentence says so.

## 1. The objective: the packet stream is the specification

- This is reverse engineering, not server development. The deliverable is the packet stream: the same
  headers, in the same order, at the same sizes, with the same field layout and the same values the
  reference puts in those fields, produced by the same trigger.
- Copy the reference verbatim. A packet whose content looks wrong is reproduced exactly as the
  reference sends it, and the oddity is reported. A faithful copy can be corrected later; a corrected
  copy hides what the live server does.
- Decode and name every handled packet: header, direction, size, field layout and inferred function,
  named from evidence, with the evidence written next to the name. Write a guess as a guess. List an
  unhandled packet as unhandled.
- "Works" means a real client behaves as it does against the live server. A green test is not proof of
  parity; the client is.

## 2. The command is the requirement

- Do exactly the command. When the owner says port it, port it. When the owner says fix X, change X
  and nothing else. The instruction is the specification, not a starting point for improvements.
- An explanation is not an instruction. When the owner explains how something in the game works, that
  is context for reading the reference, and it changes no behaviour.
- A measurement is evidence, not permission. Capture it, report it, wait.
- Report, do not decide, when the reference's own behaviour looks wrong. The owner decides, and the
  decision joins the ledger in `docs/HANDOFF.md`.
- Restore first, discuss second. When something that worked stops working, the first commit restores
  the last known-good behaviour.
- One change, one subject. A port, a fix and an improvement never travel in the same commit.
- Never re-decide a decision already made. The decisions table is in `docs/HANDOFF.md`.

### Before any behaviour change: the pre-flight sentence

Write one sentence: what changes, in which message or code path, and why it is not simply copying the
reference. If that sentence contains anything the owner did not ask for, stop and ask instead of doing
it. Name the source of truth being copied, as a `file:line` in the reference or the captured bytes,
and quote it. "I believe" is not a source.

Asking costs one turn. A regression costs one of the owner's sessions.

## 3. Testing: one targeted run, and the client is the judge

A test run starts here, not from a habit:

```powershell
.\tools\test-touched.ps1        # print what the change touches, run nothing
.\tools\test-touched.ps1 -Run   # run exactly the targeted commands
```

- Test what the change touched, nothing else. A change to one message's wire form runs that message's
  tests plus whatever crate must compile. It does not run the workspace, the battery, or every suite
  because a suite exists.
- The standing suites are a regression net. Run them when the change reaches shared machinery
  (framing, the generated tables, schema or migrations, start-up) or immediately before a commit that
  claims a milestone, and never per edit, per fix or per message.
- A whole-suite run needs its reason, and the reason is the sentence the commit carries:
  `.\tools\test-touched.ps1 -WholeSuite -Justification "milestone: M5 landing" -Run`.
- Build no new scaffolding for a small change. No new harness, fixture, oracle or captured-byte
  comparison unless its absence would let a regression through that the live client would not catch
  on its own.
- A test that encodes superseded behaviour is updated or deleted in the same change. It is never
  extended, never worked around, and never the reason to keep an old behaviour alive.
- Time budget: for a minor feature, testing stays smaller than the implementation. When the testing is
  about to take longer than the change, stop and let the live client decide.
- Report a run in one line. Not a narrative of every test, and not a second run to confirm the first.
- The ten-minute soak starts only with `ASURA_SOAK=1` set on top of `#[ignore]`.

## 4. The answering gate

Every answer to the owner passes this gate, or it is rewritten before it is sent. It covers chat, a
report, a commit message and the status lines of a run.

The gate governs answers that carry work. It does not govern **brainstorming mode**, defined at the end
of this section, where a decision is the defect and the inventory is the answer.

The shape of a compliant answer:

1. The first sentence is the answer. Not background, not a restatement of the question, not the list
   of sources consulted.
2. Then the facts it rests on, one claim per sentence, each with a `file:line`, a command, or a number.
3. Then what to do, as numbered steps a person can run or apply.
4. If there is a real fork, name the branch taken and why, in one line. A menu with no decision is a
   failed answer - outside brainstorming mode, where that sentence is reversed.

The gate, all six:

- Sentence one is the answer.
- It names a file, a command, or a number.
- A shorter answer would not carry the same information.
- No sentence is there to sound thorough.
- It ends with an action or a question, not an offer to explain more.
- Every word is the owner's or plain English.

Words that do not appear in an answer: `consider`, `may`, `might`, `could` where it means will or
should, `it is worth noting`, `importantly`, `note that`, `in order to`, `leverage`, `attest`,
`afford`, `instrument` as a metaphor, `frame`, `ladder`, `surface` as a verb, `nuanced`, `robust`,
`holistic`, `option 1` / `option 2` with no recommendation, `let me walk you through`, `to be clear`,
`as I said`. Restating a rule in different words in the same answer is also a defect: one claim, one
sentence, once.

### Brainstorming mode

The owner opens this mode by asking for an evaluation, a brainstorm, or the options: "i need to evaluate
this", "what setups am i unaware of". The mode governs that answer and every answer after it until the
owner closes it by asking for a decision or naming the axis that matters.

What this mode forbids:

- **A final answer.** No decision, no `Fork taken:`, no numbered plan of what to do, and no
  recommendation the owner did not ask for. An unasked recommendation is a design decision wearing an
  evaluation's clothes, and it is the failure this mode exists to stop.
- **A requirement invented from a goal.** "Users can contribute scripts" is context for what to compare,
  not a requirement to derive and then satisfy. The owner's goals stay the owner's.
- **One axis standing in for the evaluation.** Speed is one row of ten. The axes: embedding into Rust
  (ABI, C toolchain, Linux cross-build), threading and per-map parallelism, memory per instance,
  abortability and sandbox, determinism for replay, hot reload, debugger and tooling, library ecosystem,
  deployment, author experience. An axis that was not measured is named as not measured, in the answer.
- **A number used as a verdict.** A measurement is evidence (`# 2`). It becomes a decision only when the
  owner reads it as one.
- **The reference as an argument.** "The reference does it this way" settles a structural question. It
  settles nothing about what the owner should choose.
- **Solving a problem nobody has yet.** An evaluation that ends in an architecture for something not
  built is not an evaluation.

What the mode asks for instead: each candidate, its limits axis by axis, one claim per line with its
evidence, the setups the owner has not listed, and the list of what is still not measured. It ends with
the question that would sharpen it, never with a conclusion.

## 4a. Values are configuration, decisions are code, and a rule reads only

Three kinds of thing, kept apart (`docs/38-layer-decisions.md`, D34, D35):

1. **The client's vocabulary** - `MapType` 0/1/11, message headers, field layouts. The wire layer's,
   documented there. A file may *state* these values for review and must mark them `immutable`:
   changing one does not change what the client does, it only stops this server recognising what its
   own client sends.
2. **Values that describe the world** - who spawns where, where a character with no history enters, the
   map catalogue, the cadences this machine runs at. These are **files**, never compiled constants.
3. **Decisions** - "this destination is not an outpost, so put the character in its last one". These are
   `asura-rules`, which takes 1 and 2 as arguments, holds no value of its own, reads no file, opens no
   connection and prints nothing.

The rules that follow from that, and they are binding:

- **No hardcoded value, even when the contract never changes.** A compiled number is invisible and
  unreachable; it goes in a file so it can be read and reviewed.
- **Where the files go.** `config/Server/` for what this machine runs on (the intervals, the values a
  world falls back on - `asura-config` reads them). `config/World/<pack>/` for what the world contains,
  one JSON per concern, `Vanilla` as the faithful base (`docs/38`, D23, D24).
- **What a config file holds: values.** One key per value, each with a sentence saying what it means. It
  holds **no provenance and no reporting**: "measured", "implemented", "unverified" are not
  configuration. Evidence lives in `docs/` with the recording it names beside it - the file answers
  *what it is*, the document answers *why we believe it*.
- **`immutable` is a note to whoever edits the file**, not a check and not a policy: `true` means do not
  change this value, because it is one of the client's own numbers and changing it breaks what the client
  expects. It appears only where that is true. A value the server chooses carries no tag, and the owner may
  change it. Never use `immutable` to mean "we would rather you did not".
- **Values with no answer say so.** A key whose map has not been established is `null`, and the caller
  refuses the action with a logged reason instead of choosing on the player's behalf.
- **A layer's file list is not a licence.** Before adding a number to a constant, ask which of the three
  kinds it is; two of them are not code.

## 5. Every file is ASCII

```powershell
.\tools\ascii-check.ps1     # exit 0 = clean, exit 1 = offender listed
```

- Every file written here is ASCII: source, comments, documents, commit messages, scripts, config,
  test expectations. Write `-`, `->`, `=>`, `"`, `'`, `...`, `#`.
- `recordings/` is the one exception. Those are bytes captured from a live client, evidence rather
  than writing, and they are never edited.
- Never rewrite a file with a `Get-Content` / `Set-Content` round trip. Windows PowerShell reads a
  BOM-less file as ANSI and writes the corruption back. Use the file tools, or
  `[System.IO.File]::ReadAllText` with explicit UTF-8.

## 6. The dependency rule

- No external project is a dependency of this repository's code. `Cargo.toml` names none, no test
  calls one, and every service builds, starts and runs with that project absent from the machine.
- A tool in `tools/` is allowed to use another project's library to measure something. Three
  conditions keep the line: it fails closed when the library is absent, nothing outside `tools/`
  mentions it, and what crosses back is bytes under `recordings/`.
- The vocabulary is ours as well. A module is named after the game's noun, a doc comment states the
  shape as this server's, and nothing in `crates/` describes the code as a copy of another project's
  file or class.

## 7. Where things are

| need | command or path |
|---|---|
| start everything | `asura-supervisor start` (the one start path on Windows and Linux) |
| runbook: what to run, what success looks like | `docs/19-running.md` |
| packet catalogue, framing, field layouts | `crates/asura-protocol/src/packets.rs`, `docs/03-protocol.md` |
| the socket layer, and the login path | `crates/asura-gateway/`, `crates/asura-auth/`, `docs/06-reference-session.md` |
| game rules a protocol answer needs | `crates/asura-rules/`, `docs/38-layer-decisions.md` |
| world, movement, pathing, inventory | `crates/asura-world/` |
| database, placement, inventory rows | `crates/asura-db/` |
| portal HTTP service | `crates/asura-webgate/` |
| service supervision | `crates/asura-supervisor/`, `config/services.toml` |
| the whole path in one command, no client needed | `.\tools\test-battery.ps1` |
| the sources: what answers a client, a server or a structure question | "The sources, in the order they answer", above |
| cargo is not on PATH in a fresh shell | `C:\Users\Apo\.cargo\bin\cargo.exe` |

- `docker/` is an optional bundle. It does not drive this project (owner's instruction, 2026-10-28).
  Start, test and demo with the supervisor.
- The reference capture the offline checks read is committed under `recordings/gwps/world/`
  (`recordings/README.md`). No reference checkout is consulted at runtime.
- The client is Windows-only. A world trace is `ASURA_TRACE=1`; every message both ways, decrypted,
  is `--capture PATH` or `ASURA_CAPTURE`.

## 8. Detail

`docs/HANDOFF.md` holds the incident ledger (eight recorded cases of a rule invented on top of an
instruction, or of a written rule ignored), why each rule exists, the decisions already made, the
environment state and what is built. Read it before deciding that something is missing.

Reading order for a fresh session: `docs/HANDOFF.md` -> `MILESTONES.md` -> `docs/01-scope.md` ->
`docs/02-architecture.md` -> `docs/03-protocol.md` -> `docs/roadmap.md` -> the ADR that matches the
question. For anything about the client, that ADR is
`docs/adr/0013-py4gw-stealth-is-our-link-to-the-client.md`, and it is read before the documents rather
than after them. `docs/00-glossary.md` defines every term.

When this file and `docs/HANDOFF.md` disagree, this file is newer and wins.

A correction replaces the text it corrects. It is never appended beside the old text, in a file or in
an answer.
