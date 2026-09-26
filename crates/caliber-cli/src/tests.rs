use super::{
    Dependency, LockFile, SyncState, pin_project, status_project, sync_project, update_project,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let ticks = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "caliber-cli-test-{}-{ticks}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _temp: TempDir,
    project: PathBuf,
    remote: PathBuf,
    source: PathBuf,
    lock: LockFile,
    first: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new();
        let remote = temp.path().join("remote.git");
        let source = temp.path().join("source");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        git(None, &["init", "--bare", remote.to_str().unwrap()]);
        fs::create_dir(&source).unwrap();
        git(Some(&source), &["init", "--initial-branch=main"]);
        git(
            Some(&source),
            &["config", "user.email", "test@example.invalid"],
        );
        git(Some(&source), &["config", "user.name", "Caliber tests"]);
        fs::write(source.join("file.txt"), "first\n").unwrap();
        git(Some(&source), &["add", "file.txt"]);
        git(Some(&source), &["commit", "-m", "first"]);
        git(
            Some(&source),
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(Some(&source), &["push", "--quiet", "-u", "origin", "main"]);
        let first = git(Some(&source), &["rev-parse", "HEAD"]);
        let lock = LockFile {
            schema: 1,
            dependencies: BTreeMap::from([(
                "sample".into(),
                Dependency {
                    repository: remote.to_string_lossy().into_owned(),
                    tracked_ref: Some("main".into()),
                    revision: first.clone(),
                },
            )]),
        };
        write_lock(&project, &lock);
        write_config(&project, &["git", "-C", "{candidate}", "rev-parse", "HEAD"]);
        Self {
            _temp: temp,
            project,
            remote,
            source,
            lock,
            first,
        }
    }

    fn commit_second(&mut self) -> String {
        fs::write(self.source.join("file.txt"), "second\n").unwrap();
        git(Some(&self.source), &["add", "file.txt"]);
        git(Some(&self.source), &["commit", "-m", "second"]);
        git(Some(&self.source), &["push", "--quiet", "origin", "main"]);
        git(Some(&self.source), &["rev-parse", "HEAD"])
    }
}

