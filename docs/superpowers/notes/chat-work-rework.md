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
