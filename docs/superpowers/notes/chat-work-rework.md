# Chat and Work rework: notes

Collected from using the app. Nothing here is built yet. The Models tab is the
reference for the look (stacked bands in the theme's colours, icon buttons,
pills for facts).

## From trying the agent UI (2026-10-29)

- **The agent settings are not discoverable.** The only hint is a chip that
  reads "Folder: Folder Name". It has no icon suggesting settings, and the
  "···" next to it looks like it only changes the folder. Everything the agent
  can be configured with (folder, step budget, tools) hides behind it.
- **The popover is too minimal.** It needs layered sections (cards with their
  own headings and a slightly different background). Showing the selected
  choice only by text colour reads as unfinished; use a filled pill or segmented
  control.
- **The path belongs in the title:** `Untitled (~/Documents/Code/Folder Name)`,
  with an ellipsis in the middle when it is too long and the full path in a
  hover tooltip.
- **The title is underpronounced:** bigger and bolder, with some marker.
- **Tool permissions are a large piece of work** (the Off / Ask / Auto control
  is a stopgap; the approval card is easy to miss).
- **Agent sessions without a folder** should be allowed, and a folder attachable
  later (see ROADMAP).

## Suggestions for the title row

A header band like the Models pages, tinted with the theme's accent, holding:

1. A small rounded tile with an icon: a speech bubble for a chat, a bubble with
   a spark (or a folder) for an agent. It says at a glance which kind of session
   this is and gives the title a colour anchor.
2. The title in the heading style (the one the Models pages use), semibold and
   larger than now. An untitled session shows its first message in grey italics
   instead of the word "Untitled".
3. For an agent, the path as a muted suffix in parentheses, cut in the middle
   when it does not fit, full path on hover.
4. On the right: a status dot with a word (idle, working, waiting for you) and
   the buttons. The dot carries information, so it earns its place more than a
   decorative one would.

Instead of an underline, the band itself (a line under it, as on the Models
pages) separates the header from the conversation and stays consistent.

## Suggestions for the agent settings

- A button labelled **Agent settings** with a gear icon, so nobody has to guess.
  Folder stops being its own chip; it is the first section inside.
- The popover as three cards: **Folder** (path, Change…, Remove once folders are
  optional), **Run** (step budget), **Tools** (the permissions, later the whole
  permissions design).
- Segmented controls for Off / Ask / Auto with a filled selected segment.
- The "···" menu keeps only session actions (rename, pin, delete).

## Header actions (decided 2026-10-29)

The "···" overflow menu was replaced by a pin, a red trash can and a rename
pencil. When there are meaningful extra actions (export or copy as Markdown,
duplicate, history manager actions), bring "···" back on the **left** end of
the button stack, so it does not interrupt the right-in scan of the pin and
the trash can. Not before there is a second such action.

## Execute (X) is opt-in (decided 2026-10-29)

