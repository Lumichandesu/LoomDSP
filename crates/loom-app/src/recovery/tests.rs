use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A fresh folder under the system temp folder, deleted when dropped.
pub(crate) struct TempDir(pub PathBuf);

impl TempDir {
    pub(crate) fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("loom-recovery-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn patch(name: &str) -> Patch {
    Patch::new(name)
}

/// Session folders under `root`.
pub(crate) fn sessions(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<_> = fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// Waits until `done` holds, calling `tick` in between, for up to 5 s.
fn wait_for(mut tick: impl FnMut(), mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < until, "timed out");
        tick();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Leaves a folder as a crashed session would: a copy and no live lock.
pub(crate) fn crashed_session(root: &Path, copy: &Patch, info: Option<&Info>) -> PathBuf {
    let Startup { mut session, orphan } = start(root).unwrap();
    drop(orphan);
    let t = Instant::now();
    let info = info.cloned();
    session.schedule.write_now();
    session.tick(t, Some(copy.clone()), || {
        info.clone().unwrap_or_else(|| Info::new(None, None, SystemTime::now()))
    });
    let dir = session.dir.clone();
    wait_for(|| {}, || dir.join(INFO).is_file());
    // No clean exit: dropping keeps the folder and frees the lock.
    drop(session);
    dir
}

// --- Schedule ---

/// A change is written after 2 s of quiet, not before.
#[test]
fn writes_after_two_quiet_seconds() {
    let mut s = Schedule::default();
    let t0 = Instant::now();
    let p = Some(patch("a"));
    assert_eq!(s.tick(t0, p.clone()), None);
    assert_eq!(s.tick(t0 + Duration::from_millis(1500), p.clone()), None);
    assert_eq!(s.tick(t0 + QUIET, p.clone()), Some(Action::Write(patch("a"))));
}

/// Changes every 500 ms never give 2 s of quiet, but a copy is written no
/// later than 10 s after the first.
#[test]
fn writes_within_ten_seconds_while_changes_keep_coming() {
    let mut s = Schedule::default();
    let t0 = Instant::now();
    let mut written = None;
    for i in 0..30u32 {
        let now = t0 + Duration::from_millis(500) * i;
        if let Some(Action::Write(_)) = s.tick(now, Some(patch(&format!("v{i}")))) {
            written = Some(now - t0);
            break;
        }
    }
    let after = written.expect("a copy was written");
    assert!(after <= LONGEST, "written after {after:?}");
    assert!(after >= LONGEST - Duration::from_millis(500), "not early: {after:?}");
}

/// Nothing to keep and nothing on disk: nothing to do.
#[test]
fn no_unsaved_work_writes_nothing() {
    let mut s = Schedule::default();
    let t0 = Instant::now();
    for i in 0..30 {
        assert_eq!(s.tick(t0 + Duration::from_secs(i), None), None);
    }
}

/// Once the patch is saved again, the copy is deleted at the next tick.
#[test]
fn saved_work_deletes_the_copy_at_once() {
    let mut s = Schedule::default();
    let t0 = Instant::now();
    s.tick(t0, Some(patch("a")));
    let Some(Action::Write(_)) = s.tick(t0 + QUIET, Some(patch("a"))) else {
        panic!("written")
    };
    s.sent(1, Some(patch("a")));
    s.done(1, true, t0 + QUIET);
    assert_eq!(s.tick(t0 + QUIET + TICK, None), Some(Action::Delete));
}

/// A job on its way is not sent again; a newer target is.
#[test]
fn a_job_in_flight_is_not_repeated() {
    let mut s = Schedule::default();
    let t0 = Instant::now();
    s.tick(t0, Some(patch("a")));
    assert!(s.tick(t0 + QUIET, Some(patch("a"))).is_some());
    s.sent(1, Some(patch("a")));
    assert_eq!(s.tick(t0 + QUIET + TICK, Some(patch("a"))), None);
    // An older job's report does not count for the newer one.
    s.tick(t0 + QUIET + TICK * 2, Some(patch("b")));
    let Some(Action::Write(_)) = s.tick(t0 + QUIET * 3, Some(patch("b"))) else {
        panic!("b is due")
    };
    s.sent(2, Some(patch("b")));
    s.done(1, true, t0 + QUIET * 3);
    assert_eq!(
        s.tick(t0 + QUIET * 3 + TICK, Some(patch("b"))),
        None,
        "b still on its way"
    );
    s.done(2, true, t0 + QUIET * 3 + TICK);
    assert_eq!(s.tick(t0 + QUIET * 4, Some(patch("b"))), None, "b on disk");
}

/// After a failure, the next try comes 10 s later, with no new change.
#[test]
fn a_failed_write_is_tried_again_after_ten_seconds() {
    let mut s = Schedule::default();
    let t0 = Instant::now();
    s.tick(t0, Some(patch("a")));
    s.tick(t0 + QUIET, Some(patch("a")));
    s.sent(1, Some(patch("a")));
    s.done(1, false, t0 + QUIET);
    let mut now = t0 + QUIET;
    while now < t0 + QUIET + RETRY {
        assert_eq!(s.tick(now, Some(patch("a"))), None, "too early at {:?}", now - t0);
        now += TICK;
    }
    assert!(s.tick(t0 + QUIET + RETRY, Some(patch("a"))).is_some());
}

/// After a restore, the copy is written at the next tick.
#[test]
fn write_now_skips_the_quiet_time() {
    let mut s = Schedule::default();
    s.write_now();
    assert!(s.tick(Instant::now(), Some(patch("a"))).is_some());
}

// --- Folders and locks ---

/// Each start gets its own folder, locked while the session lives.
#[test]
fn a_session_folder_is_locked_while_it_runs() {
    let root = TempDir::new("lock");
    let Startup { session, orphan } = start(&root.0).unwrap();
    assert!(orphan.is_none());
    let other = lock_file(&session.dir.join(LOCK)).unwrap();
    assert!(other.try_lock().is_err(), "held by the session");
    assert!(session.finish());
    assert!(sessions(&root.0).is_empty(), "a clean exit deletes the folder");
}

/// A running session's folder is never offered, changed, or deleted.
#[test]
fn a_running_session_is_left_alone() {
    let root = TempDir::new("running");
    let Startup { mut session, .. } = start(&root.0).unwrap();
    session.schedule.write_now();
    session.tick(Instant::now(), Some(patch("live")), || {
        Info::new(None, None, SystemTime::now())
    });
    let dir = session.dir.clone();
    wait_for(|| {}, || dir.join(INFO).is_file());
    let before = fs::read_to_string(dir.join(PATCH)).unwrap();

    let second = start(&root.0).unwrap();
    assert!(second.orphan.is_none(), "not offered");
    assert_eq!(fs::read_to_string(dir.join(PATCH)).unwrap(), before, "not changed");
    assert!(second.session.finish());
    assert!(dir.is_dir(), "not deleted");
    assert!(session.finish());
}

/// A crashed session with a copy is offered; one with no copy is deleted
/// without a question.
#[test]
fn orphans_with_a_copy_are_offered_and_empty_ones_deleted() {
    let root = TempDir::new("orphans");
    let with_copy = crashed_session(&root.0, &patch("work"), None);
    let empty = {
        let Startup { session, .. } = start(&root.0).unwrap();
        let dir = session.dir.clone();
        drop(session);
        dir
    };
    // A new start finds both.
    let Startup { session, orphan } = start(&root.0).unwrap();
    let orphan = orphan.expect("the copy is offered");
    assert_eq!(orphan.dir, with_copy);
    assert_eq!(orphan.load().unwrap(), patch("work"));
    assert!(!empty.exists(), "an orphan with no copy is deleted");
    drop(orphan);
    assert!(session.finish());
}

/// Only the newest orphan is offered; the others stay for later starts.
#[test]
fn the_newest_orphan_is_offered_first() {
    let root = TempDir::new("newest");
    let old = Info::new(None, None, SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000));
    let new = Info::new(None, None, SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000));
    let old_dir = crashed_session(&root.0, &patch("old"), Some(&old));
    let new_dir = crashed_session(&root.0, &patch("new"), Some(&new));
    let first = start(&root.0).unwrap();
    assert_eq!(first.orphan.as_ref().unwrap().dir, new_dir);
    first.orphan.unwrap().discard().unwrap();
    assert!(first.session.finish());
    let second = start(&root.0).unwrap();
    assert_eq!(second.orphan.as_ref().unwrap().dir, old_dir, "the older one waited");
    drop(second.orphan);
    assert!(second.session.finish());
}

