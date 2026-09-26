use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

mod diagnostics;

const LOCK_FILE: &str = "dependencies.lock.json";
const CONFIG_FILE: &str = "caliber.config.json";
const LOCK_SCHEMA: u32 = 1;

#[derive(Debug)]
pub struct Error(String);

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub repository: String,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub tracked_ref: Option<String>,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LockFile {
    pub schema: u32,
    #[serde(flatten)]
    pub dependencies: BTreeMap<String, Dependency>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncState {
    Synchronized,
    Missing,
    Mismatched,
}

#[derive(Clone, Debug)]
pub struct StatusEntry {
    pub name: String,
    pub revision: String,
    pub path: PathBuf,
    pub head: Option<String>,
    pub dirty: Option<bool>,
    pub state: SyncState,
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectConfig {
    schema: u32,
    #[serde(default)]
    validation: BTreeMap<String, ValidationHook>,
    #[serde(default)]
    diagnostics: Option<diagnostics::DiagnosticsConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidationHook {
    #[serde(default)]
    windows: Option<Vec<String>>,
    #[serde(default)]
    unix: Option<Vec<String>>,
    #[serde(default)]
    command: Option<Vec<String>>,
}

struct TemporaryDirectory(PathBuf);

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| {
            arg.into_string()
                .map_err(|_| Error::new("command arguments must be valid UTF-8"))
        })
        .collect::<Result<_>>()?;
    let Some(command) = args.first().map(String::as_str) else {
        return Err(Error::new(usage()));
    };
    if command == "--help" || command == "help" {
        println!("{}", usage());
        return Ok(());
    }
    let root = option_path(&args[1..], "--project-root")?.unwrap_or(
        env::current_dir()
            .map_err(|error| Error::new(format!("read current directory: {error}")))?,
    );
    let root = canonical_project_root(&root)?;

    match command {
        "doctor" => {
            let library = option_path(&args[1..], "--library")?;
            diagnostics::doctor(&root, library.as_deref())
        }
        "check" => {
            let library = option_path(&args[1..], "--library")?;
            diagnostics::check(&root, library.as_deref())
        }
        "sync" => {
            let (overrides, allow_dirty) = parse_overrides(&args[1..])?;
            let entries = read_lock(&root)?;
            let paths = sync_project(&root, &entries, &overrides, allow_dirty)?;
            for (name, path) in paths {
                println!("{name}: {}", path.display());
            }
            Ok(())
        }
        "status" => {
            let entries = status_project(&root)?;
            print_status(&entries);
            Ok(())
        }
        "update" => {
            let name = positional(&args[1..], 0, "dependency name")?;
            update_project(&root, name)?;
            Ok(())
        }
        "pin" => {
            let name = positional(&args[1..], 0, "dependency name")?;
            let checkout = positional(&args[1..], 1, "local checkout")?;
            pin_project(&root, name, Path::new(checkout))?;
            Ok(())
        }
        other => Err(Error::new(format!(
            "unknown command {other:?}\n{}",
            usage()
        ))),
    }
}

fn usage() -> &'static str {
    "Usage:\n  caliber sync [--project-root DIR] [--override NAME=PATH ...] [--allow-dirty-overrides]\n  caliber status [--project-root DIR]\n  caliber doctor [--project-root DIR] [--library PATH]\n  caliber check [--project-root DIR] [--library PATH]\n  caliber update NAME [--project-root DIR]\n  caliber pin NAME LOCAL-CHECKOUT [--project-root DIR]"
}

fn positional<'a>(args: &'a [String], index: usize, label: &str) -> Result<&'a str> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project-root" | "--override" => i += 2,
            "--allow-dirty-overrides" => i += 1,
            value if value.starts_with("--") => {
                return Err(Error::new(format!("unknown option {value}")));
            }
            value => {
                values.push(value);
                i += 1;
            }
        }
    }
    values
        .get(index)
        .copied()
        .ok_or_else(|| Error::new(format!("missing {label}\n{}", usage())))
}

