//! End-to-end tests against real `git` in temporary directories.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use coocoo_proto::util::new_id;
use coocoo_proto::{ConnSink, DiffResult, DiffStat, Emitter, HostStats, Paths, Project, TerminalInfo, WorktreeRef};
use coocoo_workspace::{Stats, Terminals, Workspace};
use serde_json::{Value, json};

/// Temporary directory removed on drop.
struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("coocoo-ws-test-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(std::fs::canonicalize(&dir).unwrap())
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A repository with deterministic local identity and one commit on `main`.
fn init_repo(dir: &Path, commit: bool) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    git(dir, &["config", "core.hooksPath", "/dev/null"]);
    if commit {
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.join("gone.txt"), "bye\n").unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", "initial"]);
    }
}

struct Env {
    _tmp: Temp,
    root: PathBuf,
    emitter: Emitter,
    ws: Arc<Workspace>,
}

fn env() -> Env {
    let tmp = Temp::new();
    let root = tmp.0.clone();
    let emitter = Emitter::new("host-a", 256);
    let ws = Workspace::new(emitter.clone(), &Paths::under(root.join("home")));
    Env { _tmp: tmp, root, emitter, ws }
}

async fn call<T: serde::de::DeserializeOwned>(ws: &Arc<Workspace>, method: &str, params: Value) -> T {
    let value = ws.dispatch(method, params).await.unwrap_or_else(|e| panic!("{method}: {e}"));
    serde_json::from_value(value).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn project_git_info_diff_and_persistence() {
    let e = env();
    let repo = e.root.join("repo");
    init_repo(&repo, true);

    let project: Project = call(&e.ws, "projects.add", json!({ "path": repo, "name": "Repo" })).await;
    assert_eq!(project.host_id, "host-a");
    assert_eq!(project.name, "Repo");
    let info = project.git.clone().expect("git info");
    assert_eq!(info.branch.as_deref(), Some("main"));
    assert_eq!(info.changed, 0);
    assert_eq!(info.last_commit.as_ref().unwrap().summary, "initial");
    assert_eq!(info.head.as_deref(), Some(git(&repo, &["rev-parse", "--short", "HEAD"]).as_str()));
    assert_eq!(info.worktrees.len(), 1);
    assert!(info.worktrees[0].is_main);

    // Modify, delete, add untracked.
    std::fs::write(repo.join("a.txt"), "one\nTWO\nthree\nfour\n").unwrap();
    std::fs::remove_file(repo.join("gone.txt")).unwrap();
    std::fs::write(repo.join("new.txt"), "x\ny\n").unwrap();
    std::fs::write(repo.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();

    let refreshed: Project = call(&e.ws, "projects.refresh", json!({ "id": project.id })).await;
    assert_eq!(refreshed.git.unwrap().changed, 4);

    let diff: DiffResult = call(&e.ws, "git.diff", json!({ "cwd": repo })).await;
    assert_eq!(diff.base, "HEAD");
    assert!(!diff.truncated);
    let file = |p: &str| diff.files.iter().find(|f| f.path == p).unwrap_or_else(|| panic!("{p} missing"));
    let a = file("a.txt");
    assert_eq!((a.status.as_str(), a.additions, a.deletions), ("M", 2, 1));
    assert!(a.patch.starts_with("@@ "), "{}", a.patch);
    assert!(a.patch.contains("+TWO\n") && a.patch.contains("-two\n"));
    let gone = file("gone.txt");
    assert_eq!((gone.status.as_str(), gone.deletions), ("D", 1));
    let new = file("new.txt");
    assert_eq!((new.status.as_str(), new.additions, new.deletions), ("?", 2, 0));
    assert_eq!(new.patch, "@@ -0,0 +1,2 @@\n+x\n+y\n");
    let blob = file("blob.bin");
    assert!(blob.binary && blob.patch.is_empty());

    let stat: DiffStat = call(&e.ws, "git.diffStat", json!({ "cwd": repo })).await;
    assert_eq!(stat, DiffStat { files: 4, additions: 4, deletions: 2 });

    // Commit everything.
    let commit: coocoo_proto::CommitInfo =
        call(&e.ws, "git.commit", json!({ "cwd": repo, "message": "second" })).await;
    assert_eq!(commit.summary, "second");
    assert_eq!(commit.author, "Test");
    let err = e.ws.dispatch("git.commit", json!({ "cwd": repo, "message": "again" })).await.unwrap_err();
    assert_eq!(err.code, "conflict");
    let log: Vec<coocoo_proto::CommitInfo> = call(&e.ws, "git.log", json!({ "cwd": repo, "limit": 5 })).await;
    assert_eq!(log.iter().map(|c| c.summary.as_str()).collect::<Vec<_>>(), ["second", "initial"]);

    // Errors.
    let err = e.ws.dispatch("git.diff", json!({ "cwd": e.root.join("missing") })).await.unwrap_err();
    assert_eq!(err.code, "not_found");
    let plain = e.root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let err = e.ws.dispatch("git.diff", json!({ "cwd": plain })).await.unwrap_err();
    assert_eq!(err.code, "invalid_params");
    let plain_project: Project = call(&e.ws, "projects.add", json!({ "path": plain })).await;
    assert!(plain_project.git.is_none());
    assert_eq!(plain_project.name, "plain");
    let err = e.ws.dispatch("git.worktreeCreate", json!({ "projectId": plain_project.id })).await.unwrap_err();
    assert_eq!(err.code, "invalid_params");

    // Update / persistence / remove.
    let updated: Project =
        call(&e.ws, "projects.update", json!({ "id": project.id, "devUrls": ["http://localhost:5173"] })).await;
    assert_eq!(updated.dev_urls, ["http://localhost:5173"]);
    let reloaded = Workspace::new(Emitter::new("host-b", 16), &Paths::under(e.root.join("home")));
    let projects = reloaded.projects();
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0].host_id, "host-b");
    assert_eq!(projects[0].dev_urls, ["http://localhost:5173"]);
    drop(reloaded);
    let _: Value = call(&e.ws, "projects.remove", json!({ "id": plain_project.id })).await;
    assert_eq!(e.ws.projects().len(), 1);
    let err = e.ws.dispatch("projects.get", json!({ "id": plain_project.id })).await.unwrap_err();
    assert_eq!(err.code, "not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn worktree_diff_and_merge() {
    let e = env();
    let repo = e.root.join("repo");
    init_repo(&repo, true);
    let project: Project = call(&e.ws, "projects.add", json!({ "path": repo })).await;

    let wt: WorktreeRef = call(&e.ws, "git.worktreeCreate", json!({ "projectId": project.id })).await;
    assert!(wt.branch.starts_with("coocoo/"));
    assert_eq!(wt.base, "main");
    let wt_path = PathBuf::from(&wt.path);
    assert!(wt_path.join("a.txt").exists());
    assert!(wt_path.starts_with(e.root.join("home/data/worktrees")));
    assert_eq!(git(&wt_path, &["rev-parse", "--abbrev-ref", "HEAD"]), wt.branch);
    let p: Project = call(&e.ws, "projects.get", json!({ "id": project.id })).await;
    let wts = p.git.unwrap().worktrees;
    assert_eq!(wts.len(), 2);
    assert_eq!(wts[1].branch.as_deref(), Some(wt.branch.as_str()));
    assert_eq!(wts[1].changed, Some(0));

    // Commit on the branch; diff against base shows it.
    std::fs::write(wt_path.join("feature.txt"), "feature\n").unwrap();
    let _: Value = call(&e.ws, "git.commit", json!({ "cwd": wt.path, "message": "feature" })).await;
    let diff: DiffResult = call(&e.ws, "git.diff", json!({ "cwd": wt.path, "base": "main" })).await;
    assert_eq!(diff.base, git(&repo, &["rev-parse", "main"]));
    assert_eq!(diff.files.len(), 1);
    assert_eq!((diff.files[0].path.as_str(), diff.files[0].status.as_str()), ("feature.txt", "A"));
    let plain: DiffResult = call(&e.ws, "git.diff", json!({ "cwd": wt.path })).await;
    assert!(plain.files.is_empty());

    // Merge back.
    let merged: Value = call(&e.ws, "git.merge", json!({ "projectId": project.id, "branch": wt.branch })).await;
    assert_eq!(merged["ok"], true, "{merged}");
    assert!(repo.join("feature.txt").exists());
    assert_eq!(git(&repo, &["log", "-1", "--format=%P"]).split(' ').count(), 2, "merge commit expected");

    // Conflict path aborts cleanly.
    let wt2: WorktreeRef =
        call(&e.ws, "git.worktreeCreate", json!({ "projectId": project.id, "branch": "conflict/one" })).await;
    assert!(wt2.path.ends_with("conflict-one"));
    std::fs::write(PathBuf::from(&wt2.path).join("a.txt"), "branch\n").unwrap();
    let _: Value = call(&e.ws, "git.commit", json!({ "cwd": wt2.path, "message": "branch side" })).await;
    std::fs::write(repo.join("a.txt"), "main\n").unwrap();
    let _: Value = call(&e.ws, "git.commit", json!({ "cwd": repo, "message": "main side" })).await;
    let head_before = git(&repo, &["rev-parse", "HEAD"]);
    let merged: Value = call(&e.ws, "git.merge", json!({ "projectId": project.id, "branch": "conflict/one" })).await;
    assert_eq!(merged["ok"], false);
    assert!(merged["output"].as_str().unwrap().contains("CONFLICT"), "{merged}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert!(!repo.join(".git/MERGE_HEAD").exists());

    // Remove worktrees (+ branch).
    let _: Value = call(
        &e.ws,
        "git.worktreeRemove",
        json!({ "projectId": project.id, "path": wt2.path, "deleteBranch": true }),
    )
    .await;
    assert!(!PathBuf::from(&wt2.path).exists());
    assert_eq!(git(&repo, &["branch", "--list", "conflict/one"]), "");
    let _: Value = call(&e.ws, "git.worktreeRemove", json!({ "projectId": project.id, "path": wt.path })).await;
    assert!(!git(&repo, &["branch", "--list", &wt.branch]).is_empty());
    let p: Project = call(&e.ws, "projects.get", json!({ "id": project.id })).await;
    assert_eq!(p.git.unwrap().worktrees.len(), 1);
    let err = e.ws.dispatch("git.worktreeRemove", json!({ "projectId": project.id, "path": wt.path })).await.unwrap_err();
    assert_eq!(err.code, "not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_without_commits() {
    let e = env();
    let repo = e.root.join("empty");
    init_repo(&repo, false);
    std::fs::write(repo.join("staged.txt"), "s\n").unwrap();
    git(&repo, &["add", "staged.txt"]);
    std::fs::write(repo.join("loose.txt"), "l\nm").unwrap();

    let project: Project = call(&e.ws, "projects.add", json!({ "path": repo })).await;
    let info = project.git.unwrap();
    assert_eq!(info.branch.as_deref(), Some("main"));
    assert!(info.head.is_none() && info.last_commit.is_none());
    assert_eq!(info.changed, 2);

    let diff: DiffResult = call(&e.ws, "git.diff", json!({ "cwd": repo })).await;
    assert_eq!(diff.files.len(), 2);
    let staged = diff.files.iter().find(|f| f.path == "staged.txt").unwrap();
    assert_eq!((staged.status.as_str(), staged.additions), ("A", 1));
    let loose = diff.files.iter().find(|f| f.path == "loose.txt").unwrap();
    assert_eq!((loose.status.as_str(), loose.additions), ("?", 2));
    assert!(loose.patch.ends_with("+m\n\\ No newline at end of file\n"));
    let log: Vec<Value> = call(&e.ws, "git.log", json!({ "cwd": repo })).await;
    assert!(log.is_empty());
    let err = e.ws.dispatch("git.worktreeCreate", json!({ "projectId": project.id })).await.unwrap_err();
    assert_eq!(err.code, "conflict");
    let commit: coocoo_proto::CommitInfo = call(&e.ws, "git.commit", json!({ "cwd": repo, "message": "first" })).await;
    assert_eq!(commit.summary, "first");
}

/// Run git expecting failure (merge conflicts and the like).
fn git_fails(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(!out.status.success(), "git {args:?} unexpectedly succeeded");
}

fn commit_file(dir: &Path, name: &str, content: &str, message: &str) {
    std::fs::write(dir.join(name), content).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

#[tokio::test(flavor = "multi_thread")]
async fn upstream_base_fetch_and_fast_forward() {
    let e = env();
    let up = e.root.join("up");
    init_repo(&up, true);
    let repo = e.root.join("repo");
    git(&e.root, &["clone", "-q", up.to_str().unwrap(), repo.to_str().unwrap()]);
    for (k, v) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false")] {
        git(&repo, &["config", k, v]);
    }
    let project: Project = call(&e.ws, "projects.add", json!({ "path": repo })).await;
    let main = |p: &Project| p.git.as_ref().unwrap().worktrees[0].clone();
    assert_eq!(main(&project).upstream.as_deref(), Some("origin/main"));
    assert_eq!((main(&project).ahead, main(&project).behind), (0, 0));
    assert!(!repo.join(".git/FETCH_HEAD").exists());
    assert_eq!(project.git.as_ref().unwrap().fetched_at, None);

    // A linked worktree: counted against the main worktree's branch; its own FETCH_HEAD counts.
    let wt: WorktreeRef = call(&e.ws, "git.worktreeCreate", json!({ "projectId": project.id })).await;
    let wt_path = PathBuf::from(&wt.path);
    commit_file(&wt_path, "feature.txt", "f\n", "feature");
    commit_file(&repo, "main.txt", "m\n", "main moves");
    git(&wt_path, &["fetch", "-q"]);
    let p: Project = call(&e.ws, "projects.refresh", json!({ "id": project.id })).await;
    let info = p.git.clone().unwrap();
    assert!(info.fetched_at.is_some(), "FETCH_HEAD of the linked worktree");
    let linked = info.worktrees.iter().find(|w| w.path == wt.path).unwrap();
    assert_eq!((linked.base_ahead, linked.base_behind), (Some(1), Some(1)));
    assert_eq!((linked.conflicts, linked.operation), (0, None));
    assert_eq!((main(&p).base_ahead, main(&p).ahead), (None, 1));

    // A conflicting merge in the worktree shows up as operation + conflicts.
    commit_file(&wt_path, "a.txt", "branch\n", "branch side");
    commit_file(&repo, "a.txt", "main\n", "main side");
    git_fails(&wt_path, &["merge", "-q", "main"]);
    let p: Project = call(&e.ws, "projects.refresh", json!({ "id": project.id })).await;
    let linked = p.git.unwrap().worktrees.into_iter().find(|w| w.path == wt.path).unwrap();
    assert_eq!(linked.operation, Some(coocoo_proto::GitOperation::Merge));
    assert_eq!(linked.conflicts, 1);
    git(&wt_path, &["merge", "--abort"]);

    // Upstream moves: unknown until fetched, then fast-forward once the local commits are gone.
    commit_file(&up, "u1.txt", "1\n", "upstream 1");
    commit_file(&up, "u2.txt", "2\n", "upstream 2");
    let fetched: Value = call(&e.ws, "git.fetch", json!({ "projectId": project.id })).await;
    assert_eq!(fetched["ok"], true, "{fetched}");
    let p: Project = call(&e.ws, "projects.get", json!({ "id": project.id })).await;
    assert_eq!((main(&p).ahead, main(&p).behind), (2, 2));
    let ff = json!({ "projectId": project.id, "path": repo });
    let err = e.ws.dispatch("git.fastForward", ff.clone()).await.unwrap_err();
    assert_eq!(err.code, "conflict");
    assert!(err.message.contains("2 commit(s)"), "{}", err.message);

    git(&repo, &["reset", "-q", "--hard", "HEAD~2"]);
    std::fs::write(repo.join("loose.txt"), "x\n").unwrap();
    let err = e.ws.dispatch("git.fastForward", ff.clone()).await.unwrap_err();
    assert!(err.message.contains("uncommitted"), "{}", err.message);
    std::fs::remove_file(repo.join("loose.txt")).unwrap();
    let done: Value = call(&e.ws, "git.fastForward", ff.clone()).await;
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), git(&up, &["rev-parse", "HEAD"]));
    let p: Project = call(&e.ws, "projects.get", json!({ "id": project.id })).await;
    assert_eq!((main(&p).ahead, main(&p).behind), (0, 0));

    // A branch without upstream cannot be fast-forwarded.
    let err = e.ws.dispatch("git.fastForward", json!({ "projectId": project.id, "path": wt.path })).await.unwrap_err();
    assert!(err.message.contains("no upstream"), "{}", err.message);
}

#[tokio::test(flavor = "multi_thread")]
async fn fs_list_orders_and_hides() {
    let e = env();
    let dir = e.root.join("browse");
    for d in ["Zeta", "alpha", ".hidden", "proj"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::create_dir_all(dir.join("proj/.git")).unwrap();
    for f in ["b.txt", "A.md", ".env"] {
        std::fs::write(dir.join(f), "").unwrap();
    }
    let listing: Value = call(&e.ws, "fs.list", json!({ "path": dir })).await;
    let names: Vec<&str> = listing["entries"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["alpha", "proj", "Zeta", "A.md", "b.txt"]);
    assert_eq!(listing["entries"][1]["isGit"], true);
    assert_eq!(listing["entries"][0]["isGit"], false);
    assert_eq!(listing["entries"][3]["isDir"], false);
    assert_eq!(listing["parent"], e.root.to_string_lossy().as_ref());
    let root: Value = call(&e.ws, "fs.list", json!({ "path": "/" })).await;
    assert!(root.get("parent").is_none());
    let home: Value = call(&e.ws, "fs.list", json!({})).await;
    assert_eq!(home["path"], std::fs::canonicalize(dirs_home()).unwrap().to_string_lossy().as_ref());
    let err = e.ws.dispatch("fs.list", json!({ "path": dir.join("nope") })).await.unwrap_err();
    assert_eq!(err.code, "not_found");
}

fn dirs_home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn watcher_emits_after_commit() {
    let e = env();
    let repo = e.root.join("repo");
    init_repo(&repo, true);
    let project: Project = call(&e.ws, "projects.add", json!({ "path": repo })).await;
    let mut rx = e.emitter.subscribe();
    // Let the add/initial-scan events settle, then drain them.
    tokio::time::sleep(Duration::from_millis(600)).await;
    while rx.try_recv().is_ok() {}

    std::fs::write(repo.join("c.txt"), "c\n").unwrap();
    git(&repo, &["add", "c.txt"]);
    git(&repo, &["commit", "-q", "-m", "external commit"]);
    let started = std::time::Instant::now();
    let updated = tokio::time::timeout(Duration::from_millis(1500), async {
        loop {
            let ev = rx.recv().await.unwrap();
            if ev.name != "project.updated" {
                continue;
            }
            let env: Value = serde_json::from_str(&ev.json).unwrap();
            let p: Project = serde_json::from_value(env["data"].clone()).unwrap();
            if p.id == project.id && p.git.as_ref().and_then(|g| g.last_commit.as_ref()).is_some_and(|c| c.summary == "external commit") {
                return p;
            }
        }
    })
    .await
    .expect("project.updated after commit");
    assert!(started.elapsed() < Duration::from_millis(1200), "took {:?}", started.elapsed());
    assert_eq!(updated.git.unwrap().changed, 0);
}

/// Next `event` whose payload satisfies `pred`.
async fn next_event<T: serde::de::DeserializeOwned>(
    rx: &mut tokio::sync::broadcast::Receiver<Arc<coocoo_proto::bus::BusEvent>>,
    event: &str,
    pred: impl Fn(&T) -> bool,
) -> T {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let ev = rx.recv().await.unwrap();
            if ev.name != event {
                continue;
            }
            let env: Value = serde_json::from_str(&ev.json).unwrap();
            let data: T = serde_json::from_value(env["data"].clone()).unwrap();
            if pred(&data) {
                return data;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no matching {event}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn actions_file_is_watched_and_runs_in_worktrees() {
    let e = env();
    let repo = e.root.join("repo");
    init_repo(&repo, true);
    let project: Project = call(&e.ws, "projects.add", json!({ "path": repo })).await;
    assert!(project.actions.is_empty() && project.action_errors.is_empty());
    let mut rx = e.emitter.subscribe();
    tokio::time::sleep(Duration::from_millis(600)).await;
    while rx.try_recv().is_ok() {}

    // `.coocoo/` appears after the project was added, uncommitted.
    std::fs::create_dir_all(repo.join(".coocoo")).unwrap();
    std::fs::write(
        repo.join(".coocoo/actions.toml"),
        "[[action]]\nname = \"Where\"\nrun = \"pwd; echo \\\"$COOCOO_ACTION in $COOCOO_WORKDIR\\\"; exit 3\"\ncwd = \"sub\"\n",
    )
    .unwrap();
    let started = std::time::Instant::now();
    let p: Project = next_event(&mut rx, "project.updated", |p: &Project| p.id == project.id && !p.actions.is_empty()).await;
    assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
    assert_eq!(p.actions[0].id, "where");

    // Editing it again: a broken entry is reported, the good one stays.
    let text = std::fs::read_to_string(repo.join(".coocoo/actions.toml")).unwrap();
    std::fs::write(repo.join(".coocoo/actions.toml"), format!("{text}\n[[action]]\nname = \"Broken\"\n")).unwrap();
    let p: Project = next_event(&mut rx, "project.updated", |p: &Project| p.id == project.id && !p.action_errors.is_empty()).await;
    assert_eq!(p.actions.len(), 1);
    assert!(p.action_errors[0].starts_with("actions.toml:6:1 – action “Broken”"), "{:?}", p.action_errors);

    // Runs in an agent's worktree (its `cwd` inside it), keeps its exit code after it ended.
    let wt: WorktreeRef = call(&e.ws, "git.worktreeCreate", json!({ "projectId": project.id })).await;
    std::fs::create_dir_all(Path::new(&wt.path).join("sub")).unwrap();
    let terminals = Terminals::new(e.emitter.clone());
    let run = |cwd: String| {
        let (ws, terminals) = (e.ws.clone(), terminals.clone());
        let params = json!({ "projectId": project.id, "actionId": "where", "cwd": cwd });
        async move { ws.dispatch_actions(&terminals, "actions.run", params).await }
    };
    let info: TerminalInfo = serde_json::from_value(run(wt.path.clone()).await.unwrap()).unwrap();
    assert_eq!(info.cwd, format!("{}/sub", std::fs::canonicalize(&wt.path).unwrap().display()));
    assert_eq!(info.action.as_ref().unwrap().base, wt.path);
    let ended: TerminalInfo = next_event(&mut rx, "terminal.updated", |t: &TerminalInfo| t.id == info.id && t.exited_at.is_some()).await;
    assert_eq!(ended.exit_code, Some(3));
    let (tx, _out) = tokio::sync::mpsc::channel(8);
    let conn = ConnSink::new(1, tx);
    let attached = terminals.dispatch("terminal.attach", json!({ "id": info.id }), &conn).await.unwrap();
    let output = attached["data"].as_str().unwrap();
    assert!(output.contains(&format!("where in {}", std::fs::canonicalize(&wt.path).unwrap().display())), "{output}");

    // Running it again replaces the finished run; another directory runs alongside, once the
    // action's `cwd` exists there too (it is resolved per base directory).
    let again: TerminalInfo = serde_json::from_value(run(wt.path.clone()).await.unwrap()).unwrap();
    let err = run(repo.display().to_string()).await.unwrap_err();
    assert!(err.code == "not_found" && err.message.contains("sub"), "{err}");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    let main: TerminalInfo = serde_json::from_value(run(repo.display().to_string()).await.unwrap()).unwrap();
    let list: Vec<TerminalInfo> =
        serde_json::from_value(terminals.dispatch("terminal.list", Value::Null, &conn).await.unwrap()).unwrap();
    let ids: Vec<&str> = list.iter().map(|t| t.id.as_str()).collect();
    assert!(!ids.contains(&info.id.as_str()) && ids.contains(&again.id.as_str()) && ids.contains(&main.id.as_str()), "{ids:?}");

    // Outside the project and its worktrees: refused.
    let outside = e.root.join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    let err = run(outside.display().to_string()).await.unwrap_err();
    assert_eq!(err.code, "invalid_params", "{err}");

    // Closing a finished run removes it.
    next_event(&mut rx, "terminal.updated", |t: &TerminalInfo| t.id == again.id && t.exited_at.is_some()).await;
    terminals.dispatch("terminal.close", json!({ "id": again.id }), &conn).await.unwrap();
    let gone: Value = next_event(&mut rx, "terminal.removed", |v: &Value| v["id"] == again.id.as_str()).await;
    assert_eq!(gone["id"], again.id.as_str());
}

#[tokio::test(flavor = "multi_thread")]
async fn stats_sample_and_watch() {
    let emitter = Emitter::new("host-s", 64);
    let child = Command::new("sleep").arg("5").spawn().unwrap();
    let child_pid = child.id();
    let stats = Stats::new(emitter.clone(), Arc::new(move || vec![("agent-1".to_owned(), child_pid)]));
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let conn = ConnSink::new(7, tx);

    let sample: HostStats = serde_json::from_value(stats.dispatch("stats.get", Value::Null, &conn).await.unwrap()).unwrap();
    assert_eq!(sample.host_id, "host-s");
    assert!(sample.cpu_count > 0);
    assert!(sample.mem_total > 0 && sample.mem_used > 0);
    let daemon = sample.procs.iter().find(|p| p.key == "daemon").unwrap();
    assert_eq!(daemon.pid, std::process::id());
    // The daemon tree includes the child process too.
    let agent = sample.procs.iter().find(|p| p.key == "agent-1").unwrap();
    assert!(agent.rss > 0 && daemon.rss >= agent.rss);

    let mut rx = emitter.subscribe();
    stats.dispatch("stats.watch", json!({ "enabled": true }), &conn).await.unwrap();
    let ev = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
    assert_eq!(ev.name, "host.stats");
    stats.conn_closed(7);
    tokio::time::sleep(Duration::from_millis(300)).await;
    while rx.try_recv().is_ok() {}
    assert!(tokio::time::timeout(Duration::from_millis(2500), rx.recv()).await.is_err(), "no stats after unwatch");
    let mut child = child;
    let _ = child.kill();
    let _ = child.wait();
}