/// Instances starting at the same moment never delete or offer each
/// other's folders.
#[test]
fn simultaneous_starts_leave_each_other_alone() {
    let root = TempDir::new("race");
    for _ in 0..50 {
        let starts: Vec<_> = (0..4)
            .map(|_| {
                let root = root.0.clone();
                std::thread::spawn(move || start(&root).unwrap())
            })
            .collect();
        let startups: Vec<Startup> = starts.into_iter().map(|t| t.join().unwrap()).collect();
        for s in &startups {
            assert!(s.orphan.is_none(), "a live session was offered");
            assert!(s.session.dir.join(LOCK).is_file(), "a live session folder was deleted");
        }
        for s in startups {
            assert!(s.session.finish());
        }
    }
    assert!(sessions(&root.0).is_empty());
}

/// A recovery folder that cannot be made turns autosave off with a reason.
#[test]
fn an_unusable_folder_is_an_error() {
    let root = TempDir::new("blocked");
    let file = root.0.join("not-a-folder");
    fs::write(&file, "x").unwrap();
    let error = start(&file).err().expect("a file is not a folder");
    assert!(error.contains("cannot"), "{error}");
}

// --- The copy and its file ---

/// `info.ron` round-trips, and times keep their full precision.
#[test]
fn info_round_trips() {
    let root = TempDir::new("info");
    let file = root.0.join("drone.ron");
    fs::write(&file, "x").unwrap();
    let info = Info::new(Some(&file), Stamp::of(&file), SystemTime::now());
    let text = ron::ser::to_string_pretty(&info, ron::ser::PrettyConfig::new()).unwrap();
    let back: Info = ron::from_str(&text).unwrap();
    assert_eq!(back, info);
    assert_eq!(back.stamp(), Stamp::of(&file));
    assert!(text.contains("version: 1"), "{text}");
}

