# ADR 0014 - BehaviorTree.CPP is the behavior-tree execution reference

- **Status:** Accepted; implementation has not started.
- **Date:** 2026-11-01
- **Authority:** the owner's instruction: "lets mark beahviouttree cpp as our standars", with the
  differences recorded because the later HeroAI port will be delicate.

## Decision

Use BehaviorTree.CPP as the reference for the execution rules of ASURA's behavior-tree nodes.
Implement the execution runtime in Rust. Use Py4GW Reforged's existing HeroAI as the source of
game decisions to express in external Lua scripts, with an explicit mapping from its node rules
to the target execution rules.

This is a semantic reference, not a C++ runtime dependency or a choice of tree file format.
Rust owns the execution machinery and the tick. Lua is the configuration layer: game-specific
conditions, action decisions, priorities, skill knowledge, and tree structure are declared in
external configuration, and Lua constructs each tree from Rust-provided node kinds and wrapped
leaves. Lua never ticks, holds running node state, or executes per-tick logic. The generic Rust
runtime must not embed skill-specific or profile-specific rules. This boundary follows the
owner's 2026-11-02 clarification in docs/30-world-content-and-scripting.md; it does not select a
configuration format or a Lua binding API.
Custom profiles must be able to adapt HeroAI for custom skills without changing Rust when the
required mechanics exist.

Matching node names do not establish matching behavior. Select the reference variant by its
execution rules, including running, completion, interruption, and reset. Do not automatically
replace every sequence with a reactive sequence or every selector with a reactive fallback.

## Source baseline

The comparison below is a source inspection, not an executed test or live-game measurement.
These sources are the only witnesses for their respective node rules.

