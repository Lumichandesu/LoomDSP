# Autosave and Crash Recovery Specification

The desktop app keeps a recovery copy of the open patch while it has unsaved changes, and after a crash it offers to bring that copy back.

- **Kind:** living specification.
- **Status:** Implemented
- **Decision behind it:** none of its own. Builds on [ADR-0008](../adr/0008-patch-format-ron.md) (the patch format) and [Open, Save, and Save As](files.md), which leaves autosave and crash recovery out of its scope.
- **Use case and requirements:** NFR-REL-01 (the app autosaves and recovers after a crash), UC3 (perform live).
- **Background:** none.
- **Prior art:** most desktop editors keep a hidden copy of unsaved work and offer it after a crash. We use only that convention: a quiet background copy, and a question at the next start.
- **Issues:** [#15](https://github.com/brylie/LoomDSP/issues/15).
- **Code:** `crates/loom-app/src/recovery.rs` (folders, locks, the writer thread, the schedule, the offer), and `App::boot`, `App::restored`, and `App::autosave` in `main.rs`. It reuses `Patch::from_graph` and `Patch::save` from `loom-patch`, finds the folders with `etcetera`, and formats times with `jiff`. Tests: `recovery/tests.rs` and `autosave_tests.rs`.

<!-- Spec first: agree on this before building. Keep it true as the code is
     built. See README.md in this folder. -->

## Goal

> I have patched for an hour without saving. The app crashes, or my laptop runs out of battery while the app is open. When I start LoomDSP again, it tells me it found my unsaved patch from a few seconds before the crash, and one click brings it back.

> On stage the app crashes in the middle of a set. I restart it, press Restore, and I am playing the same patch again within seconds.

## Terms

| Term | Meaning |
|---|---|
| **Session** | One run of the app, from start to exit. Each session has its own recovery folder. |
| **Recovery copy** | The patch as it was at the last autosave, in the session's recovery folder. |
| **Clean exit** | The app closes through its own close path (window close, quit), after any question about unsaved changes. |
| **Orphan** | A recovery folder whose session is no longer running and did not exit cleanly. |
| **Current file** and **unsaved changes** | As in [files.md](files.md#terms). |

## Behaviour

### When autosave runs

The app writes a recovery copy **2 seconds after the last change**. If changes keep coming without a 2 second pause (a long knob turn, a long drag), it writes anyway **10 seconds after the first change it has not written yet**. So at most about 10 seconds of work can be lost.

| Value | Setting |
|---|---|
| Quiet time after the last change | 2 s |
| Longest wait while changes keep coming | 10 s |

*Why these values:* 2 seconds of quiet means the player has paused, so the write does not land in the middle of a gesture. The 10 second ceiling bounds the loss when a gesture never stops, for example a slow filter sweep during a set. Writes are small (a patch file is a few kilobytes) and happen on another thread, so this rate costs nothing the player can notice.

A **change** is anything that sets the unsaved mark ([files.md](files.md#what-counts-as-an-unsaved-change)): add, delete, connect, disconnect, parameter, assignment, MIDI mapping, and module moves. Undo and redo are changes too. A gesture still in progress also counts: the copy holds the patch as it is when the copy is taken, including a knob halfway through a turn or a module halfway through a drag. *Why:* a crash in the middle of a long gesture should still keep most of it.

Not changes: selection, pan, zoom, transport start or stop, playing notes, and Hold note. They are not in the patch file, so they do not start the timer.

**Only unsaved work is copied.** When the patch has no unsaved changes (just saved, just opened, or undone back to the saved state), the app deletes the session's recovery copy instead of writing one. *Why:* the file on disk already holds that patch. A copy of it would only cause a pointless question after a crash.

### Where the recovery copy lives

All sessions share one recovery folder, in the operating system's per-user data folder:

| OS | Folder |
|---|---|
| macOS | `~/Library/Application Support/LoomDSP/recovery/` |
| Windows | `%LOCALAPPDATA%\LoomDSP\recovery\` |
| Linux and other Unix | `$XDG_STATE_HOME/loomdsp/recovery/`, or `~/.local/state/loomdsp/recovery/` when `XDG_STATE_HOME` is unset, empty, or not an absolute path |

*Why these folders:* macOS and Windows use the same base folder as [preferences](preferences.md#where-the-file-lives), so all LoomDSP files on one machine are in one place. On Linux the XDG specification separates settings (config) from data the app keeps between runs (state); a recovery copy is state.

Inside it, one `scan.lock` file that instances lock briefly at start (see How the app knows the last run crashed), and **one folder per session**, named `<start time>-<process ID>`, for example `20261020T143005-48211`. Each session folder holds:

| File | Content |
|---|---|
| `lock` | Empty. The session holds an exclusive OS file lock on it for its whole run, through `std::fs::File::try_lock` (Rust 1.89 and later; the workspace needs 1.99). |
| `patch.ron` | The recovery copy. An ordinary patch file: Open can load it by hand. Present only while there are unsaved changes. |
| `info.ron` | Facts about the copy (below). |

`info.ron`:

```ron
(
    version: 1,
    file: Some("/Users/me/Documents/drone.ron"),
    file_modified: Some("2026-10-20T14:02:11Z"),
    file_size: Some(4812),
    written: "2026-10-20T14:31:47Z",
)
```

| Field | Meaning |
|---|---|
| `file` | The current file when the copy was taken, or `None` for a patch never saved. |
| `file_modified` | The modification time of the current file when the session last opened or saved it. `None` when there is no current file. |
| `file_size` | The size in bytes of the current file at that moment. `None` when there is no current file. |
| `written` | When the copy was written. |

*Why one folder per session, not one per patch or one for all:* one window holds one patch, so a session and its patch are the same thing. A folder per session lets two app instances run side by side without overwriting each other's copy, and a patch never saved has no file name to key on. *Why a plain patch file:* if the app itself cannot start, a user can still find `patch.ron` and open it.

A command-line flag `--recovery-dir <folder>` uses another folder instead. *Why:* tests must not read or change the real folder, as with `--preferences`.

### Finding the folders

The [`etcetera`](https://crates.io/crates/etcetera) crate (MIT or Apache-2.0) finds the per-user folders. Each row of the table in Where the recovery copy lives comes from one of its strategies:

| OS | Strategy and function |
|---|---|
| macOS | `Apple`, `data_dir()` |
| Windows | `Windows`, `cache_dir()` (the Local AppData known folder; its `data_dir()` is the roaming folder, which we do not want) |
| Linux and other Unix | `Xdg`, `state_dir()` |

*Why `etcetera`, not `dirs`:* `dirs` depends on `option-ext`, which is MPL-2.0, and `deny.toml` does not allow MPL (ADR-0006). `etcetera` depends only on `cfg-if` and `windows-sys`. The paths in the table are what matters: the unit test of the path function checks them, whatever the crate.

### How the app knows the last run crashed

At start, before the window opens, the app:

1. takes an exclusive lock on `recovery/scan.lock`, waiting if another instance holds it;
2. creates its own session folder and takes the lock on its `lock` file;
3. looks at every other session folder. If it can take that folder's lock, the session that owned it is no longer running. If it cannot, another instance is running, and the app leaves that folder alone. It keeps holding the lock of every orphan it found until it has dealt with that orphan, so no other instance offers or deletes the same one;
4. releases `scan.lock`.

*Why `scan.lock`:* without it, two instances starting at the same moment could each see the other's new folder before its lock is taken, and delete it as an empty orphan. Under `scan.lock`, a folder is never looked at between its creation and its lock. The scan touches only small files, so another instance waits a few milliseconds at most.

A clean exit deletes the session's own folder, as the last thing it does. So a folder whose lock is free was left by a session that did not exit cleanly: a crash, a forced kill, or a power loss.

- An orphan **with** a `patch.ron` is offered for recovery (below).
- An orphan **without** a `patch.ron` had no unsaved work. The app deletes it without asking.

*Why a lock, not only a marker written at exit:* a marker alone cannot tell a crashed session from one that is still running in another instance. The operating system releases a file lock when a process ends, however it ends, so a free lock is reliable proof that the owner is gone.

### The offer at start

When there is an orphan with a recovery copy, the app shows a question at start, before the window opens and before it loads any patch:

> **Restore unsaved changes to "drone.ron"?**
> LoomDSP did not close properly. A copy of your unsaved changes from 20 October, 14:31 was kept.
> **[Restore]  [Discard]  [Not Now]**

For a patch never saved, the name is `Untitled`. The time is `written`, in local time.

| Choice | Result |
|---|---|
| **Restore** | Loads the recovery copy as the open patch (see After a restore). |
| **Discard** | Deletes the orphan folder. Asks no second question. The app starts as it would without the offer. |
| **Not Now** | Leaves the orphan folder as it is. The app starts as it would without the offer, and the same question comes at the next start. |

*Why Not Now:* on stage, the player may want to start a known patch fast and look at the lost one later. Discard is final; Not Now is the safe default when unsure.

*Why before the window:* the window then opens on the patch the player chose, never first on an empty or older one.

If the window is closed or the dialog cannot be shown, the result is **Not Now**. On Linux without `zenity` or `kdialog` (see [files.md](files.md#asking-before-losing-work)), the status bar says: `Unsaved changes from a crash were kept. Install zenity or kdialog to restore them, or open <path to patch.ron>.` *Why:* nothing is ever deleted without the user's choice.

**Several orphans** (two instances crashed, or the user chose Not Now before): the app asks about the newest one only. The others stay for later starts. *Why:* one window holds one patch, so one restore per start is all the app can do. A list would be a larger design (see Later).

**A patch path on the command line:** the offer still comes first. Restore replaces the command-line patch, and the status bar says so: `Restored unsaved changes from 20 October, 14:31. drone.ron was not opened.` The other two choices load the command-line patch as usual. *Why:* the player who restarts after a crash usually starts the app the same way as before, and the copy has more recent work.

**Non-interactive runs** never ask and never touch other sessions' folders: `--screenshot`, `--dump-ui`, `--bench`, `--measure`, `--measure-idle`, `--control`, `--control-script`, and `--no-recovery`. They also write no recovery copy. *Why:* scripts and tests must not stop at a dialog, and must not use up or delete a real user's recovery copy.

### When the saved file has changed since the copy

Before asking, the app reads the modification time and size of `file`. If either is **different from** `file_modified` or `file_size` (a time later or earlier), the file was changed after the copy's session last saved or opened it: by another instance, by hand, by a sync tool, or by restoring a backup (which can bring back an older time). The question then adds a line:

> "drone.ron" has changed since this copy was taken. Restoring will not overwrite it.

*Why any difference, not only a later time:* an earlier time does not prove the file is the same. Only an equal time and an equal size are taken as "unchanged".

When the player chooses Restore, the app reads the time and size again, just before it sets the current file. If they changed while the question was open, the copy opens without a current file, as above. *Why:* the file can change while the dialog waits.

*Why not a content hash:* the time and size together catch the usual changes: a save by another instance, a sync, or a backup restored. They do not prove the content is the same (see Limitation 6). A content hash would add a dependency and a read of the whole file at every open and save. This is an accepted trade-off.

And Restore opens the copy **without a current file** (see After a restore), so Save asks where to write instead of replacing the changed file.

If `file` no longer exists, the question adds: `"drone.ron" no longer exists.` Restore again opens the copy without a current file.

*Why:* the recovery copy is not always the newest work. The app does not guess which is right. It shows both facts and keeps both versions, so the player can compare them and choose.

### After a restore

| Restored patch | Current file | Unsaved mark | Title |
|---|---|---|---|
| Had a file, file unchanged since | The same file | Set | `* drone.ron - LoomDSP` |
| Had a file, file changed or missing | None | Set | `* drone (recovered) - LoomDSP` |
| Never saved | None | Set | `* Untitled - LoomDSP` |

- The status bar says `Restored unsaved changes from 20 October, 14:31.`
- The patch is **always marked unsaved**. *Why:* it differs from what is on disk, or there is nothing on disk. The mark tells the player to save it.
- It **keeps its file path** only when the file is unchanged since the copy's session last touched it. Then Save writes the recovered work where the player expects. Otherwise there is no current file, and Save acts as Save As, starting in the file's folder.
- The undo history is empty. *Why:* the history is not saved in the copy. The patch counts as changed, as after `History::clear` ([files.md](files.md#what-counts-as-an-unsaved-change)).
- Positions come from the copy, as for Open. Selection, pan, and zoom are reset.
- Restore builds a new engine and controller, as Open does ([files.md](files.md#open-replaces-the-patch)). If the copy does not load, the app shows `Could not restore the unsaved changes: <the patch error>. The copy is kept at <path to patch.ron>.`, keeps the orphan folder, and starts empty.

**The orphan is deleted only after the new session has its own copy.** Right after a successful restore, the app writes the new session's recovery copy at once (no 2 second wait), and deletes the orphan folder when that write succeeds. *Why:* if the app crashes again during the restore, the work must still be somewhere.

### Writing without stalling the UI

1. On the UI thread, the app builds the patch data with `Patch::from_graph` and the module positions, as Save does. This is a copy of plain data and is fast.
2. It sends that data to a **recovery writer thread**: one plain `std::thread` that the app starts with the session and stops at exit. The channel holds one pending copy. A newer copy replaces an older one that the writer has not started.
3. The writer turns the patch into RON text and writes `patch.ron` through `Patch::save` (a temporary file and a rename, so a crash during the write leaves the old copy whole), then `info.ron` the same way.
4. The writer sends the result back to the UI thread, which reads it at its next check.

**`patch.ron` and `info.ron` always belong to the same patch.** The writer deletes them together whenever the copy is deleted (a save, New, Open, Don't Save), `info.ron` first, then `patch.ron`. Their content only changes together at those moments: between them, a new write changes `patch.ron` and only the `written` time in `info.ron`. So after a crash the folder holds a matching pair, `patch.ron` alone, or `info.ron` alone. `patch.ron` without `info.ron`, or with an `info.ron` that does not parse, is offered as a patch never saved, with no current file and the time of `patch.ron` itself. `info.ron` without `patch.ron` is an orphan with no copy, and is deleted. *Why:* a crash between the two writes must never pair a new patch with an old file path, because Save would then overwrite the wrong file.

*Why a thread of its own:* file writes can block for a long time on a slow or network disk, and the UI thread must keep drawing. The audio thread is never involved (hard rule 1). *Why latest wins:* only the newest copy matters. A queue of old copies would only delay it.

Deleting the copy when the patch becomes saved goes through the same writer, in order with the writes. *Why:* a delete must never overtake a write that was sent before it.

Save, Open, and New stay on the UI thread as today ([files.md](files.md)). Only recovery writes use the writer thread.

### When a write fails

The first failure shows in the status bar, with no modal:

> Autosave failed: <system reason>. Your changes are not protected against a crash. Save to keep them.

The app tries again every 10 seconds while the copy is still out of date, whether or not new changes come. It does not repeat the message while writes keep failing, and says `Autosave works again.` after the first write that succeeds. *Why:* a full disk should not interrupt playing, but the player must know their safety net is gone.

If the recovery folder cannot be created or locked at start, autosave is off for the session and the status bar says: `Autosave is off: <system reason>.` The app still starts. *Why:* the app must reach a playing state on stage even when its data folder is broken, as with [preferences](preferences.md#reading-the-file).

### Clean exit

On window close or quit, after any "Save changes?" question ([files.md](files.md#asking-before-losing-work)):

1. the app stops the writer thread and waits up to 2 seconds for its current write, if any, to finish. If the write is still running after that, the app exits at once and skips steps 2 and 3. The session folder stays behind as an orphan. At the next start it is deleted without a question if it holds no `patch.ron`, and offered as usual if it does;
2. it deletes `info.ron`, then `patch.ron`;
3. it closes the `lock` file, which releases the lock, and deletes the session folder.

*Why the 2 second limit:* a write can hang on a slow or network disk, and closing the app must never hang with it. Leaving the folder is safe: it holds either the old copy or the new one (the rename is atomic), and at worst the player sees one extra question at the next start. *Why this order:* Windows cannot delete a file that is still open, so the lock is closed before the folder goes. The copy is deleted first, so another instance that takes the free lock in that moment finds no `patch.ron` and only removes an empty folder.

**Don't Save** counts as a clean exit: the user chose to drop the changes, so the copy goes with them. **Cancel** keeps the app open and changes nothing. If deleting the folder fails, the next start finds an orphan; if it holds a `patch.ron`, the user is asked about it. *Why:* a leftover question is better than lost work.

On New or Open, after the question about unsaved changes, the session's copy (`info.ron`, then `patch.ron`) is deleted, because the new patch starts with no unsaved changes. The session folder and lock stay. *Why:* the session goes on; only its patch changed.

## Rules

1. **Recovery never deletes work the user did not choose to drop.** Only Discard, Don't Save, a save, or a confirmed restore removes a recovery copy. A failed dialog, a closed dialog, or a failed restore keeps it. *Why:* the feature exists to stop silent loss; it must not cause any.
2. **Only unsaved work is copied.** *Why:* a copy of a saved patch would only add a pointless question after a crash.
3. **One recovery folder per session, held by an OS file lock.** *Why:* it tells a crashed session from a running one, and lets instances run side by side.
4. **Writing never stalls the UI and never touches the audio thread.** The UI thread only builds plain patch data; a writer thread does the I/O. *Why:* hard rule 1 for audio, and a frozen UI on stage is as bad as a crash.
5. **Writes are atomic and in order.** Each file is written through a temporary file and a rename, and writes and deletes reach the disk in the order they were sent. *Why:* a crash during a write must leave the previous copy whole.
6. **A restored patch is always marked unsaved, and never silently overwrites a file that changed since the copy.** *Why:* the player decides which version wins.
7. **Recovery lives in `loom-app` only.** No engine crate reads or writes recovery files. *Why:* hard rule 6.
8. **Non-interactive runs neither ask nor touch recovery files.** *Why:* tests and scripts must not block on a dialog or consume a user's copy.

## Acceptance criteria

Tests point `--recovery-dir` (or the function behind it) at a temporary folder. They never touch the real folder.

- [ ] Given a clean patch, when a knob turns once, then `patch.ron` appears 2 s later (test with a fake clock), and loading it gives the new knob value.
- [ ] Given changes every 500 ms without pause, then a copy is written no later than 10 s after the first change.
- [ ] Given selection, pan, zoom, transport, or note changes only, then no copy is written.
- [ ] Given unsaved changes and a copy on disk, when the patch is saved, then `patch.ron` is deleted. When an edit is made and undone back to the saved state, `patch.ron` is deleted at the next autosave.
- [ ] Given a session folder whose lock is held by another process, when the app starts, then it does not offer, change, or delete that folder.
- [ ] Given an orphan folder with a free lock and a `patch.ron`, when the app starts, then the offer appears with the file name and the `written` time.
- [ ] Given an orphan folder with a free lock and no `patch.ron`, when the app starts, then it is deleted without a question.
- [ ] Given the offer, when Discard is chosen, the orphan folder is gone. When Not Now is chosen, the folder is unchanged and the offer appears at the next start.
- [ ] Given an orphan with a current file unchanged since, when restored, then the current file is that file and the title shows the unsaved mark.
- [ ] Given an orphan whose current file was modified after `file_modified`, then the offer carries the "has changed" line, and after Restore there is no current file and the saved file is unchanged.
- [ ] Given an orphan for a never-saved patch, when restored, then the title is `* Untitled - LoomDSP`.
- [ ] After a restore, the new session's `patch.ron` exists before the orphan folder is deleted.
- [ ] Given an orphan whose `patch.ron` does not parse, when Restore is chosen, then the error appears, the app starts empty, and the orphan folder is unchanged.
- [ ] Given a recovery folder that cannot be written, when a change is made, then the status bar shows the failure once and the app keeps running.
- [ ] Given a failed write and no further changes, when the folder becomes writable again, then a copy is written within 10 s and the status bar says autosave works again.
- [ ] Given two instances started at the same moment against one recovery folder, then neither deletes or offers the other's session folder (test with two threads or processes, many runs).
- [ ] Given an orphan with `patch.ron` and no `info.ron`, when restored, then there is no current file.
- [ ] Given an orphan whose current file has a modification time earlier than `file_modified`, then the offer carries the "has changed" line, and after Restore there is no current file.
- [ ] Given `--screenshot`, `--dump-ui`, `--control`, or `--no-recovery`, when an orphan exists, then no dialog appears and the orphan is unchanged.
- [ ] On clean exit, and on exit after Don't Save, the session folder is deleted.
- [ ] Given a write that does not finish (a writer blocked by the test), when the window closes, then the app exits within 2.5 s and the session folder is left for the next start.
- [ ] Given an orphan whose current file has the same modification time but a different size, then the offer carries the "has changed" line. Given a file that changes while the question is open, then after Restore there is no current file.
- [ ] Given the default paths, then on each OS the folder matches the table in Where the recovery copy lives. (Unit test of the path function against the environment variables the app reads. The test reads them and does not set them: setting environment variables while other tests run is unsound.)
- [ ] With a 300-module patch (`--generate 300`), building the copy on the UI thread takes under 1 ms (benchmark, in [p1-canvas.md](../benchmarks/p1-canvas.md) or its own note).
- [ ] **Manual check.** On macOS, Windows, and Linux: make changes, wait 3 s, kill the app with the OS (Activity Monitor, Task Manager, `kill -9`). Start it again: the offer appears, Restore brings back the changes, and the title shows the unsaved mark.
- [ ] **Manual check.** On a slow disk (a network share or USB stick as `--recovery-dir`), turning a knob continuously for 30 s keeps the canvas responsive.

## Out of scope

- Autosaving to the patch's own file. Autosave only writes recovery copies; the user's file changes only when they save. *Why:* the user decides when their file changes.
- Restoring the undo history.
- A list of all orphans to restore or discard at once (see Later).
- Recovery for several windows (there is one window per app; see [files.md](files.md#out-of-scope)).
- Forcing each copy to disk (`fsync`) before the rename. See Limitations.
- A preference to change the timing or turn autosave off. The [preferences](preferences.md) window can gain one later.
- Recovery in the web runtime and in `loom play` and `loom render`.

## Limitations

Known gaps, accepted to keep the first version simple. Each can be revisited if it causes trouble in practice.

1. **Up to about 10 seconds of work can be lost.** That is the longest wait between autosaves while changes keep coming.
2. **Power loss or an OS crash can lose the last copy.** The writer does not force the copy to disk (`fsync`) before the rename. After a power loss, the newest copy may be missing, or on some file systems the renamed file may be empty. An app crash or a forced kill is not affected, because the operating system still writes out what the app wrote. A failed write never leaves a half-written copy in place of a good one: the old copy stays whole until the rename. An empty or broken copy is handled like any copy that does not load: the error shows, the app starts empty, and the folder is kept. *Why accept it:* power loss is rare, and the copy is a safety net, not the user's file.
3. **Only the newest orphan is offered at each start.** Others wait for later starts. Someone with many orphans must restore or discard them one start at a time.
4. **Orphans are kept until the user acts.** Not Now keeps a copy for ever. Each is a few kilobytes.
5. **The undo history is not restored.**
6. **A changed file can look unchanged.** The app compares the file's modification time and size, not its content. A file changed to the same size within the timestamp resolution of its file system (2 seconds on FAT), or by a tool that keeps the old modification time, looks unchanged. Restore then keeps the path, and Save writes over that change.

## Settled in review

- The file lock uses `std::fs::File::try_lock`. The workspace's minimum Rust version is raised to 1.99, the latest stable release.
- The folders come from `etcetera` (Finding the folders).
- The writer does not `fsync` (Limitation 2).
- Only the newest orphan is offered at each start (Limitation 3).
- With a patch on the command line, the offer comes first and Restore wins.

## Later

Not needed to build this spec. Each becomes an issue when it is needed.

- **A list of orphans.** If users collect many orphans through Not Now (Limitations 3 and 4), add a "Recover unsaved patches..." window that lists them.
- **A copy from the panic hook.** NFR-REL-02 asks that a UI panic does not stop audio. If the app later survives UI panics, it could also write a copy at once from the panic hook. The 10 s bound is enough for now.

## Open points

None.
