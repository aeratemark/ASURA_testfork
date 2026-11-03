# ASURA control panel: operations, research, tests, and content

Design draft, 2026-11-02. The owner expanded the requested manager beyond map content editing:
it must start/stop and handle the server, launch and manage Py4GW Stealth sniffing sessions,
read in-game data through Py4GW Stealth, provide the user-facing place to conduct tests, and
serve as the configuration and management console. The owner explicitly confirmed that Stealth
is a major research capability, not merely a packet-capture attachment.

This defines the requested capabilities and a proposed interface/workflow. It does not select
desktop versus browser, a UI framework, remote-host support, or an implementation language for
the panel. Those choices remain questions before dependent implementation.
The simulator remains Rust, content scripting remains Lua, and the server start path remains
asura-supervisor. No process, capture, test, or configuration edit was started by writing this.

## Product intention

The owner should be able to answer the following questions from one interface:

- Is my server running, and what needs attention?
- Which client and environment am I recording, and where is the evidence?
- What does the running client actually hold: its map, agents, party, items, skills, and quests?
- What test am I performing, what steps remain, and what happened?
- What configuration/content is installed, and how do I change it correctly?

Commands and protocol details can remain available in expandable details; the main workflow
uses task names, client choices, clear status, instructions, and results. A user should not have
to reconstruct a session from several terminals and unnamed capture files.

The control panel does not replace the game client as the gameplay acceptance test. It guides
the operator, starts the selected tools, records outcomes, and keeps evidence connected.
Selecting a target, starting a capture, and changing content are separate user actions.

Py4GW Stealth integration is a core requested panel capability. Its external runtime remains
optional for operating ASURA itself: a machine without the research rig can still start/stop
and manage the server. Missing research setup disables the affected research actions with a
clear explanation; it does not turn the whole control panel into an unavailable application.

## Requested scope and proposed navigation

| Workspace | User-facing purpose | Initial information and actions |
|---|---|---|
| Overview | Know what is happening now | Service readiness, active captures/test runs, recent results, concrete setup failures |
| Server | Operate ASURA | Start/stop through the supervisor, restart a selected service, inspect status and logs |
| Capture sessions | Record a selected client/environment | Choose client and capture settings, start/stop, take a supported state snapshot, inspect session status and output |
| Live client data | Inspect the running game through Py4GW Stealth | Select a client, browse supported data readers, refresh a view, inspect field results, and save a labeled snapshot |
| Tests | Perform a named test with evidence | Read steps, start the selected capture tasks, mark progress/observations, finish with an explicit result |
| Content and configuration | Maintain the installed world and operator settings | Map/default spawn/portal-arrival editor first; service settings kept distinct from game content |
| Recordings | Find and inspect previous evidence | Filter by subject, target, date, client, build, and capture layer; open the associated run and artifacts |

These are proposed workspaces within one panel, not separate applications or requirements
for separate backend services. A settings dialog or tabs can replace a navigation item if the owner
chooses that arrangement. New NPC/quest editors arrive with their simulator phases, not as
empty controls that pretend those systems already work.

## The central workflow: a test run

A test run is the record joining an intention, a target, the work performed, and its evidence.
A standalone capture or data-inspection session is allowed; the operator does not need to
invent a test or run packet sniffing just to read the client.

Example: investigating alternating click movement and manual movement across a bridge.

1. Open the movement test or create a named run with the relevant instructions.
2. Select the target environment and the actual client process.
3. Choose client packet capture, server wire capture where available, and state snapshots as
   separate evidence channels. The panel shows prerequisites and which channels are ready.
4. Start the requested recording work and perform the gameplay steps in the real client.
5. Mark a checkpoint or write an observation without stopping the recording.
6. Stop cleanly and preserve the files, exit/cleanup status, and notes.
7. Record whether the gameplay issue reproduced. Link another run against live or ASURA to
   perform the same steps for comparison.

The panel must not infer that all proposed channels are required for every test. A simple
targeted recording must remain simple. A live recording does not require starting ASURA.

### Run information

Store enough information to resume review without the original terminal:

- Stable run id, subject, created/start/end times and original clock/timezone information.
- Declared environment, such as live or ASURA; source of that identification and any uncertainty.
- Selected client PID and executable identity available from the tool; do not trust a filename
  alone to establish which server it is connected to.
- ASURA build/world/config identity where known, and the test instructions used.
- Requested capture channels, directions, filters, duration, and actual tool/interpreter identity.
- Artifact paths, capture layer, process outcome, missing/partial output, and cleanup result.
- Operator checkpoints, observations, gameplay verdict, and links to related runs.