fn orphan_for(root: &Path, file: Option<&Path>) -> Orphan {
    let info = Info::new(file, file.and_then(Stamp::of), SystemTime::now());
    crashed_session(root, &patch("copy"), Some(&info));
    start(root).unwrap().orphan.unwrap()
}

#[test]
fn an_unchanged_file_is_kept() {
    let root = TempDir::new("unchanged");
    let file = root.0.join("drone.ron");
    fs::write(&file, "saved").unwrap();
    let orphan = orphan_for(&root.0, Some(&file));
    assert_eq!(orphan.file_state(), FileState::Unchanged(file));
    let (title, text) = orphan.question();
    assert_eq!(title, "Restore unsaved changes to \"drone.ron\"?");
    assert!(text.starts_with("LoomDSP did not close properly. A copy of your unsaved changes from "));
    assert!(!text.contains("changed"), "{text}");
}

fn set_modified(file: &Path, time: SystemTime) {
    File::options()
        .write(true)
        .open(file)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

#[test]
fn a_later_time_means_changed() {
    let root = TempDir::new("later");
    let file = root.0.join("drone.ron");
    fs::write(&file, "saved").unwrap();
    let orphan = orphan_for(&root.0, Some(&file));
    set_modified(&file, SystemTime::now() + Duration::from_secs(60));
    assert_eq!(orphan.file_state(), FileState::Changed(file));
    assert!(
        orphan
            .question()
            .1
            .contains("\"drone.ron\" has changed since this copy was taken. Restoring will not overwrite it.")
    );
}

#[test]
fn an_earlier_time_means_changed() {
    let root = TempDir::new("earlier");
    let file = root.0.join("drone.ron");
    fs::write(&file, "saved").unwrap();
    let orphan = orphan_for(&root.0, Some(&file));
    set_modified(&file, SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000));
    assert_eq!(orphan.file_state(), FileState::Changed(file));
}

#[test]
fn a_new_size_at_the_same_time_means_changed() {
    let root = TempDir::new("size");
    let file = root.0.join("drone.ron");
    fs::write(&file, "saved").unwrap();
    let before = Stamp::of(&file).unwrap();
    let orphan = orphan_for(&root.0, Some(&file));
    fs::write(&file, "saved, and longer").unwrap();
    set_modified(&file, before.modified);
    assert_eq!(orphan.file_state(), FileState::Changed(file));
}

#[test]
fn a_missing_file_is_named() {
    let root = TempDir::new("missing");
    let file = root.0.join("drone.ron");
    fs::write(&file, "saved").unwrap();
    let orphan = orphan_for(&root.0, Some(&file));
    fs::remove_file(&file).unwrap();
    assert_eq!(orphan.file_state(), FileState::Missing(file));
    assert!(orphan.question().1.contains("\"drone.ron\" no longer exists."));
}

#[test]
fn a_never_saved_patch_is_untitled() {
    let root = TempDir::new("untitled");
    let orphan = orphan_for(&root.0, None);
    assert_eq!(orphan.file_state(), FileState::NeverSaved);
    assert_eq!(orphan.name(), "Untitled");
}

