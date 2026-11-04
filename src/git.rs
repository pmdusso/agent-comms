use std::path::Path;
use std::process::Command;

fn git_output(root: &Path, args: &[&str]) -> Option<std::process::Output> {
    Command::new("git").args(args).current_dir(root).output().ok()
}

pub fn default_branch(root: &Path) -> String {
    if let Some(output) = git_output(root, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        && output.status.success()
    {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() {
            return branch.split_once('/').map(|(_, branch)| branch).unwrap_or(&branch).to_string();
        }
    }
    for candidate in ["main", "master", "dev"] {
        if git_output(root, &["rev-parse", "--verify", "--quiet", candidate]).is_some_and(|output| output.status.success()) {
            return candidate.to_string();
        }
    }
    git_output(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|branch| !branch.is_empty())
        .unwrap_or_else(|| "main".to_string())
}

pub fn worktree_add(root: &Path, branch: &str, dir: &Path, base: &str) -> Result<(), String> {
    let output = Command::new("git")
        .args(["worktree", "add", "-b", branch])
        .arg(dir)
        .arg(base)
        .current_dir(root)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}