Persist the run record so closing or reopening the panel does not erase the history. The storage
format for this record is not chosen yet. Raw recordings stay immutable; notes and indices are
separate from the captured bytes. Reuse recordings/index.jsonl for the public archive instead
of introducing a competing public recording catalogue.

## Capture sessions

### Starting a session

The operator selects a client explicitly, especially when several Gw processes exist. Display
the selected identity and declared target beside Start and throughout the run. Pin packet and
context tasks to that same PID; never silently fall back to another client when it exits.

Check the prerequisites of the chosen channel, rather than requiring every research tool just
to open the panel. Client sniffing requires the external rig, an x86 Python interpreter,
appropriate access to the client, and a compatible client process. Normal server operation
must still work without them.

Create a distinct output location for each run and preserve it on failure. The current capture
tools accept explicit output paths. Do not overwrite a previous capture because a subject name
or the panel's session directory is reused.

Packet capture and periodic state dumps are separate tasks under the session. Preserve channel
failures independently: a working capture plus a failed dump is not an entirely successful run,
but the valid capture must not be discarded.

### Progress and stopping

Proposed session states are Preparing, Needs setup, Starting, Recording, Stopping, Completed,
Failed, and Interrupted. These describe actual tool state, not just which button was pressed.
Show elapsed time, latest activity where available, output location, and actionable errors.

Stop requests must let the capture tool leave its loop, flush, and disconnect. A process exiting
does not prove that hook cleanup succeeded. Record that result separately. If the client exits,
the helper is lost, or forced termination is necessary, preserve partial output and mark the
session interrupted/cleanup uncertain. Reattaching after uncertain cleanup needs a verified
recovery path; do not automatically start another hook on the same client.

ADR 0013 records that conflicting owners of the same hook corrupt the client. Coordinate
research tasks for a client and verify which packet/context operations can coexist before
launching them together. Independent recordings of different clients are a separate capability;
the initial number of concurrent sessions remains a question for the owner.

### Capture layers

Keep these distinctions visible in session details and recording views:

| Channel | Meaning | Valid use |
|---|---|---|
| Client packet capture | Client message structures, normally four-byte headers | Headers, order, triggers, counts, and observed client fields |
| ASURA server capture | Decrypted wire messages, two-byte headers | Wire sizes, layouts, bytes, and outgoing/incoming server behavior |
| State snapshot | Client state at a recorded moment | Relate a packet sequence to what the client actually held |

Use tools/capture-read.py's layer rules. Do not compare client-structure sizes directly with
wire sizes or label the difference a protocol regression. Preserve original timestamps and
client ticks; different streams do not automatically share a clock or comparable timestamp.
Operator markers help correlate actions, but do not manufacture a proven latency measurement.

Follow the existing tool's sanitized/raw distinction and recordings/README.md. A raw session
does not become a public archive entry automatically. Any sanitized derivative is a new file,
not an edit to the raw evidence. The default and available controls need to reflect the source
tool rather than inheriting live-session.ps1's unconditional raw mode by accident.

Server wire capture currently uses startup configuration. If enabling it requires a restart,
show that requirement and let the operator choose it explicitly; a Capture button must not
silently restart a running world or claim a channel is active before output exists.

## Live client data through Py4GW Stealth

The panel needs a proper client-data workspace, not only a button that writes an opaque dump.
This is the other half of the research workflow: packet capture shows what was sent/received;
the data readers show the client's state before and after an action.

### Reader groups and support

Expose supported measurements in understandable groups. The following groups describe the
intended organization, not a claim that every field or interactive browser is already wrapped
by ASURA's tools:

| Group | Data to inspect where the installed readers support it |
|---|---|
| Map and instance | Current map/instance information, position/plane, entry/spawn and portal measurements, pathing |
| Agents | Agent lists, identity/type, position, and available living-agent state |
| Character and party | Character context, party membership, selected targets, and available progression state |
| Items and inventory | Item context, bags/equipment, and available item properties |
| Skills and effects | Skillbar state, available skill properties, casting and active effects |
| Quests and interactions | Available quest, dialogue/service, trade, and interaction context |
| Additional contexts | Other supported client contexts and archive measurements used in the research |