fn option_path(args: &[String], name: &str) -> Result<Option<PathBuf>> {
    let mut found = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            let value = args
                .get(i + 1)
                .ok_or_else(|| Error::new(format!("{name} requires a path")))?;
            found = Some(PathBuf::from(value));
            i += 1;
        }
        i += 1;
    }
    Ok(found)
}

fn parse_overrides(args: &[String]) -> Result<(BTreeMap<String, PathBuf>, bool)> {
    let mut overrides = BTreeMap::new();
    let mut allow_dirty = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--override" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| Error::new("--override requires NAME=PATH"))?;
                let (name, path) = value
                    .split_once('=')
                    .ok_or_else(|| Error::new("--override requires NAME=PATH"))?;
                validate_name(name)?;
                if path.is_empty() {
                    return Err(Error::new("--override path cannot be empty"));
                }
                if overrides
                    .insert(name.to_owned(), PathBuf::from(path))
                    .is_some()
                {
                    return Err(Error::new(format!("duplicate override for {name}")));
                }
                i += 1;
            }
            "--allow-dirty-overrides" => allow_dirty = true,
            _ => {}
        }
        i += 1;
    }
    Ok((overrides, allow_dirty))
}

fn canonical_project_root(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|error| {
        Error::new(format!(
            "project root does not exist: {} ({error})",
            path.display()
        ))
    })
}

fn read_lock(root: &Path) -> Result<LockFile> {
    let path = root.join(LOCK_FILE);
    let bytes = fs::read(&path).map_err(|error| {
        Error::new(format!(
            "malformed lock: cannot read {} ({error})",
            path.display()
        ))
    })?;
    let lock: LockFile = serde_json::from_slice(&bytes)
        .map_err(|error| Error::new(format!("malformed lock {}: {error}", path.display())))?;
    validate_lock(&lock)?;
    Ok(lock)
}

fn validate_lock(lock: &LockFile) -> Result<()> {
    if lock.schema != LOCK_SCHEMA {
        return Err(Error::new(format!(
            "malformed lock: schema {} is unsupported (expected {LOCK_SCHEMA})",
            lock.schema
        )));
    }
    if lock.dependencies.is_empty() {
        return Err(Error::new("malformed lock: no dependencies are defined"));
    }
    for (name, dependency) in &lock.dependencies {
        validate_name(name).map_err(|error| Error::new(format!("malformed lock: {error}")))?;
        if dependency.repository.trim().is_empty() {
            return Err(Error::new(format!(
                "malformed lock: {name} has no Git repository"
            )));
        }
        validate_revision(&dependency.revision)
            .map_err(|error| Error::new(format!("malformed lock: {name} {error}")))?;
        if dependency
            .tracked_ref
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(Error::new(format!(
                "malformed lock: {name} has an empty tracked ref"
            )));
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(Error::new(format!(
            "invalid dependency name {name:?}; use letters, digits, '-' or '_'"
        )));
    }
    Ok(())
}

fn validate_revision(revision: &str) -> Result<()> {
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error::new(format!(
            "revision must be a full 40- or 64-character Git object ID: {revision:?}"
        )));
    }
    Ok(())
}

pub fn sync_project(
    root: &Path,
    lock: &LockFile,
    overrides: &BTreeMap<String, PathBuf>,
    allow_dirty_overrides: bool,
) -> Result<BTreeMap<String, PathBuf>> {
    validate_lock(lock)?;
    for name in overrides.keys() {
        if !lock.dependencies.contains_key(name) {
            return Err(Error::new(format!(
                "override names unknown dependency {name:?}"
            )));
        }
    }
    if allow_dirty_overrides && overrides.is_empty() {
        return Err(Error::new(
            "--allow-dirty-overrides requires at least one --override",
        ));
    }

    let root = canonical_project_root(root)?;
    let managed = managed_root(&root, true)?;
    let mut resolved = BTreeMap::new();
    for (name, dependency) in &lock.dependencies {
        if let Some(path) = overrides.get(name) {
            let actual_path = validate_override(name, dependency, path, allow_dirty_overrides)?;
            resolved.insert(name.clone(), actual_path);
        } else {
            let path = managed.join(name);
            sync_managed(&root, &managed, name, dependency, &path)?;
            resolved.insert(
                name.clone(),
                fs::canonicalize(path).map_err(|error| {
                    Error::new(format!(
                        "dependency {name} checkout missing after sync: {error}"
                    ))
                })?,
            );
        }
    }
    Ok(resolved)
}

