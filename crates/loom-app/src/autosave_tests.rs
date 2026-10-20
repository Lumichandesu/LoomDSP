//! The app's side of autosave and crash recovery (`docs/design/autosave.md`):
//! real `App`s in a temporary recovery folder, with a fake clock and fake
//! dialogs. A dropped app stands in for a crash: it leaves its folder and
//! frees its lock.

use super::*;
use crate::recovery::tests::{TempDir, sessions};
use crate::recovery::{Choice, Orphan, Prompt};
use std::fs;

/// Answers the start-up offer and records what was shown.
struct Answer {
    choice: Choice,
    asked: Vec<(String, String)>,
    errors: Vec<String>,
}

impl Answer {
    fn new(choice: Choice) -> Self {
        Self {
            choice,
            asked: Vec::new(),
            errors: Vec::new(),
        }
    }
}

impl Prompt for Answer {
    fn ask(&mut self, orphan: &Orphan) -> Choice {
        self.asked.push(orphan.question());
        self.choice
    }

    fn error(&mut self, message: &str) {
        self.errors.push(message.to_string());
    }
}

/// A recovery folder and a copy of an example patch to work on.
struct Setup {
    tmp: TempDir,
}

impl Setup {
    fn new(label: &str) -> Self {
        let tmp = TempDir::new(label);
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/patches/subtractive.ron");
        fs::copy(example, tmp.0.join("drone.ron")).unwrap();
        Self { tmp }
    }

    fn root(&self) -> PathBuf {
        self.tmp.0.join("recovery")
    }

    fn file(&self) -> PathBuf {
        self.tmp.0.join("drone.ron")
    }

    fn args(&self, patch: Option<PathBuf>) -> Args {
        Args {
            patch,
            recovery_dir: Some(self.root()),
            ..tests::app_args()
        }
    }

    fn boot(&self, patch: Option<PathBuf>, prompt: &mut Answer) -> App {
        App::boot(self.args(patch), prompt)
    }
}

fn folder(app: &App) -> PathBuf {
    app.recovery.as_ref().expect("autosave is on").dir().to_path_buf()
}

