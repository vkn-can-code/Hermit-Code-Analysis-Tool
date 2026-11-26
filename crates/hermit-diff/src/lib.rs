//! Git diff extraction: turns a `base..head` range into changed files and line ranges,
//! and reads file contents at either revision without touching the working tree.

use anyhow::{Context, Result, anyhow};
use git2::{Delta, DiffFindOptions, DiffOptions, Oid, Repository};
use hermit_core::{ChangedFile, FileStatus, Language, LineRange};
use std::path::{Path, PathBuf};

/// An opened repository with resolved base and head commits.
pub struct ChangeSet {
    repo: Repository,
    pub base: Oid,
    pub head: Oid,
}

impl ChangeSet {
    /// Opens `repo_path` and resolves `base` and `head` (any revspec: branch, tag, SHA).
    ///
    /// Like GitHub's pull-request view, the base used is the merge-base of the two,
    /// so commits that landed on the base branch after the fork are not counted.
    pub fn open(repo_path: &Path, base: &str, head: &str) -> Result<Self> {
        let repo = Repository::discover(repo_path)
            .with_context(|| format!("not a git repository: {}", repo_path.display()))?;
        let head_oid = resolve(&repo, head)?;
        let base_tip = resolve(&repo, base)?;
        let base_oid = repo.merge_base(base_tip, head_oid).unwrap_or(base_tip);
        Ok(Self {
            repo,
            base: base_oid,
            head: head_oid,
        })
    }

    /// Repository root (working directory or bare git dir).
    pub fn root(&self) -> PathBuf {
        self.repo
            .workdir()
            .unwrap_or_else(|| self.repo.path())
            .to_path_buf()
    }

    /// Lists every file changed between base and head, with exact added/removed line ranges.
    pub fn changed_files(&self) -> Result<Vec<ChangedFile>> {
        let base_tree = self.repo.find_commit(self.base)?.tree()?;
        let head_tree = self.repo.find_commit(self.head)?.tree()?;

        let mut opts = DiffOptions::new();
        opts.context_lines(0).ignore_submodules(true);
        let mut diff =
            self.repo
                .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))?;
        diff.find_similar(Some(DiffFindOptions::new().renames(true)))?;

        let mut files = Vec::new();
        for idx in 0..diff.deltas().len() {
            let delta = diff.get_delta(idx).expect("index in range");
            let status = match delta.status() {
                Delta::Added | Delta::Copied => FileStatus::Added,
                Delta::Deleted => FileStatus::Deleted,
                Delta::Renamed => FileStatus::Renamed,
                Delta::Modified | Delta::Typechange => FileStatus::Modified,
                _ => continue,
            };
            if delta.new_file().is_binary() || delta.old_file().is_binary() {
                continue;
            }
            let new_path = delta.new_file().path().map(Path::to_path_buf);
            let old_path = delta.old_file().path().map(Path::to_path_buf);
            let path = match status {
                FileStatus::Deleted => old_path.clone(),
                _ => new_path.clone(),
            }
            .ok_or_else(|| anyhow!("delta without path"))?;

            let mut file = ChangedFile {
                language: Language::from_path(&path),
                old_path: (status == FileStatus::Renamed)
                    .then(|| old_path.clone())
                    .flatten(),
                path,
                status,
                added: Vec::new(),
                removed: Vec::new(),
                additions: 0,
                deletions: 0,
            };

            if let Some(patch) = git2::Patch::from_diff(&diff, idx)? {
                for h in 0..patch.num_hunks() {
                    let lines = patch.num_lines_in_hunk(h)?;
                    for l in 0..lines {
                        let line = patch.line_in_hunk(h, l)?;
                        match line.origin() {
                            '+' => {
                                if let Some(n) = line.new_lineno() {
                                    push_line(&mut file.added, n);
                                    file.additions += 1;
                                }
                            }
                            '-' => {
                                if let Some(n) = line.old_lineno() {
                                    push_line(&mut file.removed, n);
                                    file.deletions += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            files.push(file);
        }
        Ok(files)
    }

    /// Renders the diff with 3 lines of context and explicit head line numbers:
    ///
    /// ```text
    /// ### services/billing.py
    ///    12 |   def f(x):
    ///       | - old line
    ///    13 | + new line
    /// ```
    ///
    /// Line numbers let a language model cite exact locations. Output stops after `max_lines`.
    pub fn numbered_patch(&self, only: &[PathBuf], max_lines: usize) -> Result<String> {
        let base_tree = self.repo.find_commit(self.base)?.tree()?;
        let head_tree = self.repo.find_commit(self.head)?.tree()?;
        let mut opts = DiffOptions::new();
        opts.context_lines(3).ignore_submodules(true);
        for p in only {
            opts.pathspec(p);
        }
        let mut diff =
            self.repo
                .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))?;
        diff.find_similar(Some(DiffFindOptions::new().renames(true)))?;

        let mut out = String::new();
        let mut count = 0usize;
        'files: for idx in 0..diff.deltas().len() {
            let Some(patch) = git2::Patch::from_diff(&diff, idx)? else {
                continue;
            };
            let delta = patch.delta();
            if delta.new_file().is_binary() {
                continue;
            }
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            out.push_str(&format!("### {path}\n"));
            for h in 0..patch.num_hunks() {
                if h > 0 {
                    out.push_str("      | ...\n");
                }
                for l in 0..patch.num_lines_in_hunk(h)? {
                    let line = patch.line_in_hunk(h, l)?;
                    let content = String::from_utf8_lossy(line.content());
                    let content = content.trim_end_matches(['\n', '\r']);
                    let row = match line.origin() {
                        '+' => format!("{:>5} | + {content}\n", line.new_lineno().unwrap_or(0)),
                        '-' => format!("      | - {content}\n"),
                        ' ' => format!("{:>5} |   {content}\n", line.new_lineno().unwrap_or(0)),
                        _ => continue,
                    };
                    out.push_str(&row);
                    count += 1;
                    if count >= max_lines {
                        out.push_str("      | [diff truncated]\n");
                        break 'files;
                    }
                }
            }
            out.push('\n');
        }
        Ok(out)
    }

    /// Reads a file's text at the base commit. `None` if absent or not UTF-8.
    pub fn read_base(&self, path: &Path) -> Result<Option<String>> {
        self.read_at(self.base, path)
    }

    /// Reads a file's text at the head commit. `None` if absent or not UTF-8.
    pub fn read_head(&self, path: &Path) -> Result<Option<String>> {
        self.read_at(self.head, path)
    }

    /// Lists every blob path in the head tree (used to build the repo-wide symbol index).
    pub fn head_paths(&self) -> Result<Vec<PathBuf>> {
        let tree = self.repo.find_commit(self.head)?.tree()?;
        let mut out = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob)
                && let Ok(name) = entry.name()
            {
                out.push(PathBuf::from(dir).join(name));
            }
            git2::TreeWalkResult::Ok
        })?;
        Ok(out)
    }

    fn read_at(&self, commit: Oid, path: &Path) -> Result<Option<String>> {
        let tree = self.repo.find_commit(commit)?.tree()?;
        let entry = match tree.get_path(path) {
            Ok(e) => e,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let obj = entry.to_object(&self.repo)?;
        let Some(blob) = obj.as_blob() else {
            return Ok(None);
        };
        Ok(std::str::from_utf8(blob.content()).ok().map(str::to_owned))
    }
}