fn managed_root(root: &Path, create: bool) -> Result<PathBuf> {
    let path = root.join(".deps");
    if create {
        fs::create_dir_all(&path).map_err(|error| {
            Error::new(format!(
                "create managed dependency directory {}: {error}",
                path.display()
            ))
        })?;
    }
    if !path.exists() {
        return Ok(path);
    }
    let actual = fs::canonicalize(&path).map_err(|error| {
        Error::new(format!(
            "inspect managed dependency directory {}: {error}",
            path.display()
        ))
    })?;
    if !actual.starts_with(root) {
        return Err(Error::new(format!(
            "managed dependency directory resolves outside the project: {}",
            actual.display()
        )));
    }
    Ok(actual)
}

fn validate_managed_path(managed: &Path, path: &Path, name: &str) -> Result<()> {
    if path.exists() {
        let actual = fs::canonicalize(path).map_err(|error| {
            Error::new(format!(
                "inspect managed {name} checkout {}: {error}",
                path.display()
            ))
        })?;
        if !actual.starts_with(managed) {
            return Err(Error::new(format!(
                "managed {name} checkout resolves outside .deps: {}",
                actual.display()
            )));
        }
        if actual.parent() != Some(managed) || actual.file_name() != path.file_name() {
            return Err(Error::new(format!(
                "managed {name} path must be a real direct checkout under .deps; refusing to follow {} to {}",
                path.display(),
                actual.display()
            )));
        }
    }
    Ok(())
}

fn validate_override(
    name: &str,
    dependency: &Dependency,
    path: &Path,
    allow_dirty: bool,
) -> Result<PathBuf> {
    let repo = git_output(
        None,
        &[
            "-C",
            path.to_string_lossy().as_ref(),
            "rev-parse",
            "--show-toplevel",
        ],
    )
    .map_err(|error| {
        if error.to_string().contains("Git is missing") {
            return error;
        }
        Error::new(format!(
            "dependency missing: {name} override is not a Git checkout: {}",
            path.display()
        ))
    })?;
    let root = PathBuf::from(repo.trim());
    let origin = remote_origin(&root)?;
    let repository_matches = same_repository(&origin, &dependency.repository);
    let head = git_output(Some(&root), &["rev-parse", "HEAD"])
        .map_err(|error| Error::new(format!("read {name} override HEAD: {error}")))?;
    let head = head.trim().to_owned();
    let dirty = git_output(
        Some(&root),
        &["status", "--porcelain", "--untracked-files=all"],
    )
    .map_err(|error| Error::new(format!("inspect {name} override: {error}")))?;
    if allow_dirty {
        eprintln!(
            "WARNING: using developer-owned {name} override at {} ({}){}{}; it will not be modified",
            root.display(),
            &head[..head.len().min(12)],
            if dirty.trim().is_empty() {
                ""
            } else {
                ", dirty"
            },
            if repository_matches {
                ""
            } else {
                ", repository differs from lock"
            }
        );
        return Ok(root);
    }
    if !repository_matches {
        return Err(Error::new(format!(
            "mismatched override for {name}: repository expected {}, found {origin}; use --allow-dirty-overrides to opt into this developer-owned checkout",
            dependency.repository
        )));
    }
    if head != dependency.revision {
        return Err(Error::new(format!(
            "mismatched override for {name}: locked {}, found {}; use --allow-dirty-overrides to opt into this developer-owned checkout",
            dependency.revision, head
        )));
    }
    if !dirty.trim().is_empty() {
        return Err(Error::new(format!(
            "dirty pin source: {name} override has uncommitted or untracked changes; use --allow-dirty-overrides to explicitly use it"
        )));
    }
    Ok(root)
}

