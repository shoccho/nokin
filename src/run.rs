use std::path::Path;

use crate::config::ProjectConfig;

pub fn command_for(
    config: &ProjectConfig,
    workspace: &Path,
    file: Option<&Path>,
) -> Option<String> {
    let template = file
        .and_then(|path| path.extension()?.to_str())
        .and_then(|extension| config.command_for_extension(extension))
        .or(config.workspace_command.as_deref())?;
    Some(expand_placeholders(template, workspace, file))
}

pub fn expand_placeholders(template: &str, workspace: &Path, file: Option<&Path>) -> String {
    let workspace = shell_escape(&absolute(workspace));
    let file = file.map(absolute);
    let file_dir = file.as_deref().and_then(Path::parent).map(shell_escape);
    let file = file.as_deref().map(shell_escape);
    template
        .replace("${workspace}", &workspace)
        .replace("${file_dir}", file_dir.as_deref().unwrap_or(""))
        .replace("${file}", file.as_deref().unwrap_or(""))
}

pub fn shell_escape(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}

fn absolute(path: &Path) -> &Path {
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn quotes_shell_metacharacters_and_single_quotes() {
        assert_eq!(
            shell_escape(Path::new("/tmp/a file's.c")),
            "'/tmp/a file'\"'\"'s.c'"
        );
    }

    #[test]
    fn file_command_takes_precedence_and_expands_paths() {
        let config = ProjectConfig::parse(
            "[run]\nworkspace = \"make run\"\n[run.files]\nc = \"cc ${file} -o ${file_dir}/a.out\"\n",
        );
        assert_eq!(
            command_for(
                &config,
                Path::new("/work/demo"),
                Some(Path::new("/work/demo/src/a file.c"))
            ),
            Some("cc '/work/demo/src/a file.c' -o '/work/demo/src'/a.out".into())
        );
    }

    #[test]
    fn workspace_command_is_fallback() {
        let config = ProjectConfig {
            workspace_command: Some("cd ${workspace} && make run".into()),
            ..ProjectConfig::default()
        };
        assert_eq!(
            command_for(&config, &PathBuf::from("/work/demo"), None),
            Some("cd '/work/demo' && make run".into())
        );
    }
}