Discover and verify available read operations from the installed rig and the existing measurement
tools. Separate Available, Unsupported, Unavailable in this client state, and Failed. Do not
show zero or an empty list as though a reader succeeded when it did not. The first implemented
groups and their fields need a concrete acceptance case before coding; do not silently promise
the entire external API in the first panel increment.

The panel presents data and measurements. Copy no external source, offset, byte pattern, or
catalogue table into the server. Record the tool, client/build, selected reader, time, and
output beside conclusions, following ADR 0013.

### Inspection workflow

1. Select the actual client and declared environment; keep that identity visible while reading.
2. Open a supported data group and request a refresh. Show when the result was read and any
   reader errors; an old result must not masquerade as current live state.
3. Inspect the returned records and fields. Display named units and identifiers where verified;
   label unknown meanings rather than inventing names from their values.
4. Save a labeled snapshot, independently or attached to the active test run.
5. Perform the gameplay action, read again, and retain the second snapshot with the observation.

Manual refresh and recorded snapshots establish the initial workflow. Periodic sampling is
available where the tool already supports it or is added as assigned work, with its interval
shown explicitly. It does not mean polling every exposed reader on every UI frame.

Reading state does not require enabling the packet sniffer. tools/context-dump.py explicitly
uses the rig's read-only connection with game_thread=False; retain that capability instead of
requiring the hook-writing connection for every inspection. Capability-specific prerequisites
still apply, and client read results are not necessarily atomic snapshots of all contexts.

Client inspection and ASURA configuration are separate operations. A measured spawn, portal,
or agent property can provide evidence for a later content edit, but reading it must not
automatically change database content or running-world behavior.

The owner requested data reading, not arbitrary client writes or a general memory editor.
Expose the supported read workflows; additional client-control features require an instruction.

## Tests: the operator's testing interface

Distinguish three things in the results:

- Tool execution: started, stopped, failed, interrupted, or completed normally.
- Automated checks: which command/check actually passed, failed, or was skipped.
- Gameplay observation: passed, failed, inconclusive, not performed, with the operator's notes.

A sniffer exit code of zero is not a passed movement test. A passing Rust test is not a
client-confirmed feature. A process failure is not proof of a gameplay regression.

Existing live-session presets provide instructions to reuse. The first tests should use real
existing procedures, such as travel and the recorded movement reproduction, rather than a new
invented full-game test catalogue. Version or snapshot instructions in each run so a later
edit to a test description does not change what a historical run says was performed.

User actions in the client remain manual unless the owner explicitly asks for automation.
The panel guides, records checkpoints, and launches the selected measurements. It does not
press game buttons or issue gameplay commands merely because a test is underway.

Targeted automated checks can be exposed as explicit jobs using the current testing policy.
Do not run the whole battery automatically on panel start, after every content save, or before
every capture. Preserve the reason required for a whole-suite/milestone run.

## Server operations and configuration

Reuse asura-supervisor for process ownership, readiness, retries, stop/restart, and registry
updates. The panel must not introduce a second watcher that races the supervisor.
Show stale/unknown status honestly rather than displaying an old ready value as current.

Initial operations are start, status, stop, selected-service restart, and viewing its logs.
Build/schema/seed/rolling-deploy jobs already exist as commands, but whether to expose all of
them in the first panel is an owner scope decision. The supervisor checks PostgreSQL; starting
the database remains outside its current responsibilities.

Service configuration and world content are different views. A saved configuration is not
necessarily applied to a running process. Display pending restart/application status instead
of claiming a live change the runtime does not implement.

Content editing continues to use the initial map 148/146 flow in docs/32-world-simulation-plan.md.
The selected content model is one self-contained profile, without inheritance or mod stacking
(docs/34-self-contained-profiles.md). Identify the profile being edited and the profile currently
loaded by the server; a content save does not switch the running profile. Each profile has separate
account settings, characters, levels, inventory, and progress. Profile packaging and the physical
storage layout remain unresolved. The future launcher supplies startup selection.
Share validation with the world/database operations. Do not add a generic SQL console, user
administration, a visual map canvas, or simulator controls for nonexistent features just because
the product is now called a control panel.

## Execution components and boundaries

| Component | Role | Existing boundary to preserve |
|---|---|---|
| Panel interface | The user's workflows, status, steps, results, forms | Final desktop/browser delivery is not selected |
| Operations/content adapter | Invoke supervisor operations and the shared content operations | No duplicate service supervisor or independent SQL implementation |
| Windows Py4GW research runner | Manage packet sessions and supported in-game data reads; return structured results and artifacts | Core panel research capability, optional for server operation; research remains in tools and no server crate imports or requires Py4GW Stealth |
| Run history and artifact index | Link commands, captures, checks, observations, and files | Recorded evidence is preserved; public archive uses its existing index |