fn sync_managed(
    root: &Path,
    managed: &Path,
    name: &str,
    dependency: &Dependency,
    path: &Path,
) -> Result<()> {
    validate_managed_path(managed, path, name)?;
    if !path.exists() {
        let stage = temporary_directory(managed, &format!("{name}-clone"))?;
        initialize_remote(&stage.0, &dependency.repository)?;
        materialize_locked_revision(&stage.0, name, dependency)?;
        let origin = remote_origin(&stage.0)?;
        if !same_repository(&origin, &dependency.repository) {
            return Err(Error::new(format!(
                "managed {name} origin mismatch: expected {}, found {origin}",
                dependency.repository
            )));
        }
        if path.exists() {
            return Err(Error::new(format!(
                "managed dependency path appeared during sync: {}",
                path.display()
            )));
        }
        fs::rename(&stage.0, path).map_err(|error| {
            Error::new(format!(
                "install managed {name} checkout at {}: {error}",
                path.display()
            ))
        })?;
        return Ok(());
    }
    if !path.is_dir() {
        return Err(Error::new(format!(
            "dependency missing: managed path exists but is not a checkout directory: {}",
            path.display()
        )));
    }
    let checkout = git_output(Some(path), &["rev-parse", "--show-toplevel"]).map_err(|_| {
        Error::new(format!(
            "dependency missing: managed path is not a Git checkout: {}",
            path.display()
        ))
    })?;
    let checkout = fs::canonicalize(checkout.trim())
        .map_err(|error| Error::new(format!("resolve managed {name} root: {error}")))?;
    if checkout != fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
        || !checkout.starts_with(managed)
        || !checkout.starts_with(root)
    {
        return Err(Error::new(format!(
            "managed {name} checkout escapes the project-owned .deps directory: {}",
            checkout.display()
        )));
    }
    let origin = remote_origin(&checkout)?;
    if !same_repository(&origin, &dependency.repository) {
        return Err(Error::new(format!(
            "managed {name} origin mismatch: expected {}, found {origin}; the checkout was left untouched",
            dependency.repository
        )));
    }
    let dirty = git_output(
        Some(&checkout),
        &["status", "--porcelain", "--untracked-files=all"],
    )?;
    if !dirty.trim().is_empty() {
        return Err(Error::new(format!(
            "dirty managed checkout: {name} has local edits; it was left untouched. Commit/stash them or move {} aside yourself",
            path.display()
        )));
    }
    let head = git_output(Some(&checkout), &["rev-parse", "HEAD"])?;
    if head.trim() == dependency.revision {
        return Ok(());
    }
    if !has_commit(&checkout, &dependency.revision) {
        fetch_locked_revision(&checkout, name, dependency)?;
    }
    if !has_commit(&checkout, &dependency.revision) {
        return Err(Error::new(format!(
            "commit missing: {name} repository does not contain locked commit {} after fetching",
            dependency.revision
        )));
    }
    git_checked(
        Some(&checkout),
        &["checkout", "--quiet", "--detach", &dependency.revision],
    )
    .map_err(|error| {
        Error::new(format!(
            "checkout locked {name} revision {}: {error}",
            dependency.revision
        ))
    })?;
    Ok(())
}

fn initialize_remote(path: &Path, repository: &str) -> Result<()> {
    git_checked(Some(path), &["init", "--quiet"])
        .map_err(|error| Error::new(format!("initialize managed Git checkout: {error}")))?;
    git_checked(Some(path), &["remote", "add", "origin", repository])
        .map_err(|error| Error::new(format!("configure Git origin: {error}")))?;
    Ok(())
}

