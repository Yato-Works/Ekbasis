//! A thin, explicit wrapper around the `git` command line.
//!
//! AION never re-implements git: it creates branches and worktrees exactly the way a human
//! would, so alternative timelines stay inspectable with ordinary git tooling (plan §10).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// A git invocation context: one executable plus one working directory.
#[derive(Debug, Clone)]
pub struct Git {
    dir: PathBuf,
    exe: String,
    verbose: bool,
}

/// Commit metadata used in reports and CLI output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub sha: String,
    pub short: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub prunable: bool,
}

/// Executable used for git, overridable for tests and unusual installations.
fn default_exe() -> String {
    std::env::var("EKBASIS_GIT_EXE")
        .or_else(|_| std::env::var("AION_GIT_EXE"))
        .unwrap_or_else(|_| "git".to_string())
}

impl Git {
    /// Locates the repository containing `start` (like `git rev-parse --show-toplevel`).
    pub fn discover(start: impl AsRef<Path>) -> Result<Git> {
        let start = start.as_ref();
        let git = Git {
            dir: start.to_path_buf(),
            exe: default_exe(),
            verbose: false,
        };
        let output = git.raw(&["rev-parse", "--show-toplevel"]).context(
            "cannot run `git` - is it installed and on PATH? (override with AION_GIT_EXE)",
        )?;
        if !output.status.success() {
            bail!(
                "`{}` is not inside a git repository (run `git init` first)",
                start.display()
            );
        }
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if root.is_empty() {
            bail!("git did not report a repository root for `{}`", start.display());
        }
        Ok(Git {
            dir: PathBuf::from(root),
            ..git
        })
    }

    /// Same executable, different working directory (used for worktrees).
    pub fn in_dir(&self, dir: impl Into<PathBuf>) -> Git {
        Git {
            dir: dir.into(),
            ..self.clone()
        }
    }

    /// Prints every git command AION executes, when enabled.
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    /// Repository root (the directory that contains `.git`).
    pub fn root(&self) -> &Path {
        &self.dir
    }

    /// Working directory of this context.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Runs git and returns the raw output (never fails on a non-zero exit status).
    pub fn raw(&self, args: &[&str]) -> Result<Output> {
        if self.verbose {
            eprintln!("  $ git {}", args.join(" "));
        }
        Command::new(&self.exe)
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            .output()
            .with_context(|| format!("cannot execute `{} {}`", self.exe, args.join(" ")))
    }

