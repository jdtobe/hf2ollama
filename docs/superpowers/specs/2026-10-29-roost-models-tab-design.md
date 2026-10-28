# Models tab: Browse and Installed in one view

Status: approved layout "A" from the comps (https://claude.ai/artifact/V5KLZeHo4wYmn6CYTWutbe).

## Intent

Today the Models tab is two screens behind a switch. It becomes one view,
like VS Code's extensions view: one search box, one list with an Installed
section and a Hugging Face section, and one detail pane.

Decided with the user:

- Layout A. The Browse (⌘3) and Installed (⌘4) shortcuts are dropped; the menu
  has one Models entry (⌘3).
- The detail pane shows the action buttons first and the model details (the
  "About" card) below them.
- Installed models get **Chat** and **Work** buttons next to Delete.
- The usage bar keeps its two segments (imported with Roost, pulled from
  Ollama) because Ollama does not report free space.

## Layout

- The top bar's search field is shown on this tab. It filters Installed (every
  word must appear in the name, repository or quantization) and searches
  Hugging Face (as Browse does today, debounced).
- Left column, 310 to 360 px wide:
  1. A chips row: **Filters ▾** (a popover holding today's filter controls:
     format, parameters, fits my machine, task), **Fits my machine** (a toggle
     chip), and the sort chip. The 200 px filter rail is removed.
  2. The usage bar and a one-line summary.
  3. **Installed** section: one compact row per model (name, size, quant, and
     the source repository when there is one). Hidden while Ollama is
     unreachable, with a one-line explanation.
  4. **Hugging Face** section: today's result rows. A result whose repository
     an installed model came from shows an **installed** badge.
- Detail pane: the selection is either an installed model or a repository.
  - Repository: today's detail pane (header, notices, About, files, import
    plan) unchanged.
  - Installed model: name and facts, **Chat**, **Work** and **Delete…** (two
    clicks with the existing confirm delay), then **Details** (family,
    context, capabilities, digest, run command), then **About**: the source
    repository's model card when the model came from one (with a link that
    selects the repository), otherwise "Pulled from the Ollama library".

## Behaviour

- Chat starts a new chat with the model and switches to the Chat tab. Work
  opens the New agent form with the model chosen and switches to Work.
- Selecting an installed model requests its details once; selecting a
  repository works as before.
- Deleting keeps the existing toast and list refresh. If the deleted model was
  selected, the selection clears.
- The tab remembers its selection while you visit other tabs.

## Stages (one pull request each)

1. **View merge:** `View::Models` replaces Browse and Installed; the list with
   both sections, the chips row, the installed detail with Chat, Work and
   Delete, menu and navigation changes. The old Installed table and the filter
   rail are removed with their tests.
2. **Cross-links and About:** the installed badge on results, the About card on
   installed models that came from a repository, a constant "Hugging Face" section title (a "Popular" title
   was tried and dropped: it stops being true when the sort order changes).

## Testing

Layout tests at 900x640 and 1180x820 in all four themes: both sections and the
chips row stay inside the list, the detail pane keeps at least 520 px at the
minimum width, and every button is visible and does what it says (the Chat and
Work buttons send the right commands and switch tabs; Delete takes two clicks).
