use std::env;

use nokin::workspace::Workspace;

fn main() {
    let path = match env::args_os().nth(1) {
        Some(path) => Some(path.into()),
        None => match nokin::ui::choose_workspace() {
            Ok(path) => path,
            Err(error) => {
                eprintln!("nokin: {error}");
                std::process::exit(1);
            }
        },
    };
    let Some(path) = path else {
        return;
    };
    let workspace = match Workspace::from_optional_path(Some(path)) {
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