fn git(cwd: Option<&Path>, args: &[&str]) -> String {
    let mut command = Command::new("git");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command
        .args(args)
        .output()
        .expect("Git must be installed for CLI tests");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn write_lock(project: &Path, lock: &LockFile) {
    fs::write(
        project.join("dependencies.lock.json"),
        serde_json::to_vec_pretty(lock).unwrap(),
    )
    .unwrap();
}

fn write_config(project: &Path, command: &[&str]) {
    let value = serde_json::json!({"schema":1,"validation":{"sample":{"command":command}}});
    fs::write(
        project.join("caliber.config.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
}

#[test]
fn sync_materializes_missing_dependency_and_status_reports_it() {
    let fixture = Fixture::new();
    let paths = sync_project(&fixture.project, &fixture.lock, &BTreeMap::new(), false).unwrap();
    assert_eq!(
        git(Some(&paths["sample"]), &["rev-parse", "HEAD"]),
        fixture.first
    );
    assert_eq!(
        status_project(&fixture.project).unwrap()[0].state,
        SyncState::Synchronized
    );
    sync_project(&fixture.project, &fixture.lock, &BTreeMap::new(), false).unwrap();
}

#[test]
fn sync_is_offline_when_the_locked_object_is_already_local() {
    let mut fixture = Fixture::new();
    let second = fixture.commit_second();
    sync_project(&fixture.project, &fixture.lock, &BTreeMap::new(), false).unwrap();
    let checkout = fixture.project.join(".deps/sample");
    git(
        Some(&checkout),
        &["checkout", "--quiet", "--detach", &second],
    );
    fixture
        .lock
        .dependencies
        .get_mut("sample")
        .unwrap()
        .revision = fixture.first.clone();
    fs::rename(
        &fixture.remote,
        fixture.remote.with_extension("unavailable"),
    )
    .unwrap();
    sync_project(&fixture.project, &fixture.lock, &BTreeMap::new(), false).unwrap();
    assert_eq!(git(Some(&checkout), &["rev-parse", "HEAD"]), fixture.first);
    assert_eq!(
        status_project(&fixture.project).unwrap()[0].state,
        SyncState::Synchronized
    );
}

#[test]
fn dirty_managed_checkout_is_rejected_untouched() {
    let mut fixture = Fixture::new();
    let second = fixture.commit_second();
    sync_project(&fixture.project, &fixture.lock, &BTreeMap::new(), false).unwrap();
    let checkout = fixture.project.join(".deps/sample");
    git(
        Some(&checkout),
        &["checkout", "--quiet", "--detach", &second],
    );
    fs::write(checkout.join("user-work.txt"), "keep\n").unwrap();
    let error = sync_project(&fixture.project, &fixture.lock, &BTreeMap::new(), false).unwrap_err();
    assert!(error.to_string().contains("dirty managed checkout"));
    assert!(checkout.join("user-work.txt").exists());
    assert_eq!(git(Some(&checkout), &["rev-parse", "HEAD"]), second);
}

#[test]
fn developer_override_is_read_only_and_requires_explicit_dirty_opt_in() {
    let fixture = Fixture::new();
    let override_path = fixture.source.clone();
    fs::write(override_path.join("local.txt"), "keep\n").unwrap();
    let overrides = BTreeMap::from([("sample".into(), override_path.clone())]);
    assert!(sync_project(&fixture.project, &fixture.lock, &overrides, false).is_err());
    let head = git(Some(&override_path), &["rev-parse", "HEAD"]);
    assert_eq!(
        sync_project(&fixture.project, &fixture.lock, &overrides, true).unwrap()["sample"],
        override_path
    );
    assert_eq!(git(Some(&fixture.source), &["rev-parse", "HEAD"]), head);
    assert!(fixture.source.join("local.txt").exists());
}

#[test]
fn status_distinguishes_missing_mismatched_and_dirty_without_fetching() {
    let mut fixture = Fixture::new();
    assert_eq!(
        status_project(&fixture.project).unwrap()[0].state,
        SyncState::Missing
    );
    let second = fixture.commit_second();
    sync_project(&fixture.project, &fixture.lock, &BTreeMap::new(), false).unwrap();
    let checkout = fixture.project.join(".deps/sample");
    git(
        Some(&checkout),
        &["checkout", "--quiet", "--detach", &second],
    );
    assert_eq!(
        status_project(&fixture.project).unwrap()[0].state,
        SyncState::Mismatched
    );
    fs::write(checkout.join("dirty.txt"), "edit\n").unwrap();
    let status = status_project(&fixture.project).unwrap();
    assert_eq!(status[0].state, SyncState::Mismatched);
    assert_eq!(status[0].dirty, Some(true));
}

#[test]
fn update_changes_lock_only_after_validation() {
    let mut fixture = Fixture::new();
    let second = fixture.commit_second();
    assert_eq!(update_project(&fixture.project, "sample").unwrap(), second);
    let before = fs::read(fixture.project.join("dependencies.lock.json")).unwrap();
    write_config(
        &fixture.project,
        &["git", "-C", "{candidate}", "show", "does-not-exist"],
    );
    assert!(update_project(&fixture.project, "sample").is_err());
    assert_eq!(
        fs::read(fixture.project.join("dependencies.lock.json")).unwrap(),
        before
    );
}

#[test]
fn pin_requires_clean_reproducible_source_and_does_not_mutate_it() {
    let mut fixture = Fixture::new();
    let second = fixture.commit_second();
    assert_eq!(
        pin_project(&fixture.project, "sample", &fixture.source).unwrap(),
        second
    );
    assert_eq!(git(Some(&fixture.source), &["rev-parse", "HEAD"]), second);
    let before = fs::read(fixture.project.join("dependencies.lock.json")).unwrap();
    fs::write(fixture.source.join("local.txt"), "uncommitted\n").unwrap();
    let error = pin_project(&fixture.project, "sample", &fixture.source).unwrap_err();
    assert!(error.to_string().contains("dirty pin source"));
    assert_eq!(
        fs::read(fixture.project.join("dependencies.lock.json")).unwrap(),
        before
    );
    assert!(fixture.source.join("local.txt").exists());
}

#[test]
fn pin_validation_failure_is_transactional() {
    let mut fixture = Fixture::new();
    fixture.commit_second();
    let before = fs::read(fixture.project.join("dependencies.lock.json")).unwrap();
    write_config(
        &fixture.project,
        &["git", "-C", "{candidate}", "show", "does-not-exist"],
    );
    assert!(pin_project(&fixture.project, "sample", &fixture.source).is_err());
    assert_eq!(
        fs::read(fixture.project.join("dependencies.lock.json")).unwrap(),
        before
    );
    assert!(
        git(
            Some(&fixture.source),
            &["status", "--porcelain", "--untracked-files=all"]
        )
        .is_empty()
    );
}

#[test]
fn missing_hook_and_malformed_lock_leave_previous_lock_bytes_untouched() {
    let fixture = Fixture::new();
    let lock_path = fixture.project.join("dependencies.lock.json");
    let before = fs::read(&lock_path).unwrap();
    fs::remove_file(fixture.project.join("caliber.config.json")).unwrap();
    assert!(update_project(&fixture.project, "sample").is_err());
    assert_eq!(fs::read(&lock_path).unwrap(), before);
    fs::write(&lock_path, b"not-json\n").unwrap();
    assert!(
        status_project(&fixture.project)
            .unwrap_err()
            .to_string()
            .contains("malformed lock")
    );
}
