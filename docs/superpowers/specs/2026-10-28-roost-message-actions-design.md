# Message actions, versions, branches and side questions

Status: draft for review. Mockups: https://claude.ai/artifact/TVBX5TWnvBsHtJ2LyLFpq6

## Intent

Chat bubbles get a small row of icon actions, like other AI chats, and the
chat gains the things those actions need: versions of a message (regenerate
and edit), branching into a new chat, side questions that stay out of the
conversation (BTW), per-reply details, and reading a reply aloud. Later the
same data should support a map of how a conversation got to "now"; this spec
does not build that map but must not make it hard.

Decided with the user:

- Read means read aloud. It is disabled (or hidden) on platforms without a
  speech engine.
- Edit and Regenerate are versions inside the same chat. Branch means a new
  chat in the sidebar.
- BTW answers are kept in a small history.
- Details (ⓘ) is included.
- Layout: the row sits at the bottom-left of the bubble, outside it. It is
  always visible on the most recent message and appears on hover for older
  ones. The version counter (`‹ 2 / 3 ›`) sits at the bottom-right and is
  always visible on any message that has more than one version.

## The row

| Bubble | Icons (left to right) |
|---|---|
| Assistant | Copy, Regenerate (latest only), Branch, BTW, Read, Details |
| You | Copy, Edit |

- Icons are painted as line shapes (like the settings gear), 16 px in a 28 px
  hit area, with a tooltip each. No icon fonts.
- The row has no Regenerate, Edit or Branch while a run is active, and in the
  Work tab only Copy, BTW and Details are offered on agent messages.
- Icons that cannot work are disabled with a reason in the tooltip (Read on
  a platform without speech; Regenerate with nothing before it).

## Versions (Regenerate and Edit)

The conversation stays a linear list, `Session.messages`, which is the active
path. Everything that reads it today keeps working. A new field holds the
paths not taken:

```
forks: Vec<Fork>
Fork { after: Option<MessageId>, tails: Vec<Vec<Message>>, active: usize }
```

- A fork is a place where the chat could continue in more than one way:
  `after` is the message before it (`None` at the start), and each tail is a
  list of messages from there to the end of that version.
- Regenerate on the last reply, or Edit on any of your messages, archives the
  current tail as a version of that fork and starts a new tail. Switching
  versions swaps the active tail with an archived one. The model sees only the
  active path.
- Sessions without `forks` load as before (the field defaults to empty), so
  no migration is needed and old Roost builds ignore the field.
- Cost accepted: only one version of a fork is shown at a time, and a later
  map is derived from `forks` rather than stored as a tree.

Edit sends the edited text as a new version and runs the model from there.
Regenerate is offered on the latest reply only.

## Branch

Decision: versions (Regenerate and Edit as forks, below) were tabled, then
built before Branch (stage 2) because Regenerate otherwise discards a reply.

A branched chat shows a strip at the top: `↳ Branched from "A" › "B"`, each
name a link that opens that chat at the message where the branch began. Long
chains collapse to `… ›` and the last two. The branch stores the whole chain
of ancestors (chat id, message id, title at the time), so a branch of a branch
needs no lookups and survives an ancestor being deleted. It also stores a
fingerprint of the original's messages up to the branch point. On opening, the
strip reports one of: unchanged; "The original changed after this branch was
made" (with View original); or "The original chat was deleted". The copied
history in the branch is the history as it was, and a divider marks where the
inherited part ends.

Branch copies the active path up to and including the chosen message into a
new chat (new id, `Persisted`, title from the original plus "(branch)"). The
original is untouched. "Branch and switch model…" does the same with another
installed model chosen first. In the Work tab it keeps the folder and tool
settings.

## BTW (side question)

A one-shot request with the conversation up to the chosen message as context.
The question and its answer are never written to the transcript. They go to a
small history (see below) and show in a popover under the message with "Add
to chat" (appends the pair to the transcript) and "Dismiss".

- The request is separate from any run, so it can be asked while an agent
  works and does not steer it.
- History: per session, newest first, capped at 50 items. Ruling: stored in
  the chat's own file (field `btw`) rather than a second file, so it is
  removed with the chat and incognito chats keep it in memory with no extra
  code. A branch starts with an empty history. For agent chats the question
  sees the conversation but not tool results.

## Details

Each finished reply stores its numbers from Ollama's final stream message:
prompt tokens, reply tokens, total and load time, from which the popover shows
model, tokens in and out, speed (tokens per second) and duration. Replies
saved before this change show the model and "no timing recorded".

## Read aloud

Ruling: no speech crate. Roost runs the operating system's own tool and
writes the text to its standard input (never on a command line): `say` on
macOS, the .NET speech synthesizer through PowerShell on Windows, and
`spd-say`, `espeak-ng` or `espeak` on Linux when one is on the PATH. This adds
no dependency (nothing for `cargo deny` to vet, no OpenSSL) and makes
availability a runtime check: with no tool the icon is greyed and its tooltip
says what to install. Reads the reply as plain text (Markdown marks removed,
code blocks skipped); the icon turns into Stop while speaking. The Windows and
Linux commands are checked by unit tests of the command choice only, not
run in CI. The engine sits behind a `Speaker` trait so tests use a fake.

## Stages

Each stage is its own pull request, test first.

1. **Row.** The icon painter, row layout and visibility rules, Copy, Details
   (stores the reply numbers), and the existing Regenerate moved into the row.
2. **Versions.** `forks`, Regenerate and Edit as versions, the counter.
3. **Branch.** New chat from a message, with model switch.
4. **BTW.** One-shot request, popover, history.
5. **Read aloud.** Speech wrapper behind a trait, platform gating.

## Testing

- Core: version swap round trips and never changes the active path's
  validity; old files load; branch copies the right prefix; BTW writes
  nothing to the transcript; stats parse from the final message.
- GUI: for each size and theme, the row stays inside the pane, shows on the
  latest message and on hover for older ones, the counter is bottom-right,
  and disabled icons say why.
- Speech is behind a trait so tests use a fake.

## Not in this spec

The conversation map, merging branches, thumbs up/down, sharing.