fn resolve(repo: &Repository, spec: &str) -> Result<Oid> {
    let obj = repo
        .revparse_single(spec)
        .with_context(|| format!("cannot resolve revision `{spec}`"))?;
    Ok(obj.peel_to_commit()?.id())
}

/// Appends a line number, extending the last range when contiguous.
fn push_line(ranges: &mut Vec<LineRange>, line: u32) {
    if let Some(last) = ranges.last_mut()
        && last.end + 1 == line
    {
        last.end = line;
        return;
    }
    ranges.push(LineRange::new(line, line));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn extracts_line_ranges() {
        let dir = std::env::temp_dir().join(format!("hermit-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.py"), "a = 1\nb = 2\nc = 3\nd = 4\n").unwrap();
        std::fs::write(dir.join("gone.py"), "x = 1\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "base"]);
        git(&dir, &["checkout", "-qb", "feature"]);
        std::fs::write(dir.join("a.py"), "a = 1\nB = 2\nC = 3\nd = 4\ne = 5\n").unwrap();
        std::fs::remove_file(dir.join("gone.py")).unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "head"]);

        let cs = ChangeSet::open(&dir, "main", "feature").unwrap();
        let files = cs.changed_files().unwrap();
        let a = files.iter().find(|f| f.path == Path::new("a.py")).unwrap();
        assert_eq!(a.status, FileStatus::Modified);
        assert_eq!(a.added, vec![LineRange::new(2, 3), LineRange::new(5, 5)]);
        assert_eq!(a.removed, vec![LineRange::new(2, 3)]);
        assert_eq!((a.additions, a.deletions), (3, 2));
        let gone = files
            .iter()
            .find(|f| f.path == Path::new("gone.py"))
            .unwrap();
        assert_eq!(gone.status, FileStatus::Deleted);
        assert_eq!(cs.read_head(Path::new("gone.py")).unwrap(), None);
        assert!(cs.read_base(Path::new("gone.py")).unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