Running commands is off until the user turns it on in Settings, with a warning
that says what a command can do (it runs with the user's own rights and is not
limited to the agent's folder). Until then the X chip and the shell tools are
hidden everywhere. Once enabled, X is still Off per session by default, asks
first, and shows the exact command on the approval card.

## Timestamps (2026-10-29)

- Messages should show when they were sent, quietly: on hover (the icon row under a bubble, and a tooltip with the full date and time). `Message.created_at_ms` already holds it. Details and where to put it are in `handoff-2026-10-29.md`, next step 3.

## Direction and review (2026-10-30, with the user, from screenshots)

**Direction agreed: B + C.** One list of sessions with "agent" as a mode (a chat gains a folder and tools when you want it; folder-less agents fall out of this), plus a running/waiting badge in the list and later a dashboard-style view for concurrent runs. The user's longer view: many chats at once, eventually agents that cooperate (to be planned separately). The user's own use: Work is "just chat with access to files and tools". Nothing here is urgent; it is all backlog.

**From the screenshots (backlog, roughly in the order proposed):**

1. **Models-tab footer on Chat and Work.** "Downloads & imports / Nothing running. Pick a model above to start." and "Staging folder ... (files deleted after each import)" take about 120 px on every tab, and the first line is wrong on Chat (a model is picked). Move them to Models or into the status area.
2. **Sidebar rows blend together** (six empty "Untitled" rows, all with the same model line). Ideas: use the second line for something that differs (folder for an agent, status running / waiting / failed, a first-message preview); time on the right; a hairline or card edge between rows; an italic grey "New chat" instead of "Untitled" (as the header notes already say). The model name is the same on every row and need not be repeated. Check whether "New chat" should reuse an existing empty chat instead of creating another one (not looked at yet).
3. **A failed run has no action.** The Work screenshot shows "Failed" and "Ollama stopped responding. Retry when it is ready." with no Retry. This is handoff 4b (banner C); the comps are agreed there.
4. **The model name repeats** on every assistant message, in the composer chip and in the sidebar. Show it once per run or when it changes.
5. **Work header is crowded**: title truncates next to the Idle dot, the R and W chips, Agent settings, pin, trash. The chips are cryptic. The path is a subtitle line; the notes suggested a muted suffix.
6. Smaller: large gaps between messages (~70 px); the "x" on the selected sidebar row next to the header trash can (unclear which deletes); tool-group lines read as plain text; "Pinned / Earlier" (Chat) and "Yesterday / Earlier" (Work) group differently; the timestamps are in a monospace face that matches nothing else.

**Keep:** the tinted header band with the icon tile, collapsed tool groups, the accent bar on the user's message, the icon row under replies, the steering placeholder.

## Composer redesign (comped 2026-10-30, not built)

The input at the bottom wastes space. Agreed design (two rounds of comps with the user):

- **Input:** a one-line bubble that grows with the text up to a cap, then scrolls. The send button is inside it, at the bottom right (an up arrow; a right arrow is a cosmetic swap). While an agent works, Stop replaces Send and Enter steers; a queued steering message shows above the input.
- **Floating pills under the input, left:** `+` (menu: working folder, attach file, attach image; dictation later), a tools icon (icon only, the name in the tooltip; opens the Tools popover), and the folder as a pill with the path cut in the middle when the chat has one.
- **Right:** the R, W and X permission pills, then the model selector (name truncated, full name on hover).
- **Folder and tools are available in every chat** ("abilities added to a chat"); in a plain chat the folder is only a `+` menu item, no pill. This is the B + C direction in the section above.
- **Pills as toggles:** R and W cycle Off / Ask / Allow by click. **X cycles only Off / Ask; Allow is a right-click**, and the tooltip says so. The Tools popover always shows all three states for every tool. X appears only when running commands is enabled in Settings.
- **Narrow window:** the folder pill shrinks to an icon, the model name shortens.
- **"Step n of N"** moves from the old status line to the header, beside the Idle / Working dot (to confirm).
- Replaces the current composer (the large padded box, the model chip and the Send button under it). Layout tests at both sizes must cover the pills, wrapping, and the Stop state.

**Order agreed:** composer PR, then the run-end banner (handoff 4b), then the hand-run agent session (which also feeds the tool-consolidation question in 4d), then a pass over the tool run displays using what that session shows.

## Sidebar (comped and agreed 2026-10-30)

**1. Top of the list (built on `feat/sidebar-top`):** the split "+ New chat ▾" button is gone. Two square icon buttons sit beside the search field: new chat (primary, a pencil on a square) and new incognito chat (an eye with a line through it). Tooltips name the shortcuts (⌘N, ⇧⌘N; Ctrl+N, Ctrl+Shift+N elsewhere), which already exist in the menu bar. On the Work tab the same buttons open the New agent form (plain or incognito). Without a model both are disabled with the usual hint. The search field starts in the same place on every tab but is narrower on Chat and Work.

**2. Richer rows (not built):** title with a clock-time or date at the right; the second line is the last-message preview (not the model, which is the same on every row); an empty chat reads "New chat" in grey italics with "No messages yet"; a pin icon for pinned; incognito rows get a small eye icon, a subtle left edge and "Incognito · not saved"; agents show their folder as the second line and a status badge at the right ("Step 4 of 20", "Needs you" in amber, "Failed"); a hairline between groups. Needs core data (the last-message preview and time in `IndexRow`) and the run state per row (the existing `RunBadge` already carries part of it).

**3. Deleting (not built):** the row's "x" becomes a small gray trash can (red on hover) and one click deletes with an **Undo** toast for about 10 seconds. That needs a soft delete in the store: move the session file to a trash folder, restore on Undo, purge old entries on startup. The header trash can (red, "Click again to delete") should behave the same way so the two match. Why: today the row's x needs three clicks if the second lands inside the 0.6 s double-click guard in `confirm.rs`, because the "Delete?" label looks ready at once. Interim fix if the undo work waits: show "Delete?" dimmed for the first 0.6 s.