/// Calls the autosave tick at `now` until `done` holds, so the writer's
/// reports are read. Fails after 5 s.
fn settle(app: &mut App, now: Instant, mut done: impl FnMut(&App) -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !done(app) {
        assert!(Instant::now() < until, "timed out");
        app.autosave(now);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Turns the filter's cutoff, as a slider would.
fn turn_knob(app: &mut App, position: f32) {
    let _ = app.update(Message::Param {
        module: 4,
        index: 0,
        position,
    });
    let _ = app.update(Message::SliderReleased);
}

/// Turns a knob to `position` and waits for the copy. Returns the session
/// folder.
fn edit_and_keep_at(app: &mut App, position: f32) -> PathBuf {
    let dir = folder(app);
    let t0 = Instant::now();
    app.autosave(t0);
    turn_knob(app, position);
    app.autosave(t0 + recovery::TICK);
    settle(app, t0 + recovery::TICK + recovery::QUIET, |_| {
        dir.join("info.ron").is_file()
    });
    dir
}

fn edit_and_keep(app: &mut App) -> PathBuf {
    edit_and_keep_at(app, 0.8)
}

/// A knob turn is copied 2 s after it is first seen, not before, and the
/// copy holds the new value.
#[test]
fn a_knob_turn_is_copied_after_two_seconds() {
    let setup = Setup::new("knob");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let dir = folder(&app);
    let t0 = Instant::now();
    app.autosave(t0);
    turn_knob(&mut app, 0.8);
    let seen = t0 + recovery::TICK;
    app.autosave(seen);
    app.autosave(seen + Duration::from_millis(1500));
    std::thread::sleep(Duration::from_millis(50));
    app.autosave(seen + Duration::from_millis(1500));
    assert!(!dir.join("patch.ron").exists(), "not before 2 s");
    settle(&mut app, seen + recovery::QUIET, |_| dir.join("info.ron").is_file());
    let copy = Patch::load(dir.join("patch.ron")).unwrap();
    assert_eq!(copy, app.current_patch(), "the copy holds the new value");
    let info: recovery::Info = ron::from_str(&fs::read_to_string(dir.join("info.ron")).unwrap()).unwrap();
    assert_eq!(info.file.as_deref(), Some(setup.file().as_path()));
}

/// Selection, notes, and the transport are not in the patch: no copy.
#[test]
fn selection_and_notes_make_no_copy() {
    let setup = Setup::new("noise");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let dir = folder(&app);
    let t0 = Instant::now();
    let _ = app.update(Message::SelectModule(4));
    let _ = app.update(Message::HoldNote);
    let _ = app.update(Message::HoldNote);
    for i in 0..40 {
        app.autosave(t0 + recovery::TICK * i);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!dir.join("patch.ron").exists());
}

/// Saving deletes the copy; so does undoing back to the saved patch.
#[test]
fn saving_or_undoing_deletes_the_copy() {
    let setup = Setup::new("save");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let dir = edit_and_keep(&mut app);
    let _ = app.update(Message::Save);
    settle(&mut app, Instant::now(), |_| !dir.join("patch.ron").exists());
    assert!(!dir.join("info.ron").exists());

    let dir = edit_and_keep_at(&mut app, 0.3);
    let _ = app.update(Message::Undo);
    assert!(!app.unsaved());
    settle(&mut app, Instant::now() + Duration::from_secs(60), |_| {
        !dir.join("patch.ron").exists()
    });
}

/// A clean exit deletes the session folder.
#[test]
fn a_clean_exit_deletes_the_folder() {
    let setup = Setup::new("exit");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let dir = folder(&app);
    let _ = app.update(Message::CloseRequested);
    assert!(app.recovery.is_none());
    assert!(!dir.exists());
    assert!(sessions(&setup.root()).is_empty());
}

/// After a crash, the next start offers the copy with its file name and
/// time. Discard deletes it.
#[test]
fn the_offer_names_the_file_and_discard_deletes() {
    let setup = Setup::new("offer");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let crashed = edit_and_keep(&mut app);
    drop(app);

    let mut answer = Answer::new(Choice::Discard);
    let app = setup.boot(None, &mut answer);
    let [(title, text)] = answer.asked.as_slice() else {
        panic!("asked once: {:?}", answer.asked)
    };
    assert_eq!(title, "Restore unsaved changes to \"drone.ron\"?");
    assert!(text.contains("A copy of your unsaved changes from "), "{text}");
    assert!(!crashed.exists(), "discarded");
    assert_eq!(app.title(), "Untitled - LoomDSP", "the app starts as usual");
}

/// Not Now keeps the copy, and the same question comes at the next start.
#[test]
fn not_now_asks_again_next_time() {
    let setup = Setup::new("notnow");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let crashed = edit_and_keep(&mut app);
    drop(app);
    let before = fs::read_to_string(crashed.join("patch.ron")).unwrap();

    let mut answer = Answer::new(Choice::NotNow);
    drop(setup.boot(None, &mut answer));
    assert_eq!(answer.asked.len(), 1);
    assert_eq!(
        fs::read_to_string(crashed.join("patch.ron")).unwrap(),
        before,
        "unchanged"
    );

    let mut again = Answer::new(Choice::NotNow);
    drop(setup.boot(None, &mut again));
    assert_eq!(again.asked, answer.asked, "asked again");
}

/// No dialog program: the copy is kept and the status bar says where.
#[test]
fn no_dialog_keeps_the_copy() {
    let setup = Setup::new("nodialog");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let crashed = edit_and_keep(&mut app);
    drop(app);
    let app = setup.boot(None, &mut Answer::new(Choice::Unavailable));
    assert!(crashed.join("patch.ron").exists());
    assert!(
        app.status
            .starts_with("Unsaved changes from a crash were kept. Install zenity or kdialog"),
        "{}",
        app.status
    );
}

/// Restore with the file unchanged: the file stays current, the patch is
/// marked unsaved, and the old folder goes once the new copy is on disk.
#[test]
fn restore_keeps_an_unchanged_file() {
    let setup = Setup::new("restore");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let crashed = edit_and_keep(&mut app);
    let edited = app.current_patch();
    drop(app);

    let mut app = setup.boot(None, &mut Answer::new(Choice::Restore));
    assert_eq!(app.file.as_deref(), Some(setup.file().as_path()));
    assert_eq!(app.title(), "* drone.ron - LoomDSP");
    assert_eq!(app.current_patch(), edited, "the edit is back");
    assert!(
        app.status.starts_with("Restored unsaved changes from "),
        "{}",
        app.status
    );
    let dir = folder(&app);
    settle(&mut app, Instant::now(), |_| !crashed.exists());
    assert!(dir.join("patch.ron").is_file(), "the new copy was written first");
}

/// Restore after the file changed: no current file, a "(recovered)"
/// title, and the file is not touched.
#[test]
fn restore_after_the_file_changed_leaves_it_alone() {
    let setup = Setup::new("changed");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    edit_and_keep(&mut app);
    drop(app);
    let file = setup.file();
    fs::write(&file, fs::read_to_string(&file).unwrap() + "// edited elsewhere\n").unwrap();
    let saved = fs::read(&file).unwrap();

    let mut answer = Answer::new(Choice::Restore);
    let app = setup.boot(None, &mut answer);
    assert!(
        answer.asked[0]
            .1
            .contains("\"drone.ron\" has changed since this copy was taken.")
    );
    assert_eq!(app.file, None);
    assert_eq!(app.title(), "* drone (recovered) - LoomDSP");
    assert_eq!(fs::read(&file).unwrap(), saved);
}

/// A patch never saved comes back as Untitled, marked unsaved.
#[test]
fn restore_of_a_never_saved_patch_is_untitled() {
    let setup = Setup::new("untitled");
    let mut app = setup.boot(None, &mut Answer::new(Choice::NotNow));
    let dir = folder(&app);
    let filter = app.controller.add_module("loom.filter.svf").unwrap();
    app.commit();
    let t0 = Instant::now();
    app.autosave(t0);
    settle(&mut app, t0 + recovery::QUIET, |_| dir.join("info.ron").is_file());
    drop(app);

    let app = setup.boot(None, &mut Answer::new(Choice::Restore));
    assert_eq!(app.file, None);
    assert_eq!(app.title(), "* Untitled - LoomDSP");
    assert!(app.controller.graph().node(filter).is_some());
}

/// A copy that does not load: the error shows, the app starts empty, and
/// the folder is kept as it was.
#[test]
fn a_broken_copy_is_kept() {
    let setup = Setup::new("broken");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let crashed = edit_and_keep(&mut app);
    drop(app);
    fs::write(crashed.join("patch.ron"), "not a patch").unwrap();

    let mut answer = Answer::new(Choice::Restore);
    let app = setup.boot(Some(setup.file()), &mut answer);
    let [error] = answer.errors.as_slice() else {
        panic!("one error: {:?}", answer.errors)
    };
    assert!(error.starts_with("Could not restore the unsaved changes: "), "{error}");
    assert!(error.contains("The copy is kept at "), "{error}");
    assert_eq!(app.controller.graph().nodes().count(), 0, "starts empty");
    assert_eq!(fs::read_to_string(crashed.join("patch.ron")).unwrap(), "not a patch");
}

/// Restore wins over a patch given on the command line, and says so.
#[test]
fn restore_replaces_the_command_line_patch() {
    let setup = Setup::new("cmdline");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    edit_and_keep(&mut app);
    let edited = app.current_patch();
    drop(app);
    let app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::Restore));
    assert_eq!(app.current_patch(), edited);
    assert!(app.status.ends_with("drone.ron was not opened."), "{}", app.status);
}