The research runner executes on the machine containing the game client, which is Windows.
It is not an auth/world runtime component. Server-only installations remain independent of
the external client toolchain. The owner explicitly requested user-initiated capture launching;
it must remain independent of normal server startup and CI.

The exact UI/backend packaging and communication are not selected. Choosing a remote server
host later does not move the client sniffer to that host: it still runs beside the client.
Do not add remote execution, endpoints, or a required new service before deployment scope is
settled. The interface itself need not inherit the research runner's elevated privileges.

## Current tools: reuse and gaps

These are source-inspection findings, not tests of the future panel. Recheck before coding.

| Source | Existing capability or gap |
|---|---|
| crates/asura-supervisor/src/control.rs:70 | Status, stop, restart, and rolling-deploy control already exist on loopback |
| crates/asura-supervisor/src/main.rs | CLI start/build/schema/seed commands already exist |
| tools/sniff-capture.py:673 | Direction, duration, headers, output, PID, polling, raw mode, and external root are parameters |
| tools/sniff-capture.py:558 | The capture handles KeyboardInterrupt and has cleanup code; the runner must preserve that path |
| tools/context-dump.py:483 | External root, PID, label, selected contexts, and output are parameters |
| tools/context-dump.py:24 and :74 | Existing read-only data collection and named context readers provide the starting point for live data inspection |
| tools/live-session.ps1:37 | Guided presets, capture plus periodic dumps, and a manifest provide an existing workflow |
| tools/live-session.ps1:105 | A reused output path deletes the prior capture; not suitable for panel session history unchanged |
| tools/live-session.ps1:124 and :140 | It selects the first client and does not pass an explicit PID to the sniffer |
| tools/live-session.ps1:168 | It force-stops the sniffer; not a verified clean shutdown contract for the panel |
| tools/capture-read.py | Existing struct/wire identification and header-offset handling |
| recordings/README.md | Public archive naming, immutable evidence, sanitation, and index format |
| docs/adr/0013-py4gw-stealth-is-our-link-to-the-client.md | External research boundaries, operating requirements, and hook ownership |
| tools/test-touched.ps1 and tools/test-battery.ps1 | Existing targeted-check and whole-battery entry points |

Do not simply put a button around live-session.ps1 and call the session manager done. Keep its
useful workflow, but implement the selected PID, output ownership, stop, and status contracts
before relying on it. Any script fixes are separate scoped implementation work, not authorized
as part of writing this design.

## Proposed implementation increments

1. Agree on local/remote deployment, UI delivery, initial capture concurrency, and the exact
   first end-to-end acceptance flow. Do not choose a framework before these answers.
2. Deliver the panel shell with real supervisor status/control/logs. Show research setup
   availability without requiring the external rig to open the panel.
3. Deliver the Py4GW research workflow: one explicit-client capture session with distinct output,
   live status, clean stop, preserved evidence, persisted history, and correct failure reporting;
   plus the first selected live-data reader views with refresh and labeled snapshots. Data
   reading must work independently of packet capture.
4. Deliver one guided manual test linking its capture, checkpoints, notes, and operator result.
5. Add the map/default-spawn/portal-arrival editor once its content loading/save path is ready.
   Do not claim content management is complete when only process buttons work.
6. Extend to additional evidence channels, paired runs, and later simulator content as those
   capabilities are selected and implemented. Comparisons must respect capture layers.

This is the proposed panel work order. It does not reorder the simulator's combat/AI/quest
dependencies or make every workspace an initial release requirement.

## Questions before implementation

- Is the first setup entirely on the owner's Windows machine, or must it manage a remote
  server while captures run on the Windows client machine from the first release?
- Is the requested interface a desktop application or a browser application?
- Does the first release need one active client capture at a time or several simultaneously?
- Which live-data groups/measurements are the first acceptance case, alongside server start/stop
  and packet-session management? The capability itself is required; its initial field coverage
  is not yet selected.
- Must the map editor be delivered in that same first release, or in the next content increment?
- Before accepting session lifecycle behavior, what should closing the panel do to an active
  capture or long-running job? It must not silently force-kill an attached sniffer.

These are implementation gates, not reasons to repeat settled decisions about Lua, Rust, or
the packet-stream objective. The owner selects the unresolved product choices.
