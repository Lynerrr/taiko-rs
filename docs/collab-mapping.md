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

## Connection (`taiko_proto::mapping::Link`)

`Request::OpenMapping` (the host, for a set), `JoinMapping` (an invited player) or
`ResumeMapping` (back after a lost connection, as the same join) is answered with
`Reply::Mapping` (session, our epoch and id, the assets the server still needs, and on a resume
the last of our ops it took), and the connection then carries `MapUp` / `MapDown` lines, read and
written on threads of their own like the social connection, with a ping every 25 s. A line the
game doesn't know is passed over. The link reports `Joined`, every `Down` and `Lost`; it never
reconnects by itself, since the resume needs the session's last seq. Snapshots go in 512 KiB
parts (`snapshot_lines`, `SnapshotPieces`), keeping every line within protocol 1's 1 MiB.

## Invites, joining and hosting (`src/mapping_sessions.rs`, `collab::connect`)

- The game keeps whether the server listed `mapping/1` (`State::mapping_sessions`, cleared on
  logging out). Only then does it send `Up::Features(["mapping/1"])` on connecting the social
  side, show *Start mapping session* in the editor (over the preview's corner), and act on an
  invite.
- **Invites:** `Down::MappingInvite` is a notification ("X invites you to map"); clicking it
  joins. `Down::MappingClosed` takes it away. The host invites friends from the friends panel:
  while hosting a session in the editor, every friend online has an *invite* button
  (`HostCmd::Invite`).
- **Joining** (`connect::Connecting::guest`): `JoinMapping`, then the session's state, which
  lists the set's folder and files by sha256. Files the Songs folder doesn't have yet (in a
  folder of the set's name; a set already there is matched by hash) are fetched with
  `GetAsset` one after another and checked against their hash; a name pointing outside the
  set's folder ends the join. Then the snapshot and the ops after it. The editor opens on the
  difficulty given to us (a guest difficulty we own, or one with a part of ours), else the
  first; its chart file is written if the folder doesn't have it. Chat, presences and comments
  that came meanwhile go to the session when it starts.
- **Hosting** (`connect::Connecting::host`): *Start mapping session* reads the set's folder
  (the `.osu` charts and every other file, within 40 MB a file and 200 MB in all), sends
  `OpenMapping` with its description, uploads the files the server says it needs in 512 KiB
  pieces, and answers the new session's `SnapshotWanted` with the snapshot of its charts. A
  session the server already has (opened again) comes back like a join.
- **In the session** (`View::pump_session`): every frame, the server's lines go to the session,
  our new edits and our place go out. A lost connection is resumed (`ResumeMapping`) 2 s later,
  and at once when out of step; edits made meanwhile stay pending and go out after it. The
  server's `SnapshotWanted` at our seq is answered with our confirmed state. When the session
  ends the chart stays in the editor to edit alone.

## Rights, credits and a server restart

- **Rights** are checked by `mapping::refusal`, the same function the server runs: the host
  edits everything; a collab mapper edits objects and green lines inside their own parts and
  the gimmick of a difficulty they have a part in (shared with the other mappers of it), not
  its keys; a guest edits their own unlocked difficulty (not red lines or the set's metadata);
  modders and viewers edit nothing.
- **Credits** go in each difficulty's file as `TaikoRsMappers: anna, host` in `[Metadata]`
  (`mapping::CREDITS_KEY`): who placed its objects, most first, objects the session has no
  maker for counting as the host's. osu! passes over the key; `Creator` stays the host. Saving
  in a session writes it into the open difficulty and writes every other difficulty of the
  session to its file in the set's folder (found by its name, or a new file), so the host can
  pack and upload the whole set with its credits.
- **Server restart:** sessions live in the server's memory only. A resume the server refuses
  (`LinkEvent::Refused`, "no such session") ends the session like the host closing it: the
  chart, our unsent edits included, stays in the editor to save or to start a new session with.

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

## On the editor's screen (`src/editor/view/collab.rs`, `collab::session`)

`View::collab` holds the session (`None` while editing alone, and then nothing below is drawn
or run). `Session` reads the server's lines: the session state, ops (into `Sync`), refusals,
the others' places, chat and the end of the session.

- **Participants panel** over the preview's top right corner: each player's colour, name and
  role, the difficulty they are on when it isn't ours, *away* after a minute without moving,
  *offline*. A click on someone follows them (the playhead and the view keep to theirs while
  they are on our difficulty); a second click stops. The last row turns colouring by author on.
- **Timeline**: the collab parts of the open difficulty as bars along the ruler in their
  mapper's colour (grey for the host's), the others' playheads with their names, their pointers
  (faint lines) and the notes they have selected (rings in their colour); with colouring by
  author, a bar under each note in its maker's colour.
- Our own place goes out at most every 100 ms and only when it moved (`Session::presence`).

## Tests

`src/editor/collab/tests.rs` plays sessions in memory against a fake server that orders,
refuses and deduplicates ops like the real one, with games editing through the real editor:
random interleavings of edits, undo/redo, sends, deliveries, dropped connections and log
compaction (80 seeds) end with every game's charts equal to the server log's. Others cover
per-player undo, refused edits outside a part, edits made offline, red lines reaching every
difficulty, snapshots and the wire form of every command. The wire format of every mapping
message is kept golden in `src/regression/network.rs`; the editor's screen alone and in a
session in `screen-map-editor` and `screen-map-editor-session`. `collab::session` and
`view::collab` have their own tests (presences, throttling, following, refusals, chat, the
panel's rows and clicks, nothing drawn alone, resuming, answering snapshots, the end of a
session), and `collab::connect` hosting and joining against files on disk.
`regression::network::an_invited_player_joins_a_mapping_session_and_edits_in_it` clicks an
invite and joins a scripted server: the set fetched, the editor on the guest difficulty, an
edit sent and the session's end.
