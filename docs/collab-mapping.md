# Collaborative mapping (client side)

Live co-editing of a map set in the editor: collabs (several mappers on one difficulty, by
time-range parts) and guest difficulties (a guest owns a whole difficulty; audio, red lines and
the set's metadata stay the host's). The protocol and the server's side are specified in the web
backend's `apps/backend/docs/collab-mapping.md`; the wire types are
`crates/taiko-proto/src/mapping.rs`. This page is how the game implements its side.

## Legacy servers

A server that runs sessions lists `mapping/1` in `Reply::Account::features`. Servers from before
it never send the field, and the game sends nothing about sessions to a server that didn't list
it: no session request, no mapping stream, no social message. The single-player editor never
builds a session at all.

## Sync (`src/editor/collab.rs`)

Every edit is a `history::Command` already. In a session the history keeps an outbox of the
commands it applies (`History::share`), and `collab::Sync` turns them into ops:

- **confirmed** charts, one per difficulty: the snapshot plus every op the server ordered;
- **pending** ops: ours, not yet back from the server, in the order made;
- the editor's chart: the confirmed chart of the open difficulty with the pending ops on top.

Ops only set things: an insert of an id already there, and anything about an id that is gone, is
passed over (`apply_tolerant`). The same ops in the same order therefore give every game the same
charts. A remote op goes into the confirmed chart, and the editor's chart is made again from it
with our pending ops on top (or the op is applied directly when nothing of ours is pending). Our
own op coming back is taken off the pending list. A refused op is taken off and the editor's
chart is made again without it.

- **Ids:** a join with epoch `e` gives out ids `(e << 40) | n`, so no two games make the same id.
  After a resync the game keeps to its own range, never continuing from the largest id in the
  snapshot.
- **Red lines:** a red line that is the same in every difficulty has one id in all of them. A
  change of a red line in a set with several difficulties also goes out as an op for the shared
  timing (`diff: None`), which every game applies to every difficulty.
- **Drags:** an unsent drag step folds into the one before it, and while dragging the newest op
  waits until 50 ms after the last send.
- **Undo and redo** take back only this player's own steps, and only where the chart still is as
  the step left it (`apply_guarded`): what someone else changed since stays theirs. A step with
  nothing left to take back is passed over.
- **Reconnect:** the server's snapshot, the ops after it and the last of our ops it took
  (`done`). Our pending ops up to `done` are dropped (their effect is in the snapshot or tail, or
  they were refused); the rest stay on top and are sent again.
- **Snapshots** for the server's log are the confirmed charts' texts with their ids in the order
  the parser reads them back (`Sync::snapshot`), and who made each object.

## Tests

`src/editor/collab/tests.rs` plays sessions in memory against a fake server that orders,
refuses and deduplicates ops like the real one, with games editing through the real editor:
random interleavings of edits, undo/redo, sends, deliveries, dropped connections and log
compaction (80 seeds) end with every game's charts equal to the server log's. Others cover
per-player undo, refused edits outside a part, edits made offline, red lines reaching every
difficulty, snapshots and the wire form of every command. The wire format of every mapping
message is kept golden in `src/regression/network.rs`.