- Py4GW Reforged commit `a5fd2e54d504605200a752b1bc991d618ed33c41`:
  [BehaviorTree.py](https://github.com/apoguita/Py4GW_Reforged/blob/a5fd2e54d504605200a752b1bc991d618ed33c41/Py4GWCoreLib/py4gwcorelib_src/BehaviorTree.py)
  and [headless_tree.py](https://github.com/apoguita/Py4GW_Reforged/blob/a5fd2e54d504605200a752b1bc991d618ed33c41/Py4GWCoreLib/HeroAI/headless_tree.py).
- BehaviorTree.CPP commit `a9d6e5637f4a4fcd29fcd16b8db049de7c42006b`:
  [source tree](https://github.com/BehaviorTree/BehaviorTree.CPP/tree/a9d6e5637f4a4fcd29fcd16b8db049de7c42006b).
- The official documentation read for this decision identifies itself as version 4.8:
  [sequences](https://behaviortree.dev/docs/nodes-library/SequenceNode/),
  [fallbacks](https://behaviortree.dev/docs/nodes-library/FallbackNode/),
  [parallel nodes](https://behaviortree.dev/docs/nodes-library/ParallelNode/), and
  [asynchronous actions](https://behaviortree.dev/docs/guides/asynchronous_nodes/).

The pinned C++ source is the comparison baseline; a moving documentation page does not silently
change the target. Recheck the baseline explicitly when implementation starts or the reference is
updated. This decision does not require implementing the entire upstream node library at once.

## Differences and migration handling

Python line references below are in `Py4GWCoreLib/py4gwcorelib_src/BehaviorTree.py` unless stated
otherwise. C++ file references are relative to the pinned BehaviorTree.CPP repository root.

| Feature | Py4GW behavior and evidence | Reference behavior and evidence | Handling during the Rust port |
|---|---|---|---|
| Node status | Three result states, with `None` used before execution or after reset (`:85`, `:127`, `:234`) | Explicit idle lifecycle state and skipped result in addition to running, success, and failure (`include/behaviortree_cpp/basic_types.h:33`) | Map lifecycle and results explicitly, not by enum numbers. Define skipped-node handling for each implemented control node |
| Sequence | Saves the running child; restarts after failure (`:608`) | `Sequence` saves the running child and restarts after failure (`src/controls/sequence_node.cpp:39`) | Closest mapping is `Sequence`, not `SequenceWithMemory`, which also retains its place after failure. Reactive checks are an explicit branch decision |
| Selector | Saves the running child and skips earlier failed alternatives on the next tick (`:687`) | `Fallback` has the same running-child rule (`src/controls/fallback_node.cpp:31`) | Map to `Fallback`; choosing `ReactiveFallback` changes priority reevaluation |
| Choice | Starts at the first child on each tick; an earlier running branch does not explicitly halt a previously running later branch (`:764`) | `ReactiveFallback` starts at the first child and halts other children when a child runs (`src/controls/reactive_fallback.cpp:25`) | Closest mapping is reactive fallback, with an explicit interruption difference to handle |
| Action completion | A callback's final result is withheld until a later tick, even at zero aftercast (`:448`) | Simple actions return the callback result immediately; stateful actions use start/running/halted lifecycle methods (`src/action_node.cpp:40`, `:171`, `:197`) | Record where HeroAI relies on that extra tick. Represent intended aftercast or yielding explicitly rather than adding an implicit delay to every Rust action |
| Parallel completion | Ticks successful children again while other children run; succeeds when all succeed in the same call; first failure ends the node (`:1232`) | `Parallel` remembers completed children and supports success/failure thresholds (`src/controls/parallel_node.cpp:72`) | Choose thresholds explicitly. A continuously running service must remain running; a completed task must not rely on being called again |
| Halt and reset | Reset clears local flags and recursively resets children; no separate action halt callback in the reviewed class (`:234`, `:488`, `:598`, `:755`) | Control reset halts running children; stateful actions receive `onHalted` (`src/control_node.cpp:38`; `src/action_node.cpp:197`) | Define how each running movement, cast request, or other action relinquishes or cancels its work. Resetting tree memory alone is insufficient |
| Blackboard | Wrapper seeds the current client player's professions and assigns one shared dictionary recursively every tick (`:2024`, `:2045`, `:2054`) | Node configuration carries a blackboard pointer (`include/behaviortree_cpp/tree_node.h:104`) | Supply the server agent's context without reproducing the per-tick assignment walk. Keep mutable context separate for each agent; Rust storage remains to be designed |
| Conditions that wait | `ConditionNode` accepts a running result (`:536`); HeroAI's casting condition uses it (`headless_tree.py:332`) | The documented condition role returns success or failure, rather than running (`include/behaviortree_cpp/condition_node.h:25`) | Review the casting gate as a waiting/stateful action, not a mechanical condition rename. Preserve which checks run while waiting |
| Repeat | Fixed repeater counts both success and failure toward completion and has no child reset between iterations (`:933`) | `Repeat` counts success, resets the child, and terminates on failure (`src/decorators/repeat_node.cpp:35`) | Map repetitions and retries separately; a direct rename changes failure handling and repeated child initialization |

Sequential child calls inside a parallel node are not a difference: both runtimes use them. A
parallel behavior does not imply one operating-system thread per child. Likewise, traversing
multiple immediately completed children in one call is normal sequence/fallback behavior, not a
reason to force one node per tick. The official reference also offers explicit yielding variants.

Other source-specific nodes need an explicit definition when used: waits and throttles
(`BehaviorTree.py:1460`, `:1558`, `:1929`), switch (`:867`), lazy subtree construction and destruction
on reset (`:1311`, `:1348`), and the remaining repeaters (`:1003`, `:1085`, `:1166`). Their names alone
are not evidence of a reference equivalent. The runtime clock and polling/wakeup policy remain
separate design questions.

## HeroAI migration checkpoint

The reviewed headless tree is composed at `headless_tree.py:278` and ticked at `:260`. Its outer
sequence contains alive/distance/knockdown guards, a casting wait, and a selector containing
looting, out-of-combat work, following, and combat. A running later child prevents earlier guards
from being rechecked by that sequence until it resets. Other initialization checks run outside the
tree, so moving only the tree does not reproduce the complete caller's behavior.

Before porting each used branch:

1. Name the Python nodes, caller checks, and requested world actions, with the source revision.
2. Name the exact reference variants and list differences in call order, timing, and cancellation.
3. Show the expected behavior for immediate success, running across ticks, failure, interruption,
   and reentry. Include successful parallel siblings and a change in priority while another
   branch runs wherever those nodes are used.
4. Express game-specific leaf decisions in Lua using authoritative Rust agent queries and
   actions; keep generic execution machinery in Rust. Client globals and queues do not define
   the server's API.
5. Verify the affected cases with focused deterministic checks, then use the real client for
   gameplay and packet-stream behavior. Record intentional adaptations rather than silently
   preserving quirks or silently changing HeroAI's decisions.

This checkpoint is a requirement for the later port, not a new harness to build during planning.
The reference determines node execution; it is not evidence of Guild Wars combat rules. The
packet-stream objective, per-change pre-flight, and targeted-test rules still apply.

## Consequences

The Python class supplies behavior compositions and evidence of existing HeroAI assumptions.
Its runtime is not the target specification. The Rust node engine follows the reference, and the
HeroAI migration handles the differences at the affected branches. No runtime, AI code, or
external dependency is added by this documentation decision.
