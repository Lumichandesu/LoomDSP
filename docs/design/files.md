# Open, Save, and Save As Specification

How a user opens, saves, and closes patch files in the desktop app, and how the app tells them about unsaved work.

- **Kind:** living specification.
- **Status:** Implemented
- **Decision behind it:** none of its own yet. Builds on [ADR-0008](../adr/0008-patch-format-ron.md) (the patch format) and [ADR-0001](../adr/0001-lock-free-control-audio-boundary.md) (the engine and controller are created together).
- **Use case and requirements:** FR-FILE-01, FR-APP-01.
- **Background:** none.
- **Prior art:** every desktop editor has these commands. We use only the common conventions: shortcuts, a dot or star for unsaved work, and a three-button question.
- **Issues:** [#14](https://github.com/brylie/LoomDSP/issues/14). Follow-ups this spec absorbs: the lights watcher and telemetry monitor must follow a new engine; `*.saved.ron` files next to examples; Open clears the undo history ([undo](undo.md#commands)).
- **Code:** `crates/loom-app/src/files.rs` (dialogs, title, extension), and `App::session`, `App::replace`, `App::save`, and `App::can_discard` in `main.rs`. `History::mark_saved` and `is_saved` in `undo.rs`. `Patch::save` in `loom-patch` writes through a temporary file. The lights, telemetry, and MIDI watchers take an engine generation in their subscription identity.

<!-- Spec first: agree on this before building. Keep it true as the code is
     built. See README.md in this folder. -->

## Goal

> I open a patch from my Documents folder, change the cutoff and move two modules, and press Save. The title bar shows the file name and no unsaved mark. Later I press Open by mistake with changes pending, and the app asks me before it throws them away.

## Terms

| Term | Meaning |
|---|---|
| **Current file** | The path the patch was last opened from or saved to. A new patch has none. |
| **Unsaved changes** | The patch differs from what is on disk at the current file. See [What counts](#what-counts-as-an-unsaved-change). |
| **Replace** | Open builds a new engine and controller, and the old ones are dropped. |

## Behaviour

### Commands

| Command | Shortcut | Result |
|---|---|---|
| **New** | Cmd/Ctrl+N | An empty patch with no current file. |
| **Open...** | Cmd/Ctrl+O | A file dialog. The chosen patch replaces the current one. |
| **Save** | Cmd/Ctrl+S | Writes to the current file. With no current file, it acts as Save As. |
| **Save As...** | Cmd/Ctrl+Shift+S | A file dialog. Writes the patch there, and the new path becomes the current file. |

The commands are buttons in the toolbar and shortcuts anywhere in the window. A command-line patch path opens like Open, with the file as the current file. If it fails to load, the status bar shows the reason (no modal, because there is no patch to protect yet) and the app starts empty.

### What counts as an unsaved change

An edit that goes into the [undo history](undo.md) counts: add, delete, connect, disconnect, parameter, assignment, MIDI mapping, and **module moves**. *Why moves count:* positions are saved in the patch, so a save after a move changes the file.

Not counted: selection, pan, zoom, transport start or stop, playing notes, and Hold note. They are not in the patch file.

The mark is set when a step is recorded. It is cleared by a successful save, an open, or New. Undoing back to the saved state clears it again: the app remembers the history position at save (closes [undo](undo.md#open-points) open point 1). If the history was trimmed past the saved position, the patch stays marked unsaved.

### Window title

`<file name> - LoomDSP` when saved, `* <file name> - LoomDSP` when there are unsaved changes. A patch with no current file shows `Untitled`. The file name is the name with its extension, not the full path (the full path is the hover text of the toolbar file name, if the toolbar shows it).

### Asking before losing work

New, Open, closing the window, and quitting from the OS menu all ask when there are unsaved changes:

> **Save changes to "<file name>"?**
> Your changes will be lost if you do not save them.
> **[Save]  [Don't Save]  [Cancel]**

- **Save:** saves (with Save As for an untitled patch), then continues. If the save fails or its dialog is cancelled, the app stays and nothing is lost.
- **Don't Save:** continues without saving.
- **Cancel:** stops the command.

Quit from the OS menu is the same as closing the window. The app handles the window's close request. A quit that does not reach the app as a close request (for example Cmd+Q on macOS, if the window system skips it) cannot ask: check this by hand. On Linux the question needs `zenity` or `kdialog` (the portal backend does not draw message boxes). Without one, the app does not continue: it keeps the patch and the status bar says to save first. A crash or a forced kill cannot ask; [autosave and crash recovery](autosave.md) cover that case.

### Open replaces the patch

Open replaces the current patch in the same window. *Why:* one window has one engine and one audio output. A second window would need a second engine and audio stream, which is a larger design (see [Out of scope](#out-of-scope)).

On Open and New the app:

1. creates a new engine and controller and swaps them in (audio is rebuilt or the new engine is handed to the running stream);
2. clears the undo history and drops `pending`, `gesture`, and `slider_gesture`;
3. restarts the lights watcher and the telemetry monitor on the new engine (the watcher subscription identity must include the display reader or an engine generation);
4. resets selection, pan, and zoom, and sets the positions from the file;
5. clears MIDI learn state.

If loading fails, the old patch stays running and untouched. The new engine is built only after the file parses.

### Defaults for the dialogs

- **Extension:** `.ron`. Save As adds it if the user omits it. The filter shows `.ron` files.
- **Starting folder:** the folder of the current file; otherwise the user's Documents folder. The app does not remember a folder across runs in this version.
- Save As to an existing file uses the dialog's own overwrite confirmation.
- Save writes the patch where it was opened. Opening an example and pressing Save would overwrite the example, so the first Save of a file under `examples/` is not special-cased; the user can use Save As. The `<name>.saved.ron` behaviour goes away.

### When something fails

Failures show in a modal message with the reason and one **OK** button, and the status bar repeats it.

| Failure | Message |
|---|---|
| Open: cannot read (no permission, missing) | Could not open "<name>": <system reason>. |
| Open: does not parse or does not load | Could not open "<name>": <the patch error, with line when known>. The current patch is unchanged. |
| Save: cannot write | Could not save "<name>": <system reason>. Your changes are not saved. |

A failed save keeps the unsaved mark. A save writes to a temporary file in the same folder and renames it, so a full disk cannot leave a half-written patch. *Why:* a user's only copy must survive a failed save.

### Dialog crate

[`rfd`](https://crates.io/crates/rfd) (MIT or Apache-2.0). It supports macOS, Windows, and Linux (portal or GTK). To check before building: `cargo deny check` on all three platforms, and that the Linux backend does not pull in a GPL or LGPL crate (GTK3 is a system library, loaded at run time, not linked source; confirm with deny). Dialogs are the blocking `rfd` calls, run on the UI thread as the operating systems expect. Audio runs on its own thread and keeps playing while a dialog is open.

## Rules

1. **Never lose a patch silently.** Every path that would drop unsaved changes asks first. *Why:* work is the user's most expensive thing.
2. **A failed open changes nothing.** *Why:* a typo in a file must not kill the patch the user is playing.
3. **A failed save keeps the mark and the old file.** *Why:* see above.
4. **The engine parts follow the controller.** Anything that holds the engine's `Telemetry`, display reader, or probes is rebuilt on Open. *Why:* a stale subscription reads the old engine without any error.

## Acceptance criteria

- [ ] Given a patch with a moved module, when saved and reopened, then positions are restored (test with a temporary file).
- [ ] Given a clean patch, when a knob turns, then the title shows the mark; when saved, it clears.
- [ ] Given a saved patch, when an edit is made and undone, then the mark clears.
- [ ] Given unsaved changes, when New, Open, or window close is chosen, then the question appears; Cancel keeps everything.
- [ ] Given a file that does not parse, when opened, then the message appears and the current patch is unchanged.
- [ ] Given a read-only target, when saved, then the message appears and the mark stays.
- [ ] After Open, undo is disabled, and the lights and meters show the new patch (manual check).
- [ ] Saving does not leave a `*.saved.ron` file next to examples.
- [ ] `cargo deny check` passes with the dialog crate on macOS, Windows, and Linux (CI).

## Out of scope

- Several patches in several windows (later; needs one engine per window).
- Recent files list and remembered folders. Autosave and crash recovery have their own spec: [autosave.md](autosave.md).
- Embedded assets (FR-FILE-06) and migrations (FR-FILE-02, from the first release).
- Drag a file onto the window to open it.

## Open points

1. **Mark on moves.** *Leaning:* count them, as above. The alternative is to save positions without raising the mark.
2. **Open in the same window or a new one.** *Leaning:* same window.
3. **Linux dialog backend.** *Leaning:* `rfd` with the XDG portal backend, to avoid linking GTK. Confirm with `cargo deny` and a test on a Linux desktop.
4. **Reuse the engine on Open.** Rebuilding the engine is simple; reusing it with a cleared graph keeps the audio stream running. *Leaning:* rebuild and hand the new engine to the stream, if the audio host allows swapping an engine; otherwise reopen the stream.