fn materialize_locked_revision(path: &Path, name: &str, dependency: &Dependency) -> Result<()> {
    if !has_commit(path, &dependency.revision) {
        fetch_locked_revision(path, name, dependency)?;
    }
    if !has_commit(path, &dependency.revision) {
        return Err(Error::new(format!(
            "commit missing: fetched {} but locked revision {} is unavailable",
            name, dependency.revision
        )));
    }
    git_checked(
        Some(path),
        &["checkout", "--quiet", "--detach", &dependency.revision],
    )
    .map_err(|error| Error::new(format!("checkout locked {name} revision: {error}")))
}

fn fetch_locked_revision(path: &Path, name: &str, dependency: &Dependency) -> Result<()> {
    let mut failures = Vec::new();
    if let Some(tracked_ref) = &dependency.tracked_ref {
        if let Err(error) = git_checked(
            Some(path),
            &["fetch", "--quiet", "--no-tags", "origin", tracked_ref],
        ) {
            failures.push(error);
        }
        if has_commit(path, &dependency.revision) {
            return Ok(());
        }
    }
    if let Err(error) = git_checked(
        Some(path),
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            "origin",
            &dependency.revision,
        ],
    ) {
        failures.push(error);
    }
    if has_commit(path, &dependency.revision) {
        return Ok(());
    }
    if !failures.is_empty() {
        let remote_available = git_status_opt(Some(path), &["ls-remote", "origin"])
            .is_ok_and(|output| output.status.success());
        if remote_available {
            return Err(Error::new(format!(
                "commit missing: {name} repository is reachable but did not provide locked revision {}",
                dependency.revision
            )));
        }
        let failures = failures
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(Error::new(format!(
            "network unavailable while fetching {name}: {failures}"
        )));
    }
    Err(Error::new(format!(
        "commit missing: {name} repository did not provide locked revision {}",
        dependency.revision
    )))
}

fn has_commit(path: &Path, revision: &str) -> bool {
    git_status_opt(
        Some(path),
        &["cat-file", "-e", &format!("{revision}^{{commit}}")],
    )
    .is_ok_and(|output| output.status.success())
}

pub fn status_project(root: &Path) -> Result<Vec<StatusEntry>> {
    let root = canonical_project_root(root)?;
    let lock = read_lock(&root)?;
    let managed = managed_root(&root, false)?;
    let mut result = Vec::new();
    for (name, dependency) in lock.dependencies {
        let path = managed.join(&name);
        if !path.exists() {
            result.push(StatusEntry {
                name,
                revision: dependency.revision,
                path,
                head: None,
                dirty: None,
                state: SyncState::Missing,
                detail: Some("checkout is not materialized".into()),
            });
            continue;
        }
        let root_result = git_output(Some(&path), &["rev-parse", "--show-toplevel"]);
        let Ok(checkout) = root_result else {
            if root_result
                .as_ref()
                .is_err_and(|error| error.to_string().contains("Git is missing"))
            {
                return Err(Error::new(
                    "Git is missing; install Git and ensure it is available on PATH",
                ));
            }
            result.push(StatusEntry {
                name,
                revision: dependency.revision,
                path,
                head: None,
                dirty: None,
                state: SyncState::Mismatched,
                detail: Some("managed path is not a Git checkout".into()),
            });
            continue;
        };
        let checkout = fs::canonicalize(checkout.trim())
            .map_err(|error| Error::new(format!("resolve managed {name} checkout: {error}")))?;
        if !checkout.starts_with(&managed) || checkout.parent() != Some(managed.as_path()) {
            result.push(StatusEntry {
                name,
                revision: dependency.revision,
                path,
                head: None,
                dirty: None,
                state: SyncState::Mismatched,
                detail: Some("checkout resolves outside its managed dependency path".into()),
            });
            continue;
        }
        let head = git_output(Some(&checkout), &["rev-parse", "HEAD"])
            .ok()
            .map(|value| value.trim().to_owned());
        let dirty_text = git_output(
            Some(&checkout),
            &["status", "--porcelain", "--untracked-files=all"],
        )
        .ok();
        let dirty = dirty_text.as_deref().map(|value| !value.trim().is_empty());
        let origin = remote_origin(&checkout).ok();
        let state = if head.as_deref() == Some(dependency.revision.as_str())
            && dirty == Some(false)
            && origin
                .as_deref()
                .is_some_and(|value| same_repository(value, &dependency.repository))
        {
            SyncState::Synchronized
        } else {
            SyncState::Mismatched
        };
        let detail = if !origin
            .as_deref()
            .is_some_and(|value| same_repository(value, &dependency.repository))
        {
            Some("repository origin differs from the lock".into())
        } else if dirty != Some(false) {
            Some("working tree is dirty or could not be inspected".into())
        } else if head.as_deref() != Some(dependency.revision.as_str()) {
            Some("HEAD differs from the locked revision".into())
        } else {
            None
        };
        result.push(StatusEntry {
            name,
            revision: dependency.revision,
            path,
            head,
            dirty,
            state,
            detail,
        });
    }
    Ok(result)
}