/// Runs nobody watches neither ask nor touch the folder.
#[test]
fn unattended_runs_leave_recovery_alone() {
    let setup = Setup::new("unattended");
    let mut app = setup.boot(Some(setup.file()), &mut Answer::new(Choice::NotNow));
    let crashed = edit_and_keep(&mut app);
    drop(app);
    let before = fs::read_to_string(crashed.join("patch.ron")).unwrap();
    let runs = [
        Args {
            no_recovery: true,
            recovery_dir: None,
            ..setup.args(None)
        },
        Args {
            control: control::Options {
                control_script: Some(setup.tmp.0.join("script.jsonl")),
                ..Default::default()
            },
            ..setup.args(None)
        },
        Args {
            bench: bench::Options {
                measure_idle: true,
                ..Default::default()
            },
            ..setup.args(None)
        },
    ];
    for args in runs {
        let mut answer = Answer::new(Choice::Discard);
        let app = App::boot(args, &mut answer);
        assert!(answer.asked.is_empty());
        assert!(app.recovery.is_none());
    }
    assert_eq!(fs::read_to_string(crashed.join("patch.ron")).unwrap(), before);
    assert_eq!(sessions(&setup.root()), [crashed]);
}

/// Building the copy on the UI thread stays well under 1 ms for a
/// 300-module patch (autosave.md, acceptance criteria). Debug builds are
/// slower, so the bound here is loose; the release figure is in the PR.
#[test]
fn building_a_copy_is_cheap() {
    let mut app = App::boot(
        Args {
            generate: Some(300),
            ..tests::app_args()
        },
        &mut Answer::new(Choice::NotNow),
    );
    let _ = app.current_patch();
    let runs = 20;
    let t = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(app.current_patch());
    }
    let each = t.elapsed() / runs;
    eprintln!("building a 300-module copy: {each:?}");
    assert!(each < Duration::from_millis(20), "{each:?}");
    app.autosave(Instant::now());
}