/// A copy without `info.ron`, or with one that does not parse, is a patch
/// never saved: it is never paired with a file.
#[test]
fn a_copy_without_good_info_has_no_file() {
    let root = TempDir::new("noinfo");
    let file = root.0.join("drone.ron");
    fs::write(&file, "saved").unwrap();
    for broken in [None, Some("not ron")] {
        let info = Info::new(Some(&file), Stamp::of(&file), SystemTime::now());
        let dir = crashed_session(&root.0, &patch("copy"), Some(&info));
        match broken {
            None => fs::remove_file(dir.join(INFO)).unwrap(),
            Some(text) => fs::write(dir.join(INFO), text).unwrap(),
        }
        let Startup { session, orphan } = start(&root.0).unwrap();
        let orphan = orphan.unwrap();
        assert_eq!(orphan.file_state(), FileState::NeverSaved);
        orphan.discard().unwrap();
        assert!(session.finish());
    }
}

// --- The session and its writer ---

fn session_with(root: &Path, perform: Perform) -> Session {
    start_with(root, perform).unwrap().session
}

/// A failed write is reported once, and its recovery once.
#[test]
fn failures_are_reported_once() {
    let root = TempDir::new("fail");
    let failing = Arc::new(AtomicBool::new(true));
    let fail = failing.clone();
    let mut session = session_with(
        &root.0,
        Box::new(move |dir, job| {
            if fail.load(Ordering::SeqCst) {
                Err("disk full".to_string())
            } else {
                perform(dir, job)
            }
        }),
    );
    let info = || Info::new(None, None, SystemTime::now());
    let t0 = Instant::now();
    let mut lines = Vec::new();
    let mut now = t0;
    // 25 s of the same unsaved patch: one attempt, then retries every 10 s.
    while now < t0 + Duration::from_secs(25) {
        lines.extend(session.tick(now, Some(patch("a")), info));
        std::thread::sleep(Duration::from_millis(2));
        now += TICK;
    }
    let until = Instant::now() + Duration::from_secs(5);
    while lines.is_empty() && Instant::now() < until {
        lines.extend(session.tick(now, Some(patch("a")), info));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        lines,
        ["Autosave failed: disk full. Your changes are not protected against a crash. Save to keep them."]
    );
    // The disk has room again: the next retry writes, with no new change.
    failing.store(false, Ordering::SeqCst);
    let mut later = Vec::new();
    let mut t = now;
    let until = Instant::now() + Duration::from_secs(5);
    while later.is_empty() && Instant::now() < until {
        t += TICK;
        later.extend(session.tick(t, Some(patch("a")), info));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(later, ["Autosave works again."]);
    assert!(t - now <= RETRY + TICK * 2, "written within 10 s: {:?}", t - now);
    assert!(session.dir.join(PATCH).is_file());
    assert!(session.finish());
}

/// A write that never finishes does not hold up the exit, and the folder
/// is left for the next start.
#[test]
fn exit_does_not_wait_for_a_stuck_write() {
    let root = TempDir::new("stuck");
    let started = Arc::new(AtomicBool::new(false));
    let flag = started.clone();
    let mut session = session_with(
        &root.0,
        Box::new(move |_, _| {
            flag.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_secs(30));
            Ok(())
        }),
    );
    session.schedule.write_now();
    session.tick(Instant::now(), Some(patch("a")), || {
        Info::new(None, None, SystemTime::now())
    });
    wait_for(|| {}, || started.load(Ordering::SeqCst));
    let dir = session.dir.clone();
    let t = Instant::now();
    assert!(
        !session.finish_within(Duration::from_millis(200)),
        "the write was still running"
    );
    assert!(t.elapsed() < Duration::from_secs(2));
    assert!(dir.is_dir(), "left for the next start");
}

/// The folder is the one in the spec's table, for this OS, read from the
/// same environment variables the app reads.
#[test]
fn default_root_matches_the_spec() {
    let root = default_root().unwrap();
    let home = PathBuf::from(
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .unwrap(),
    );
    let expected = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/LoomDSP/recovery")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
            .join("LoomDSP")
            .join("recovery")
    } else {
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/state"));
        state.join("loomdsp/recovery")
    };
    assert_eq!(root, expected);
}

/// Times show in local time, as "20 October, 14:31".
#[test]
fn local_time_reads_like_a_date() {
    let shown = local_time(SystemTime::now());
    let (day, rest) = shown.split_once(' ').unwrap();
    assert!(day.parse::<u8>().is_ok(), "{shown}");
    assert!(rest.contains(", ") && rest.contains(':'), "{shown}");
}