fn print_status(entries: &[StatusEntry]) {
    println!("Dependency status (local; no remote checks)");
    for entry in entries {
        let state = match entry.state {
            SyncState::Synchronized => "synchronized",
            SyncState::Missing => "missing",
            SyncState::Mismatched => "mismatched",
        };
        let actual = entry.head.as_deref().unwrap_or("-");
        let dirty = match entry.dirty {
            Some(true) => "dirty",
            Some(false) => "clean",
            None => "unknown",
        };
        println!(
            "{}: locked={} path={} HEAD={} {} {}{}",
            entry.name,
            entry.revision,
            entry.path.display(),
            actual,
            dirty,
            state,
            entry
                .detail
                .as_deref()
                .map(|detail| format!(" ({detail})"))
                .unwrap_or_default()
        );
    }
}

pub fn update_project(root: &Path, name: &str) -> Result<String> {
    validate_name(name)?;
    let root = canonical_project_root(root)?;
    let mut lock = read_lock(&root)?;
    let dependency = lock
        .dependencies
        .get(name)
        .ok_or_else(|| Error::new(format!("unknown dependency {name:?}")))?
        .clone();
    let tracked_ref = dependency.tracked_ref.as_deref().ok_or_else(|| {
        Error::new(format!(
            "{name} has no tracked ref in {LOCK_FILE}; set its ref before updating"
        ))
    })?;
    let stage = temporary_directory(&env::temp_dir(), &format!("caliber-update-{name}"))?;
    initialize_remote(&stage.0, &dependency.repository)?;
    git_checked(
        Some(&stage.0),
        &["fetch", "--quiet", "--no-tags", "origin", tracked_ref],
    )
    .map_err(|error| {
        Error::new(format!(
            "network unavailable while fetching {name} tracked ref {tracked_ref}: {error}"
        ))
    })?;
    let revision = git_output(Some(&stage.0), &["rev-parse", "FETCH_HEAD^{commit}"])
        .map_err(|error| Error::new(format!("resolve {name} tracked ref {tracked_ref}: {error}")))?
        .trim()
        .to_owned();
    validate_revision(&revision)?;
    git_checked(
        Some(&stage.0),
        &["checkout", "--quiet", "--detach", &revision],
    )
    .map_err(|error| Error::new(format!("materialize {name} candidate {revision}: {error}")))?;
    validate_candidate(&root, name, &revision, &stage.0)?;
    lock.dependencies
        .get_mut(name)
        .expect("dependency checked above")
        .revision = revision.clone();
    write_lock_atomic(&root, &lock)?;
    println!("updated {name} to {revision}");
    Ok(revision)
}

