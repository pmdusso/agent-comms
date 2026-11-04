use crate::error::{Code, CommsError, Result};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub fn git_main_root(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let marker = dir.join(".git");
        if marker.is_dir() {
            return dir.canonicalize().ok();
        }
        if !marker.is_file() {
            continue;
        }
        let content = fs::read_to_string(&marker).ok()?;
        let line = content.lines().next()?;
        let value = line.strip_prefix("gitdir:")?.trim();
        let raw = PathBuf::from(value);
        let gitdir = if raw.is_absolute() { raw } else { dir.join(raw) };
        let commondir = gitdir.join("commondir");
        if commondir.exists() {
            let common = fs::read_to_string(commondir).ok()?;
            return fs::canonicalize(gitdir.join(common.trim()))
                .ok()?
                .parent()
                .map(Path::to_path_buf);
        }
        return dir.canonicalize().ok();
    }
    None
}

#[derive(Debug)]
pub struct Ctx {
    pub cwd: PathBuf,
    pub git_root: Option<PathBuf>,
    pub db_path: Option<PathBuf>,
    pub db_from_env: bool,
}

impl Ctx {
    pub fn discover() -> Result<Self> {
        let cwd = env::current_dir()?.canonicalize()?;
        let env_db = env::var_os("COMMS_DB").filter(|value| !value.is_empty());
        Ok(Self::discover_from(&cwd, env_db.as_deref()))
    }

    pub fn discover_from(cwd: &Path, comms_db: Option<&OsStr>) -> Self {
        let git_root = git_main_root(cwd);
        if let Some(value) = comms_db.filter(|value| !value.is_empty()) {
            let raw = PathBuf::from(value);
            let db_path = if raw.is_absolute() { raw } else { cwd.join(raw) };
            return Self { cwd: cwd.to_path_buf(), git_root, db_path: Some(db_path), db_from_env: true };
        }

        let start = git_root.as_deref().unwrap_or(cwd);
        let db_path = start
            .ancestors()
            .map(|dir| dir.join("comms.db"))
            .find(|path| path.is_file());
        Self { cwd: cwd.to_path_buf(), git_root, db_path, db_from_env: false }
    }

    pub fn require_db(&self) -> Result<&Path> {
        self.db_path.as_deref().ok_or_else(|| {
            CommsError::new(
                Code::DbMissing,
                format!("ERRO: comms.db não encontrado a partir de '{}' (raiz git e diretórios acima). Rode 'comms bootstrap' na raiz do repositório ou defina COMMS_DB.", self.cwd.display()),
            )
        })
    }

    pub fn home(&self) -> Option<&Path> {
        self.db_path.as_deref().and_then(Path::parent)
    }

    pub fn prompts_dir(&self) -> Option<PathBuf> {
        self.home().map(|home| home.join("docs/prompts"))
    }

    pub fn project_root(&self) -> &Path {
        self.git_root.as_deref().or_else(|| self.home()).unwrap_or(&self.cwd)
    }

    pub fn bootstrap_db(&self) -> PathBuf {
        self.db_path.clone().filter(|_| self.db_from_env).unwrap_or_else(|| {
            self.git_root.as_ref().unwrap_or(&self.cwd).join("comms.db")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Ctx, git_main_root};
    use std::ffi::OsStr;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn submodule_git_file_keeps_submodule_as_root() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("submodule");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(".git"), "gitdir: ../.git/modules/submodule\n").unwrap();

        assert_eq!(git_main_root(&root), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn relative_env_db_is_anchored_at_supplied_cwd() {
        let tmp = tempdir().unwrap();
        let cwd = tmp.path().join("cwd");
        fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.canonicalize().unwrap();

        let ctx = Ctx::discover_from(&cwd, Some(OsStr::new("state/comms.db")));

        assert_eq!(ctx.db_path, Some(cwd.join("state/comms.db")));
        assert!(ctx.db_from_env);
    }
}
