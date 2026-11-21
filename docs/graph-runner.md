# The state-graph runner

Skate 3 decides what the rider is doing (riding, pushing, winding up, which trick, manual, landing,
grinding ...) and what animation plays with two state graphs from its disc: `ActionGraph_OnBoard`
(turns controller intents into motion intents) and `MotionGraph_OnBoard` (states, animation trees,
values for the physics). flatground runs the player's own copies of them.

Nothing of the graphs is in this repository. `fg_import motion` compiles both from the player's disc into
the `motion/graph` entry of `motion.fgpak`; without it the game uses its own hand-written riding logic.

How the graphs are read and run follows SK8-ENGINE/skate-3-rust-engine (GPL-3.0), a Rust port of Skate 3
that runs the disc's graphs: crates/skate-data state_graph (binding), crates/skate-core graph (selection,
activation, controller) and crates/skate-game graph_host (what each behaviour and condition does).

## Code

| File | What |
|---|---|
| `game_core/src/graph/mod.rs` | the compiled format and its reader |
| `game_core/src/graph/write.rs` | the compiler (importer and tests): ownership, `enabled` inheritance, transition targets and interrupt ancestors by name, state-bound conditions |
| `game_core/src/graph/select.rs` | next-state selection and the controller (behaviour begin/update/end, hooks, state clocks) |
| `game_core/src/graph/ops.rs` | every behaviour, condition and hook parsed into a typed operation |
| `game_core/src/graph/host.rs` | the runner: both graphs, intents, what each operation does |
| `game_core/src/graph/playback.rs` | what the motion graph plays: trees joined by crossfades and sequences |
| `game_core/src/graph/input.rs` | the pad as action-graph intents, gesture recognizers |
| `game_core/src/graph/drive.rs` | the game's side: building the runner, the physics state it reads, timing |

## Selection

Each tick, for each graph:

1. From the current leaf state up to the root, look for a transition whose condition holds, `urgent`
   first, then `high`, `med`, default. A transition also needs its target (and the target's new
   ancestors) to hold their preconditions; the target is then entered at its first active leaf.
2. Without one: a state with `interruptable="false"` stays while it and its parents hold their sustain
   conditions; an interruptable one is re-resolved from the root (children in authored order, the first
   whose conditions hold); `interruptable="<Ancestor>"` re-resolves below that ancestor first.
3. Conditions carry a mask: `precond` (checked when entering), `sustain` (checked while staying on the
   current path), `always` (both). States with `active="false"` are only reached by transitions.
4. Behaviours of the states left end (newest first), the transition's hooks run, behaviours of the states
   entered begin, then every active behaviour updates.

## Numbers (stage A, Dolphin, US/EU disc)

| | |
|---|---|
| `motion/graph` in the pack | 612 KiB (action graph 234 states, 128 transitions, 876 operations; motion graph 2,088 states, 2,390 transitions, 3,247 expressions, 8,305 operations; 2,476 strings) |
| parsed in memory | 554 KB; MEM2 about 2.2 MB lower than before the graphs (pack entry, parsed tables, typed operations) |
| per tick | 0.07-0.08 ms (pad intents, both graphs, animation advance), part of `physics` in the frame line (`graph`) |
| operations | 8,169 run, 1,006 deliberate no-ops (systems flatground doesn't have), 4 unsupported (skitching, `GestureType`) |