pub fn pin_project(root: &Path, name: &str, checkout: &Path) -> Result<String> {
    validate_name(name)?;
    let root = canonical_project_root(root)?;
    let mut lock = read_lock(&root)?;
    let dependency = lock
        .dependencies
        .get(name)
        .ok_or_else(|| Error::new(format!("unknown dependency {name:?}")))?
        .clone();
    let source = git_output(
        None,
        &[
            "-C",
            checkout.to_string_lossy().as_ref(),
            "rev-parse",
            "--show-toplevel",
        ],
    )
    .map_err(|_| {
        Error::new(format!(
            "pin source is not a Git checkout: {}",
            checkout.display()
        ))
    })?;
    let source = PathBuf::from(source.trim());
    let dirty = git_output(
        Some(&source),
        &["status", "--porcelain", "--untracked-files=all"],
    )?;
    if !dirty.trim().is_empty() {
        return Err(Error::new(format!(
            "dirty pin source: {} has uncommitted or untracked changes; the checkout was not modified",
            source.display()
        )));
    }
    let source_origin = remote_origin(&source)?;
    if !same_repository(&source_origin, &dependency.repository) {
        return Err(Error::new(format!(
            "pin source repository mismatch for {name}: expected {}, found {}; the checkout was not modified",
            dependency.repository, source_origin
        )));
    }
    let revision = git_output(Some(&source), &["rev-parse", "HEAD"])?;
    let revision = revision.trim().to_owned();
    validate_revision(&revision)?;

    let stage = temporary_directory(&env::temp_dir(), &format!("caliber-pin-{name}"))?;
    initialize_remote(&stage.0, &dependency.repository)?;
    if !has_commit(&stage.0, &revision) {
        fetch_locked_revision(
            &stage.0,
            name,
            &Dependency {
                revision: revision.clone(),
                ..dependency.clone()
            },
        )
        .map_err(|error| {
            Error::new(format!(
                "pin commit cannot be reproduced from {}: {error}",
                dependency.repository
            ))
        })?;
    }
    if !has_commit(&stage.0, &revision) {
        return Err(Error::new(format!(
            "pin commit cannot be reproduced from {}: commit missing {revision}",
            dependency.repository
        )));
    }
    git_checked(
        Some(&stage.0),
        &["checkout", "--quiet", "--detach", &revision],
    )
    .map_err(|error| Error::new(format!("materialize pin candidate {revision}: {error}")))?;
    validate_candidate(&root, name, &revision, &stage.0)?;
    lock.dependencies
        .get_mut(name)
        .expect("dependency checked above")
        .revision = revision.clone();
    write_lock_atomic(&root, &lock)?;
    println!("pinned {name} to {revision}");
    Ok(revision)
}

fn validate_candidate(root: &Path, name: &str, revision: &str, candidate: &Path) -> Result<()> {
    let path = root.join(CONFIG_FILE);
    let bytes = fs::read(&path).map_err(|error| {
        Error::new(format!(
            "validation hook required: read {} ({error})",
            path.display()
        ))
    })?;
    let config: ProjectConfig = serde_json::from_slice(&bytes).map_err(|error| {
        Error::new(format!(
            "malformed project config {}: {error}",
            path.display()
        ))
    })?;
    if config.schema != 1 {
        return Err(Error::new(format!(
            "malformed project config: unsupported schema {}",
            config.schema
        )));
    }
    let hook = config.validation.get(name).ok_or_else(|| {
        Error::new(format!(
            "validation hook required: {CONFIG_FILE} has no validation command for {name}"
        ))
    })?;
    let command = if cfg!(windows) {
        hook.windows.as_ref().or(hook.command.as_ref())
    } else {
        hook.unix.as_ref().or(hook.command.as_ref())
    }
    .ok_or_else(|| {
        Error::new(format!(
            "validation hook required: {CONFIG_FILE} has no command for this platform and {name}"
        ))
    })?;
    let (program, args) = command
        .split_first()
        .filter(|(program, _)| !program.trim().is_empty())
        .ok_or_else(|| {
            Error::new(format!(
                "malformed validation hook for {name}: command must not be empty"
            ))
        })?;
    let candidate_text = hook_path(candidate);
    let mut process = Command::new(program);
    process
        .current_dir(root)
        .args(args.iter().map(|arg| {
            arg.replace("{candidate}", &candidate_text)
                .replace("{revision}", revision)
                .replace("{dependency}", name)
        }))
        .env("CALIBER_PROJECT_ROOT", hook_path(root))
        .env("CALIBER_DEPENDENCY", name)
        .env("CALIBER_CANDIDATE_REVISION", revision)
        .env("CALIBER_CANDIDATE_ROOT", &candidate_text);
    let status = process.status().map_err(|error| {
        Error::new(format!(
            "validation command could not start for {name}: {program} ({error})"
        ))
    })?;
    if !status.success() {
        return Err(Error::new(format!(
            "validation failed for {name} candidate {revision} with {status}; the lock was not changed"
        )));
    }
    Ok(())
}