    /// Runs git, expecting success, and returns trimmed stdout.
    pub fn run(&self, args: &[&str]) -> Result<String> {
        let output = self.raw(args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git {} failed: {}", args.join(" "), stderr.trim());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Runs git and reports only whether it succeeded.
    pub fn ok(&self, args: &[&str]) -> bool {
        match self.raw(args) {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    /// True when the repository has at least one commit.
    pub fn has_commits(&self) -> bool {
        self.ok(&["rev-parse", "--verify", "--quiet", "HEAD"])
    }

    /// Resolves a ref, tag or abbreviated hash to a full commit hash.
    pub fn resolve(&self, rev: &str) -> Result<String> {
        let reference = format!("{rev}^{{commit}}");
        let output = self.raw(&["rev-parse", "--verify", "--quiet", &reference])?;
        if !output.status.success() {
            bail!("`{rev}` is not a known commit, branch or tag in this repository");
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Short (abbreviated) form of a commit hash.
    pub fn short_sha(&self, rev: &str) -> Result<String> {
        self.run(&["rev-parse", "--short", rev])
    }

    /// Commit hash of HEAD.
    pub fn head_sha(&self) -> Result<String> {
        self.run(&["rev-parse", "HEAD"])
    }

    /// Current branch name, or `None` when HEAD is detached.
    pub fn head_branch(&self) -> Result<Option<String>> {
        let name = self.run(&["rev-parse", "--abbrev-ref", "HEAD"])?;
        if name == "HEAD" {
            Ok(None)
        } else {
            Ok(Some(name))
        }
    }

    /// True when a local branch with that exact name exists.
    pub fn branch_exists(&self, name: &str) -> bool {
        self.ok(&[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ])
    }

    /// All local branch names.
    pub fn local_branches(&self) -> Result<Vec<String>> {
        let output = self.run(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])?;
        Ok(output
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect())
    }

    /// True when the working tree has uncommitted changes.
    pub fn is_dirty(&self) -> Result<bool> {
        Ok(!self.run(&["status", "--porcelain"])?.trim().is_empty())
    }

    /// Metadata of one commit (used for reproducibility records).
    pub fn commit_info(&self, rev: &str) -> Result<CommitInfo> {
        let output = self.run(&["show", "-s", "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%s", rev])?;
        let mut fields = output.split('\u{1f}');
        let sha = fields.next().unwrap_or_default().trim().to_string();
        let short = fields.next().unwrap_or_default().trim().to_string();
        let author = fields.next().unwrap_or_default().trim().to_string();
        let date = fields.next().unwrap_or_default().trim().to_string();
        let subject = fields.next().unwrap_or_default().trim().to_string();
        if sha.is_empty() {
            bail!("cannot read commit metadata for `{rev}`");
        }
        Ok(CommitInfo {
            sha,
            short,
            author,
            date,
            subject,
        })
    }

    /// Files changed between two commits (`A path`, `M path`, ...).
    pub fn changed_files(&self, from: &str, to: &str) -> Result<Vec<String>> {
        let output = self.run(&["diff", "--name-status", from, to])?;
        Ok(output
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect())
    }

    /// `git diff --stat` between two commits.
    pub fn diff_stat(&self, from: &str, to: &str) -> Result<String> {
        self.run(&["diff", "--stat", from, to])
    }

    /// Stages everything and commits, returning the new commit hash.
    ///
    /// When the repository has no `user.name` / `user.email` configured, AION passes a local
    /// identity instead of failing, so experiments also work in throw-away repositories.
    pub fn add_all_and_commit(&self, message: &str) -> Result<String> {
        self.run(&["add", "--all"])?;
        let has_name = !self
            .run(&["config", "user.name"])
            .unwrap_or_default()
            .trim()
            .is_empty();
        let has_email = !self
            .run(&["config", "user.email"])
            .unwrap_or_default()
            .trim()
            .is_empty();
        let mut args: Vec<String> = Vec::new();
        if !has_name {
            args.push("-c".to_string());
            args.push("user.name=AION Experiment".to_string());
        }
        if !has_email {
            args.push("-c".to_string());
            args.push("user.email=aion@localhost".to_string());
        }
        args.push("commit".to_string());
        args.push("-m".to_string());
        args.push(message.to_string());
        let borrowed: Vec<&str> = args.iter().map(|arg| arg.as_str()).collect();
        self.run(&borrowed)?;
        self.head_sha()
    }

    /// Creates (or force-resets) a branch at `start` — the "Branch as Future" step (plan §3.2).
    pub fn create_branch(&self, name: &str, start: &str) -> Result<()> {
        self.run(&["branch", "--force", name, start])?;
        Ok(())
    }

    /// Deletes a branch, ignoring "not found" errors.
    pub fn delete_branch(&self, name: &str) -> Result<()> {
        let output = self.raw(&["branch", "-D", name])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("not found") {
                bail!("cannot delete branch `{name}`: {}", stderr.trim());
            }
        }
        Ok(())
    }

    /// Adds a worktree. With `branch = None` a detached worktree at `start` is created.
    pub fn worktree_add(
        &self,
        path: &Path,
        branch: Option<&str>,
        start: Option<&str>,
        force: bool,
    ) -> Result<()> {
        let mut args: Vec<String> = vec!["worktree".to_string(), "add".to_string()];
        if force {
            args.push("--force".to_string());
        }
        if branch.is_none() {
            args.push("--detach".to_string());
        }
        args.push(path.to_string_lossy().to_string());
        match (branch, start) {
            (Some(branch), _) => args.push(branch.to_string()),
            (None, Some(start)) => args.push(start.to_string()),
            (None, None) => {}
        }
        let borrowed: Vec<&str> = args.iter().map(|arg| arg.as_str()).collect();
        self.run(&borrowed)?;
        Ok(())
    }

    /// Removes a worktree and prunes its administrative files.
    pub fn worktree_remove(&self, path: &Path) -> Result<()> {
        let path_text = path.to_string_lossy().to_string();
        let output = self.raw(&["worktree", "remove", "--force", &path_text])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("cannot remove worktree `{path_text}`: {}", stderr.trim());
        }
        let _ = self.worktree_prune();
        Ok(())
    }

    /// Prunes stale worktree records (e.g. after a directory was deleted manually).
    pub fn worktree_prune(&self) -> Result<()> {
        let output = self.raw(&["worktree", "prune"])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git worktree prune failed: {}", stderr.trim());
        }
        Ok(())
    }

    /// Parses `git worktree list --porcelain`.
    pub fn worktree_list(&self) -> Result<Vec<WorktreeEntry>> {
        let output = self.run(&["worktree", "list", "--porcelain"])?;
        let mut entries: Vec<WorktreeEntry> = Vec::new();
        for line in output.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("worktree ") {
                entries.push(WorktreeEntry {
                    path: PathBuf::from(rest.trim()),
                    head: None,
                    branch: None,
                    detached: false,
                    bare: false,
                    prunable: false,
                });
                continue;
            }
            let Some(entry) = entries.last_mut() else {
                continue;
            };
            if let Some(rest) = line.strip_prefix("HEAD ") {
                entry.head = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("branch ") {
                entry.branch = Some(rest.trim().trim_start_matches("refs/heads/").to_string());
            } else if line == "detached" {
                entry.detached = true;
            } else if line == "bare" {
                entry.bare = true;
            } else if line.starts_with("prunable") {
                entry.prunable = true;
            }
        }
        Ok(entries)
    }

    /// Adds an entry to `.gitignore` when missing. Returns `true` when the file changed.
    pub fn ensure_gitignore_entry(&self, entry: &str) -> Result<bool> {
        let path = self.dir.join(".gitignore");
        let existing = if path.exists() {
            std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read `{}`", path.display()))?
        } else {
            String::new()
        };
        let present = existing.lines().any(|line| {
            line.trim().trim_end_matches('/') == entry.trim().trim_end_matches('/')
        });
        if present {
            return Ok(false);
        }
        let mut updated = existing;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(entry);
        updated.push('\n');
        std::fs::write(&path, updated).with_context(|| format!("cannot write `{}`", path.display()))?;
        Ok(true)
    }

    /// Adds an entry to `.git/info/exclude`, the *local* ignore file. Returns `true` when changed.
    ///
    /// Unlike `.gitignore`, this file is never committed and applies to every branch and worktree of
    /// the clone, which is exactly what AION's own state directory needs: a branch created before the
    /// `.gitignore` entry was committed must not be able to stage `.aion/` by accident.
    pub fn ensure_local_exclude(&self, entry: &str) -> Result<bool> {
        let raw = self.run(&["rev-parse", "--git-common-dir"])?;
        let git_dir = PathBuf::from(raw);
        let git_dir = if git_dir.is_absolute() {
            git_dir
        } else {
            self.dir.join(git_dir)
        };
        let path = git_dir.join("info").join("exclude");
        let existing = if path.exists() {
            std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read `{}`", path.display()))?
        } else {
            String::new()
        };
        let present = existing.lines().any(|line| {
            line.trim().trim_end_matches('/') == entry.trim().trim_end_matches('/')
        });
        if present {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create `{}`", parent.display()))?;
        }
        let mut updated = existing;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(entry);
        updated.push('\n');
        std::fs::write(&path, updated).with_context(|| format!("cannot write `{}`", path.display()))?;
        Ok(true)
    }
}
