use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::ProjectConfig;

const SKIP_DIRS: &[&str] = &["build", "target", "dist", "out", ".git", ".cache"];

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub initial_file: Option<PathBuf>,
    pub config: ProjectConfig,
}

impl Workspace {
    pub fn from_optional_path(path: Option<PathBuf>) -> io::Result<Self> {
        let path = path.unwrap_or(std::env::current_dir()?);
        let path = fs::canonicalize(path)?;
        let (root, initial_file) = if path.is_dir() {
            (path, None)
        } else {
            let root = path
                .parent()
                .ok_or_else(|| io::Error::other("file has no parent directory"))?
                .to_path_buf();
            (root, Some(path))
        };
        let config = ProjectConfig::load(&root)?;
        Ok(Self {
            root,
            initial_file,
            config,
        })
    }

    pub fn c_files(&self) -> io::Result<Vec<PathBuf>> {
        walk_c_files(&self.root)
    }

    pub fn rs_files(&self) -> io::Result<Vec<PathBuf>> {
        walk_files(&self.root, "rs")
    }

    pub fn include_roots(&self) -> Vec<PathBuf> {
        self.config
            .include_dirs
            .iter()
            .map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    self.root.join(path)
                }
            })
            .collect()
    }
}

pub fn walk_c_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_dir() {
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_ref()) {
                    pending.push(path);
                }
            } else if is_c_file(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

pub fn walk_files(root: &Path, extension: &str) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_dir() {
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_ref()) {
                    pending.push(path);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some(extension) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

pub fn reachable_includes(
    entry: &Path,
    project_root: &Path,
    include_roots: &[PathBuf],
) -> io::Result<BTreeSet<PathBuf>> {
    let mut found = BTreeSet::new();
    let mut pending = VecDeque::from([entry.to_path_buf()]);
    while let Some(file) = pending.pop_front() {
        let contents = fs::read_to_string(&file)?;
        for include in parse_includes(&contents) {
            let mut roots = Vec::new();
            if let Some(parent) = file.parent() {
                roots.push(parent.to_path_buf());
            }
            roots.push(project_root.to_path_buf());
            roots.extend_from_slice(include_roots);
            if let Some(path) = roots
                .into_iter()
                .map(|root| root.join(&include))
                .find(|path| path.is_file())
                .and_then(|path| fs::canonicalize(path).ok())
                && found.insert(path.clone())
            {
                pending.push_back(path);
            }
        }
    }
    Ok(found)
}

pub fn parse_includes(contents: &str) -> Vec<PathBuf> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("#include")?.trim();
            let rest = rest
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .or_else(|| {
                    rest.strip_prefix('<')
                        .and_then(|rest| rest.strip_suffix('>'))
                })?;
            Some(PathBuf::from(rest))
        })
        .collect()
}

fn is_c_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("c" | "h")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_include_forms() {
        assert_eq!(
            parse_includes("#include \"local.h\"\n #include <stdio.h>\n#define X 1"),
            vec![PathBuf::from("local.h"), PathBuf::from("stdio.h")]
        );
    }

    #[test]
    fn follows_only_reachable_headers() {
        let root = temporary_directory();
        fs::create_dir(root.join("include")).unwrap();
        fs::write(root.join("main.c"), "#include \"a.h\"\n").unwrap();
        fs::write(root.join("include/a.h"), "#include \"b.h\"\n").unwrap();
        fs::write(root.join("include/b.h"), "int b;\n").unwrap();
        fs::write(root.join("include/unreferenced.h"), "int nope;\n").unwrap();
        let headers =
            reachable_includes(&root.join("main.c"), &root, &[root.join("include")]).unwrap();
        assert_eq!(headers.len(), 2);
        assert!(headers.iter().any(|path| path.ends_with("a.h")));
        assert!(headers.iter().any(|path| path.ends_with("b.h")));
        fs::remove_dir_all(root).unwrap();
    }

    fn temporary_directory() -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("nokin-test-{id}"));
        fs::create_dir(&path).unwrap();
        path
    }
}