fn hook_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        if let Some(path) = path.strip_prefix(r"\\?\") {
            return path.to_owned();
        }
    }
    path.into_owned()
}

fn write_lock_atomic(root: &Path, lock: &LockFile) -> Result<()> {
    validate_lock(lock)?;
    let target = root.join(LOCK_FILE);
    let mut bytes = serde_json::to_vec_pretty(lock)
        .map_err(|error| Error::new(format!("serialize lock: {error}")))?;
    bytes.push(b'\n');
    let temporary = root.join(format!(".{LOCK_FILE}.{}.tmp", unique_suffix()));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| {
                Error::new(format!(
                    "create temporary lock {}: {error}",
                    temporary.display()
                ))
            })?;
        file.write_all(&bytes)
            .map_err(|error| Error::new(format!("write temporary lock: {error}")))?;
        file.sync_all()
            .map_err(|error| Error::new(format!("flush temporary lock: {error}")))?;
        fs::rename(&temporary, &target).map_err(|error| {
            Error::new(format!(
                "atomically replace lock {}: {error}",
                target.display()
            ))
        })?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn temporary_directory(parent: &Path, label: &str) -> Result<TemporaryDirectory> {
    fs::create_dir_all(parent).map_err(|error| {
        Error::new(format!(
            "create temporary parent {}: {error}",
            parent.display()
        ))
    })?;
    let path = parent.join(format!(".caliber-{label}-{}", unique_suffix()));
    fs::create_dir(&path).map_err(|error| {
        Error::new(format!(
            "create temporary checkout {}: {error}",
            path.display()
        ))
    })?;
    Ok(TemporaryDirectory(path))
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn remote_origin(path: &Path) -> Result<String> {
    git_output(Some(path), &["remote", "get-url", "origin"])
        .map(|value| value.trim().to_owned())
        .map_err(|error| Error::new(format!("read Git origin at {}: {error}", path.display())))
}

fn same_repository(left: &str, right: &str) -> bool {
    fn normalize(value: &str) -> String {
        let mut value = value.trim().to_owned();
        if let Some(rest) = value.strip_prefix("ssh://git@") {
            value = format!("https://{rest}");
        } else if let Some(rest) = value.strip_prefix("git@")
            && let Some((host, path)) = rest.split_once(':')
        {
            value = format!("https://{host}/{path}");
        }
        while value.ends_with('/') {
            value.pop();
        }
        if value.ends_with(".git") {
            value.truncate(value.len() - 4);
        }
        if value.contains("://") {
            value.make_ascii_lowercase();
        }
        value
    }
    normalize(left) == normalize(right)
}

fn git_output(path: Option<&Path>, args: &[&str]) -> Result<String> {
    let output = git_status_opt(path, args)?;
    if !output.status.success() {
        return Err(Error::new(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| Error::new(format!("Git output is not UTF-8: {error}")))
}

fn git_checked(path: Option<&Path>, args: &[&str]) -> Result<()> {
    let output = git_status_opt(path, args)?;
    if !output.status.success() {
        return Err(Error::new(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn git_status_opt(path: Option<&Path>, args: &[&str]) -> Result<Output> {
    let mut command = Command::new("git");
    if let Some(path) = path {
        command.arg("-C").arg(path);
    }
    let output = command.args(args).output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::new("Git is missing; install Git and ensure it is available on PATH")
        } else {
            Error::new(format!("start Git: {error}"))
        }
    })?;
    Ok(output)
}

#[cfg(test)]
mod tests;
