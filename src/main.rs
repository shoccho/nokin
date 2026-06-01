use std::env;
use std::path::PathBuf;

use nokin::workspace::Workspace;

fn main() {
    let path = env::args_os().nth(1).map(PathBuf::from);
    let workspace = match Workspace::from_optional_path(path) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("nokin: {error}");
            std::process::exit(2);
        }
    };

    if let Err(error) = nokin::ui::run(&workspace) {
        eprintln!("nokin: {error}");
        std::process::exit(1);
    }
}
