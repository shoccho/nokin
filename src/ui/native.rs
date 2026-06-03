use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(not(unix))]
use std::process::ChildStdin;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
#[cfg(unix)]
use std::{
    os::fd::{AsRawFd, FromRawFd},
    os::unix::process::CommandExt,
};

use floem::action::{exec_after, open_file, save_as};
use floem::event::{Event, EventListener, EventPropagation};
use floem::file::FileDialogOptions;
use floem::keyboard::{Key, Modifiers, NamedKey};
use floem::menu::{Menu, MenuItem};
use floem::prelude::*;
use floem::text::{Attrs, AttrsList, FamilyOwned};
use floem::views::editor::command::{Command as EditorCommand, CommandExecuted};
use floem::views::editor::id::EditorId;
use floem::views::editor::keypress::default_key_handler;
use floem::views::editor::keypress::press::KeyPress;
use floem::views::editor::text::{Styling, WrapMethod};
use floem::views::editor::{Editor, EditorStyle};
use floem_editor_core::command::{EditCommand, MultiSelectionCommand};
use floem_editor_core::cursor::CursorMode;
use floem_editor_core::selection::{SelRegion, Selection};

use crate::config::Settings;
use crate::lsp::{self, Manager, ServerCommands};
use crate::theme::{Palette, UiPalette};
use crate::workspace::Workspace;

const SKIP_DIRS: &[&str] = &["build", "target", "dist", "out", ".git", ".cache"];
const TERMINAL_OUTPUT_LIMIT: usize = 128 * 1024;

struct NativeTerminal {
    child: Child,
    #[cfg(unix)]
    input: Arc<Mutex<File>>,
    #[cfg(not(unix))]
    input: Arc<Mutex<ChildStdin>>,
    pending_output: Arc<Mutex<String>>,
    running: Arc<AtomicBool>,
}

impl NativeTerminal {
    fn spawn(root: &Path, shell: &str) -> io::Result<Self> {
        #[cfg(unix)]
        {
            Self::spawn_unix(root, shell)
        }
        #[cfg(not(unix))]
        {
            Self::spawn_piped(root, shell)
        }
    }

    #[cfg(unix)]
    fn spawn_unix(root: &Path, shell: &str) -> io::Result<Self> {
        let (master, slave) = open_pty()?;
        let slave_fd = slave.as_raw_fd();
        let mut command = Command::new(shell);
        command
            .current_dir(root)
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));

        // SAFETY: this runs in the child just before exec. It only calls async-signal-safe
        // libc functions to make the slave side of the PTY the controlling terminal.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command.spawn()?;
        let input = master.try_clone()?;
        let pending_output = Arc::new(Mutex::new(String::new()));
        let running = Arc::new(AtomicBool::new(true));
        read_terminal_output(master, pending_output.clone());
        Ok(Self {
            child,
            input: Arc::new(Mutex::new(input)),
            pending_output,
            running,
        })
    }

    #[cfg(not(unix))]
    fn spawn_piped(root: &Path, shell: &str) -> io::Result<Self> {
        let mut command = Command::new(shell);
        command
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(not(target_os = "windows"))]
        command.arg("-i");

        let mut child = command.spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("terminal shell stdin is unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("terminal shell stdout is unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("terminal shell stderr is unavailable"))?;
        let pending_output = Arc::new(Mutex::new(String::new()));
        let running = Arc::new(AtomicBool::new(true));
        read_terminal_output(stdout, pending_output.clone());
        read_terminal_output(stderr, pending_output.clone());
        Ok(Self {
            child,
            input: Arc::new(Mutex::new(input)),
            pending_output,
            running,
        })
    }

    fn send_command(&self, command: &str) -> io::Result<()> {
        self.send_bytes(command.as_bytes())?;
        self.send_bytes(b"\n")
    }

    fn send_bytes(&self, bytes: &[u8]) -> io::Result<()> {
        let mut input = self
            .input
            .lock()
            .map_err(|_| io::Error::other("terminal shell input lock is poisoned"))?;
        input.write_all(bytes)?;
        input.flush()
    }
}

impl Drop for NativeTerminal {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_terminal_output(mut reader: impl Read + Send + 'static, output: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut buffer = [0; 4096];
        let mut parser = TerminalOutputParser::default();
        while let Ok(size) = reader.read(&mut buffer) {
            if size == 0 {
                break;
            }
            let chunk = parser.clean(&buffer[..size]);
            if let Ok(mut output) = output.lock() {
                output.push_str(&chunk);
            }
        }
    });
}

#[cfg(unix)]
fn open_pty() -> io::Result<(File, File)> {
    let mut master = 0;
    let mut slave = 0;
    // SAFETY: openpty initializes the provided fd slots on success. The returned fds are
    // immediately wrapped in File so ownership is closed exactly once.
    let status = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty returned valid, owned fds.
    Ok(unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) })
}

#[derive(Default)]
struct TerminalOutputParser {
    escape: EscapeState,
}

#[derive(Default)]
enum EscapeState {
    #[default]
    None,
    Esc,
    Csi,
    Osc,
    OscEsc,
}

impl TerminalOutputParser {
    fn clean(&mut self, bytes: &[u8]) -> String {
        let mut output = Vec::new();
        for byte in bytes {
            match self.escape {
                EscapeState::None => match *byte {
                    0x1b => self.escape = EscapeState::Esc,
                    b'\r' => {}
                    0x08 => {
                        output.pop();
                    }
                    byte if byte >= 0x20 || byte == b'\n' || byte == b'\t' => {
                        output.push(byte);
                    }
                    _ => {}
                },
                EscapeState::Esc => match *byte {
                    b'[' => self.escape = EscapeState::Csi,
                    b']' => self.escape = EscapeState::Osc,
                    _ => self.escape = EscapeState::None,
                },
                EscapeState::Csi => {
                    if (0x40..=0x7e).contains(byte) {
                        self.escape = EscapeState::None;
                    }
                }
                EscapeState::Osc => match *byte {
                    0x07 => self.escape = EscapeState::None,
                    0x1b => self.escape = EscapeState::OscEsc,
                    _ => {}
                },
                EscapeState::OscEsc => {
                    self.escape = EscapeState::None;
                }
            }
        }
        String::from_utf8_lossy(&output).into_owned()
    }
}

fn poll_terminal_output(
    transcript: RwSignal<String>,
    pending_output: Arc<Mutex<String>>,
    running: Arc<AtomicBool>,
) {
    exec_after(Duration::from_millis(50), move |_| {
        if let Ok(mut pending) = pending_output.lock()
            && !pending.is_empty()
        {
            let chunk = std::mem::take(&mut *pending);
            transcript.update(|transcript| {
                transcript.push_str(&chunk);
                trim_terminal_output(transcript);
            });
        }
        if running.load(Ordering::Relaxed) {
            poll_terminal_output(transcript, pending_output, running);
        }
    });
}

fn trim_terminal_output(output: &mut String) {
    if output.len() <= TERMINAL_OUTPUT_LIMIT {
        return;
    }
    let mut start = output.len() - TERMINAL_OUTPUT_LIMIT;
    while !output.is_char_boundary(start) {
        start += 1;
    }
    output.drain(..start);
}

fn terminal_key_bytes(event: &floem::keyboard::KeyEvent) -> Option<Vec<u8>> {
    let key = &event.key.logical_key;
    if event.modifiers.control() {
        if let Key::Character(character) = key {
            let character = character.as_str().bytes().next()?;
            let character = character.to_ascii_uppercase();
            if character.is_ascii_uppercase()
                || matches!(character, b'[' | b'\\' | b']' | b'^' | b'_')
            {
                return Some(vec![character & 0x1f]);
            }
        }
        return None;
    }
    if event.modifiers.alt() || event.modifiers.meta() {
        return None;
    }
    match key {
        Key::Character(character) => Some(character.as_bytes().to_vec()),
        Key::Named(NamedKey::Enter) => Some(b"\r".to_vec()),
        Key::Named(NamedKey::Tab) => Some(b"\t".to_vec()),
        Key::Named(NamedKey::Backspace) => Some(vec![0x7f]),
        Key::Named(NamedKey::Delete) => Some(b"\x1b[3~".to_vec()),
        Key::Named(NamedKey::Escape) => Some(vec![0x1b]),
        Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A".to_vec()),
        Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B".to_vec()),
        Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C".to_vec()),
        Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D".to_vec()),
        Key::Named(NamedKey::Home) => Some(b"\x1b[H".to_vec()),
        Key::Named(NamedKey::End) => Some(b"\x1b[F".to_vec()),
        Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~".to_vec()),
        Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~".to_vec()),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeDocument {
    id: usize,
    path: Option<PathBuf>,
    title: String,
    contents: String,
    dirty: bool,
}

impl NativeDocument {
    fn untitled(id: usize) -> Self {
        Self {
            id,
            path: None,
            title: "Untitled".into(),
            contents: String::new(),
            dirty: false,
        }
    }

    fn open(id: usize, path: &Path) -> io::Result<Self> {
        let title = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Ok(Self {
            id,
            path: Some(path.to_path_buf()),
            title,
            contents: fs::read_to_string(path)?,
            dirty: false,
        })
    }
}

struct NativeState {
    root: PathBuf,
    tabs: Vec<NativeDocument>,
    active: usize,
    next_id: usize,
    current_editor: Option<Editor>,
    pending_cursor: Option<(PathBuf, usize, usize)>,
    lsp: Manager,
}

impl NativeState {
    fn new(root: PathBuf, document: NativeDocument, settings: &Settings) -> Self {
        Self {
            lsp: Manager::new(
                &root,
                ServerCommands {
                    clangd: settings.lsp.clangd.clone(),
                    rust_analyzer: settings.lsp.rust_analyzer.clone(),
                },
            ),
            root,
            tabs: vec![document],
            active: 0,
            next_id: 1,
            current_editor: None,
            pending_cursor: None,
        }
    }

    fn active(&self) -> &NativeDocument {
        &self.tabs[self.active]
    }

    fn active_mut(&mut self) -> &mut NativeDocument {
        &mut self.tabs[self.active]
    }

    fn sync_editor(&mut self) {
        if let Some(contents) = self
            .current_editor
            .as_ref()
            .map(|editor| editor.text().to_string())
        {
            self.active_mut().contents = contents;
        }
    }

    fn open_path(&mut self, path: &Path) -> io::Result<()> {
        self.sync_editor();
        if let Some(index) = self
            .tabs
            .iter()
            .position(|document| document.path.as_deref() == Some(path))
        {
            self.active = index;
        } else {
            let document = NativeDocument::open(self.next_id, path)?;
            self.next_id += 1;
            self.tabs.push(document);
            self.active = self.tabs.len() - 1;
        }
        self.current_editor = None;
        Ok(())
    }

    fn open_folder(&mut self, path: PathBuf, settings: &Settings) {
        self.sync_editor();
        self.root = path;
        self.lsp = Manager::new(
            &self.root,
            ServerCommands {
                clangd: settings.lsp.clangd.clone(),
                rust_analyzer: settings.lsp.rust_analyzer.clone(),
            },
        );
        if settings.workspace.close_tabs_on_folder_open {
            self.tabs.clear();
            self.tabs.push(NativeDocument::untitled(self.next_id));
            self.next_id += 1;
            self.active = 0;
        }
        self.current_editor = None;
        self.pending_cursor = None;
    }

    fn new_tab(&mut self) {
        self.sync_editor();
        let document = NativeDocument::untitled(self.next_id);
        self.next_id += 1;
        self.tabs.push(document);
        self.active = self.tabs.len() - 1;
        self.current_editor = None;
    }

    fn activate(&mut self, id: usize) {
        self.sync_editor();
        if let Some(index) = self.tabs.iter().position(|document| document.id == id) {
            self.active = index;
            self.current_editor = None;
        }
    }

    fn close(&mut self, id: usize) -> bool {
        let Some(index) = self.tabs.iter().position(|document| document.id == id) else {
            return false;
        };
        if self.tabs[index].dirty {
            return false;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            let document = NativeDocument::untitled(self.next_id);
            self.next_id += 1;
            self.tabs.push(document);
            self.active = 0;
            self.current_editor = None;
        } else {
            if index < self.active {
                self.active -= 1;
            } else {
                self.active = self.active.min(self.tabs.len() - 1);
            }
            self.current_editor = None;
        }
        true
    }

    fn update_text(&mut self, text: String) {
        let path = self.active().path.clone();
        self.active_mut().contents = text.clone();
        self.active_mut().dirty = true;
        if let Some(path) = path {
            let _ = self.lsp.sync(&path, &text);
        }
    }

    fn save_to(&mut self, path: &Path) -> io::Result<()> {
        self.sync_editor();
        let contents = self.active().contents.clone();
        save_text(path, &contents)?;
        let active = self.active_mut();
        active.path = Some(path.to_path_buf());
        active.title = file_title(path);
        active.dirty = false;
        Ok(())
    }

    fn lsp_context(&mut self) -> io::Result<Option<(PathBuf, String, usize, usize)>> {
        self.sync_editor();
        let Some(path) = self.active().path.clone() else {
            return Ok(None);
        };
        let source = self.active().contents.clone();
        let offset = self
            .current_editor
            .as_ref()
            .map(|editor| editor.cursor.get_untracked().offset())
            .unwrap_or(0);
        let (line, column) = line_column(&source, offset);
        Ok(Some((path, source, line, column)))
    }

    fn lsp_definition(&mut self) -> io::Result<String> {
        let Some((file, source, line, column)) = self.lsp_context()? else {
            return Ok("Save the file before requesting a definition".into());
        };
        let Some(location) = self.lsp.definition(&file, &source, line, column)? else {
            return Ok("Definition not found".into());
        };
        self.open_path(&location.file)?;
        self.pending_cursor = Some((location.file.clone(), location.line, location.column));
        Ok(format!(
            "Definition: {}:{}:{}",
            location.file.display(),
            location.line + 1,
            location.column + 1
        ))
    }

    fn lsp_hover(&mut self) -> io::Result<String> {
        let Some((file, source, line, column)) = self.lsp_context()? else {
            return Ok("Save the file before requesting hover information".into());
        };
        Ok(self
            .lsp
            .hover(&file, &source, line, column)?
            .unwrap_or_else(|| "No hover information available".into()))
    }

    fn lsp_completion(&mut self) -> io::Result<String> {
        let Some((file, source, line, column)) = self.lsp_context()? else {
            return Ok("Save the file before requesting completion".into());
        };
        let entries = self.lsp.completion(&file, &source, line, column)?;
        Ok(if entries.is_empty() {
            "No completion candidates".into()
        } else {
            entries.join("\n")
        })
    }

    fn lsp_signature_help(&mut self) -> io::Result<String> {
        let Some((file, source, line, column)) = self.lsp_context()? else {
            return Ok("Save the file before requesting signature help".into());
        };
        Ok(self
            .lsp
            .signature_help(&file, &source, line, column)?
            .unwrap_or_else(|| "No signature help available".into()))
    }

    fn lsp_references(&mut self) -> io::Result<String> {
        let Some((file, source, line, column)) = self.lsp_context()? else {
            return Ok("Save the file before requesting references".into());
        };
        let references = self.lsp.references(&file, &source, line, column)?;
        Ok(if references.is_empty() {
            "No references found".into()
        } else {
            references
                .iter()
                .map(|location| {
                    format!(
                        "{}:{}:{}",
                        location.file.display(),
                        location.line + 1,
                        location.column + 1
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    fn lsp_diagnostics(&mut self) -> io::Result<String> {
        let Some((file, source, _, _)) = self.lsp_context()? else {
            return Ok("Save the file before requesting diagnostics".into());
        };
        self.lsp.sync(&file, &source)?;
        let diagnostics = self.lsp.diagnostics(&file);
        Ok(if diagnostics.is_empty() {
            "No diagnostics".into()
        } else {
            diagnostics
                .iter()
                .map(|diagnostic| {
                    format!(
                        "{}:{}: {}",
                        diagnostic.line + 1,
                        diagnostic.column + 1,
                        diagnostic.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    fn lsp_format(&mut self) -> io::Result<String> {
        let Some((file, source, _, _)) = self.lsp_context()? else {
            return Ok("Save the file before formatting".into());
        };
        let edits = self.lsp.formatting(&file, &source)?;
        if edits.is_empty() {
            return Ok("No formatting changes".into());
        }
        let updated = lsp::apply_text_edits(&source, &edits);
        let active = self.active_mut();
        active.contents = updated.clone();
        active.dirty = true;
        self.current_editor = None;
        let _ = self.lsp.sync(&file, &updated);
        Ok(format!("Applied {} formatting edits", edits.len()))
    }

    fn take_pending_cursor(&mut self, document: &NativeDocument) -> Option<usize> {
        let (path, line, column) = self.pending_cursor.take()?;
        (document.path.as_ref() == Some(&path))
            .then(|| line_column_offset(&document.contents, line, column))
            .flatten()
    }
}

pub fn choose_workspace() -> io::Result<Option<PathBuf>> {
    Ok(rfd::FileDialog::new()
        .set_title("Open Workspace")
        .pick_folder())
}

pub fn run(workspace: &Workspace) -> io::Result<()> {
    let root = workspace.root.clone();
    let files = workspace_files(&root)?;
    let settings = Settings::load()?;
    let document = workspace
        .initial_file
        .as_deref()
        .map(|path| NativeDocument::open(0, path))
        .transpose()?
        .unwrap_or_else(|| NativeDocument::untitled(0));
    let palette = crate::theme::load(&settings.editor.theme);
    floem::launch(move || app_view(root, files, document, settings, palette));
    Ok(())
}

fn app_view(
    root: PathBuf,
    files: Vec<PathBuf>,
    document: NativeDocument,
    settings: Settings,
    palette: Palette,
) -> impl IntoView {
    let state = Rc::new(RefCell::new(NativeState::new(
        root.clone(),
        document,
        &settings,
    )));
    let revision = create_rw_signal(0_u64);
    let selection_revision = create_rw_signal(0_u64);
    let status = create_rw_signal(String::new());
    let sidebar_visible = create_rw_signal(true);
    let terminal_visible = create_rw_signal(true);
    let find_visible = create_rw_signal(false);
    let find_query = create_rw_signal(String::new());
    let expanded_sidebar = create_rw_signal(BTreeSet::from([root.clone()]));
    let sidebar_rows_signal = create_rw_signal(
        sidebar_rows(&root, &expanded_sidebar.get_untracked()).unwrap_or_else(|_| {
            files
                .into_iter()
                .map(|path| SidebarRow::file(path, 0))
                .collect()
        }),
    );
    let terminal = match NativeTerminal::spawn(&root, &settings.terminal.shell) {
        Ok(terminal) => Some(Rc::new(terminal)),
        Err(error) => {
            status.set(format!("Terminal unavailable: {error}"));
            None
        }
    };
    let terminal_transcript = create_rw_signal(String::new());
    let terminal_shell_label = Path::new(&settings.terminal.shell)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| settings.terminal.shell.clone());
    if let Some(terminal) = &terminal {
        poll_terminal_output(
            terminal_transcript,
            terminal.pending_output.clone(),
            terminal.running.clone(),
        );
    } else {
        terminal_transcript.set("Unable to start the configured terminal shell.".into());
    }
    let ui = UiPalette::from(&palette);
    let terminal_bg = palette.default_bg;
    let terminal_font_family = settings.editor.font_family.clone();
    let terminal_font_size = settings.editor.font_size;
    let file_state = state.clone();
    let file_settings = settings.clone();
    let file = button("File")
        .popout_menu(move || {
            file_menu(
                file_state.clone(),
                file_settings.clone(),
                sidebar_rows_signal,
                expanded_sidebar,
                revision,
                selection_revision,
                status,
            )
        })
        .style(move |style| menu_button(style, ui));
    let edit_state = state.clone();
    let edit = button("Edit")
        .popout_menu(move || edit_menu(edit_state.clone(), revision, status))
        .style(move |style| menu_button(style, ui));
    let view = button("View")
        .popout_menu(move || view_menu(sidebar_visible, terminal_visible))
        .style(move |style| menu_button(style, ui));
    let build_state = state.clone();
    let build_terminal = terminal.clone();
    let build_root = root.clone();
    let build_config_state = state.clone();
    let build_config_root = root.clone();
    let build = button("Build")
        .popout_menu(move || {
            build_menu(
                build_state.clone(),
                build_terminal.clone(),
                status,
                build_root.clone(),
                revision,
                selection_revision,
                build_config_state.clone(),
                build_config_root.clone(),
            )
        })
        .style(move |style| menu_button(style, ui));
    let code_state = state.clone();
    let navigate = button("Navigate")
        .popout_menu(move || lsp_menu(code_state.clone(), revision, selection_revision, status))
        .style(move |style| menu_button(style, ui));

    let sidebar_state = state.clone();
    let sidebar = scroll(
        dyn_stack(
            move || sidebar_rows_signal.get(),
            |row| (row.path.clone(), row.is_expanded),
            move |row: SidebarRow| {
                let state = sidebar_state.clone();
                let rows = sidebar_rows_signal;
                let expanded = expanded_sidebar;
                let row_path = row.path.clone();
                let row_label = row.label.clone();
                let row_depth = row.depth;
                let row_is_dir = row.is_dir;
                let row_is_expanded = row.is_expanded;
                button(text(format!(
                    "{}{}",
                    if row_is_dir {
                        if row_is_expanded { "▾ " } else { "▸ " }
                    } else {
                        "  "
                    },
                    row_label
                )))
                .action(move || {
                    if row_is_dir {
                        expanded.update(|expanded| {
                            if !expanded.remove(&row_path) {
                                expanded.insert(row_path.clone());
                            }
                        });
                        let root = state.borrow().root.clone();
                        match sidebar_rows(&root, &expanded.get_untracked()) {
                            Ok(next_rows) => rows.set(next_rows),
                            Err(error) => status.set(format!("Refresh failed: {error}")),
                        }
                    } else {
                        match state.borrow_mut().open_path(&row_path) {
                            Ok(()) => status.set(String::new()),
                            Err(error) => status.set(format!("Open failed: {error}")),
                        }
                        revision.update(|value| *value += 1);
                        selection_revision.update(|value| *value += 1);
                    }
                })
                .style(move |style| {
                    style
                        .width_full()
                        .justify_start()
                        .padding_left(10.0 + (row_depth as f64 * 14.0))
                        .padding_right(8)
                        .padding_vert(4)
                        .border(0)
                        .border_radius(0)
                        .color(rgb(ui.foreground))
                        .background(rgb(ui.panel))
                        .hover(|style| style.background(rgb(ui.raised)))
                })
            },
        )
        .style(|style| style.width_full().flex_col()),
    )
    .style(move |style| style.width(230).height_full().background(rgb(ui.panel)));

    let tabs_state = state.clone();
    let tabs = dyn_stack(
        move || {
            revision.get();
            tabs_state.borrow().tabs.clone()
        },
        |document| document.id,
        {
            let state = state.clone();
            move |document: NativeDocument| {
                let id = document.id;
                let activate_state = state.clone();
                let close_state = state.clone();
                let title_state = state.clone();
                let active_state = state.clone();
                let close_active_state = state.clone();
                let tab_state = state.clone();
                let active_tab_bg = if tab_state.borrow().active().id == id {
                    ui.background
                } else {
                    ui.panel
                };
                h_stack((
                    button(label(move || {
                        revision.get();
                        title_state
                            .borrow()
                            .tabs
                            .iter()
                            .find(|document| document.id == id)
                            .map(tab_title)
                            .unwrap_or_default()
                    }))
                    .action(move || {
                        activate_state.borrow_mut().activate(id);
                        revision.update(|value| *value += 1);
                        selection_revision.update(|value| *value += 1);
                    })
                    .style(move |style| {
                        style
                            .height_full()
                            .padding_left(8)
                            .padding_right(2)
                            .border(0)
                            .border_radius(0)
                            .color(rgb(ui.foreground))
                            .background(rgb(if active_state.borrow().active().id == id {
                                ui.background
                            } else {
                                ui.panel
                            }))
                    }),
                    container(label(|| "×").style(|style| {
                        style
                            .height(14)
                            .line_height(1.0)
                            .font_size(13.0)
                            .padding_bottom(1)
                    }))
                    .on_click_stop(move |_| {
                        if !close_state.borrow_mut().close(id) {
                            status.set("Save changes before closing the tab".into());
                        }
                        revision.update(|value| *value += 1);
                        selection_revision.update(|value| *value += 1);
                    })
                    .style(move |style| {
                        revision.get();
                        let background = if close_active_state.borrow().active().id == id {
                            ui.background
                        } else {
                            ui.panel
                        };
                        style
                            .width(22)
                            .height(22)
                            .items_center()
                            .justify_center()
                            .border(0)
                            .border_radius(3)
                            .color(rgb(ui.foreground))
                            .background(rgb(background))
                            .hover(|style| style.color(rgb(ui.accent)).background(rgb(ui.raised)))
                    }),
                ))
                .style(move |style| {
                    revision.get();
                    let active = tab_state.borrow().active().id == id;
                    style
                        .height_full()
                        .gap(2)
                        .items_center()
                        .color(rgb(ui.foreground))
                        .background(rgb(if active { ui.background } else { active_tab_bg }))
                        .border_right(1)
                        .border_color(rgb(ui.border))
                })
            }
        },
    );

    let editor_state = state.clone();
    let dynamic_editor = dyn_container(
        move || {
            selection_revision.get();
            editor_state.borrow().active().clone()
        },
        {
            let state = state.clone();
            move |document: NativeDocument| {
                let styling = Rc::new(NativeStyling::new(
                    &document.contents,
                    palette.clone(),
                    &settings,
                ));
                let update_state = state.clone();
                let update_styling = styling.clone();
                let key_state = state.clone();
                let editor_sidebar_visible = sidebar_visible;
                let editor_terminal_visible = terminal_visible;
                let editor_find_visible = find_visible;
                let pending_cursor = state.borrow_mut().take_pending_cursor(&document);
                let view =
                    text_editor_keys(document.contents, move |editor, keypress, modifiers| {
                        if primary_shortcut(keypress, "b") {
                            editor_sidebar_visible.update(|visible| *visible = !*visible);
                            return CommandExecuted::Yes;
                        }
                        if primary_shortcut(keypress, "j") {
                            editor_terminal_visible.update(|visible| *visible = !*visible);
                            return CommandExecuted::Yes;
                        }
                        if primary_shortcut(keypress, "f") {
                            editor_find_visible.set(true);
                            return CommandExecuted::Yes;
                        }
                        if primary_shortcut(keypress, "d") {
                            let editor = editor.get_untracked();
                            select_next_occurrence(&editor, status);
                            return CommandExecuted::Yes;
                        }
                        if primary_shortcut(keypress, "s") {
                            save_current(key_state.clone(), revision, status);
                            return CommandExecuted::Yes;
                        }
                        if primary_shortcut(keypress, "o") {
                            open_file_dialog(
                                key_state.clone(),
                                revision,
                                selection_revision,
                                status,
                            );
                            return CommandExecuted::Yes;
                        }
                        if primary_shortcut(keypress, "n") {
                            key_state.borrow_mut().new_tab();
                            revision.update(|value| *value += 1);
                            selection_revision.update(|value| *value += 1);
                            return CommandExecuted::Yes;
                        }
                        default_key_handler(editor)(keypress, modifiers)
                    })
                    .styling_rc(styling)
                    .placeholder("Open a file to start editing")
                    .update(move |event| {
                        if let Some(editor) = event.editor {
                            let text = editor.text().to_string();
                            update_styling.update(&text);
                            update_state.borrow_mut().update_text(text);
                            revision.update(|value| *value += 1);
                        }
                    })
                    .editor_style(move |style| {
                        style
                            .selection_color(rgb(palette.selection_bg))
                            .cursor_color(rgb(palette.caret))
                            .current_line_color(rgb(palette.current_line_bg))
                            .gutter_accent_color(rgb(palette.default_fg))
                            .gutter_dim_color(rgb(palette.margin_fg))
                            .indent_guide_color(rgb(palette.margin_fg))
                            .indent_guide(true)
                            .smart_tab(true)
                            .wrap_method(WrapMethod::None)
                    })
                    .style(move |style| {
                        style
                            .size_full()
                            .color(rgb(palette.default_fg))
                            .background(rgb(palette.default_bg))
                    });
                let editor = view.editor().clone();
                if let Some(offset) = pending_cursor {
                    let mut cursor = editor.cursor.get_untracked();
                    cursor.set_insert(Selection::caret(offset));
                    editor.cursor.set(cursor);
                }
                state.borrow_mut().current_editor = Some(editor);
                view
            }
        },
    )
    .style(|style| style.size_full());

    let title_state = state.clone();
    let diagnostics_state = state.clone();
    let window_state = state.clone();
    let menu_bar = h_stack((file, edit, view, build, navigate)).style(move |style| {
        style
            .width_full()
            .height(32)
            .items_center()
            .font_size(14.0)
            .color(rgb(ui.foreground))
            .background(rgb(ui.panel))
            .border_bottom(1)
            .border_color(rgb(ui.border))
    });
    let sidebar_title_state = state.clone();
    let sidebar_panel = v_stack((
        label(move || {
            revision.get();
            let root = &sidebar_title_state.borrow().root;
            let root_label = root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string());
            format!("WORKSPACE  {root_label}")
        })
        .style(move |style| {
            style
                .width_full()
                .padding_horiz(10)
                .padding_vert(8)
                .font_size(11.0)
                .color(rgb(ui.foreground))
                .background(rgb(ui.raised))
        }),
        sidebar,
    ))
    .style(move |style| {
        let style = style
            .width(230)
            .height_full()
            .background(rgb(ui.panel))
            .border_right(1)
            .border_color(rgb(ui.border));
        if sidebar_visible.get() {
            style
        } else {
            style.hide()
        }
    });
    let find_state = state.clone();
    let find_status = status;
    let find_bar = dyn_container(
        move || find_visible.get(),
        move |visible| {
            if !visible {
                return empty().into_any();
            }
            let submit_state = find_state.clone();
            let close_query = find_query;
            h_stack((
                text("Find").style(move |style| {
                    style
                        .font_size(12.0)
                        .color(rgb(ui.foreground))
                        .padding_left(10)
                }),
                text_input(find_query)
                    .placeholder("Search active file")
                    .request_focus(|| {})
                    .on_event(EventListener::KeyDown, move |event| {
                        let Event::KeyDown(event) = event else {
                            return EventPropagation::Continue;
                        };
                        match event.key.logical_key {
                            Key::Named(NamedKey::Enter) => {
                                find_next_match(
                                    submit_state.clone(),
                                    find_query.get_untracked(),
                                    find_status,
                                );
                                EventPropagation::Stop
                            }
                            Key::Named(NamedKey::Escape) => {
                                find_visible.set(false);
                                EventPropagation::Stop
                            }
                            _ => EventPropagation::Continue,
                        }
                    })
                    .style(move |style| {
                        style
                            .height(24)
                            .width(260)
                            .padding_horiz(8)
                            .font_size(13.0)
                            .color(rgb(ui.foreground))
                            .background(rgb(ui.background))
                            .border(1)
                            .border_color(rgb(ui.border))
                            .border_radius(3)
                    }),
                button("×")
                    .action(move || {
                        close_query.set(String::new());
                        find_visible.set(false);
                    })
                    .style(move |style| {
                        style
                            .width(24)
                            .height(24)
                            .border(0)
                            .border_radius(3)
                            .color(rgb(ui.foreground))
                            .background(rgb(ui.panel))
                            .hover(|style| style.background(rgb(ui.raised)))
                    }),
            ))
            .style(move |style| {
                style
                    .width_full()
                    .height(36)
                    .gap(8)
                    .items_center()
                    .background(rgb(ui.panel))
                    .border_bottom(1)
                    .border_color(rgb(ui.border))
            })
            .into_any()
        },
    );
    let editor_stack = v_stack((
        tabs.style(move |style| {
            style
                .width_full()
                .height(34)
                .background(rgb(ui.panel))
                .border_bottom(1)
                .border_color(rgb(ui.border))
        }),
        find_bar,
        dynamic_editor,
    ))
    .style(|style| style.size_full());
    let terminal_running = terminal.as_ref().map(|terminal| terminal.running.clone());
    let terminal_keys = terminal.clone();
    let terminal_shortcut_state = state.clone();
    let terminal_shortcut_terminal = terminal.clone();
    let terminal_shortcut_root = root.clone();
    let terminal_title_font = terminal_font_family.clone();
    let terminal_detail_font = terminal_font_family.clone();
    let terminal_body_font = terminal_font_family.clone();
    let terminal_panel = v_stack((
        h_stack((
            text("TERMINAL").style(move |style| {
                style
                    .font_family(terminal_title_font.clone())
                    .font_size(11.0)
                    .color(rgb(ui.foreground))
                    .padding_horiz(10)
                    .padding_vert(6)
            }),
            text(format!("{}  {}", terminal_shell_label, root.display())).style(move |style| {
                style
                    .font_family(terminal_detail_font.clone())
                    .font_size(11.0)
                    .color(rgb(ui.foreground))
                    .padding_horiz(4)
                    .padding_vert(6)
            }),
        ))
        .style(move |style| {
            style
                .width_full()
                .items_center()
                .background(rgb(ui.panel))
                .border_bottom(1)
                .border_color(rgb(ui.border))
        }),
        scroll(
            label(move || terminal_transcript.get()).style(move |style| {
                style
                    .width_full()
                    .font_family(terminal_body_font.clone())
                    .font_size(terminal_font_size)
                    .line_height(1.35)
                    .color(rgb(ui.foreground))
                    .padding_horiz(12)
                    .padding_vert(10)
            }),
        )
        .scroll_to_percent(move || {
            terminal_transcript.get();
            100.0
        })
        .style(move |style| {
            style
                .width_full()
                .flex_grow(1.0)
                .background(rgb(terminal_bg))
        }),
    ))
    .keyboard_navigable()
    .on_event_stop(EventListener::KeyDown, move |event| {
        let Event::KeyDown(event) = event else {
            return;
        };
        if handle_app_shortcut(
            event,
            terminal_shortcut_state.clone(),
            revision,
            selection_revision,
            status,
            sidebar_visible,
            terminal_visible,
            find_visible,
            terminal_shortcut_terminal.clone(),
            terminal_shortcut_root.clone(),
            false,
        ) {
            return;
        }
        let Some(bytes) = terminal_key_bytes(event) else {
            return;
        };
        if let Some(terminal) = &terminal_keys {
            let _ = terminal.send_bytes(&bytes);
        }
    })
    .style(move |style| {
        let style = style
            .width_full()
            .height(230)
            .background(rgb(terminal_bg))
            .border_top(1)
            .border_color(rgb(ui.border))
            .focus(|style| style.border_color(rgb(ui.foreground)));
        if terminal_visible.get() {
            style
        } else {
            style.hide()
        }
    });
    let editor_area = v_stack((editor_stack, terminal_panel)).style(|style| style.size_full());
    let cleanup_terminal = terminal;
    let shortcut_state = state.clone();
    let shortcut_terminal = cleanup_terminal.clone();
    let shortcut_root = root.clone();
    v_stack((
        menu_bar,
        h_stack((sidebar_panel, editor_area)).style(|style| style.size_full()),
        h_stack((
            label(move || status.get()),
            label(move || {
                revision.get();
                let state = diagnostics_state.borrow();
                state
                    .active()
                    .path
                    .as_deref()
                    .map(|path| format!("{} diagnostics", state.lsp.diagnostics(path).len()))
                    .unwrap_or_default()
            }),
        ))
        .style(move |style| {
            style
                .width_full()
                .height(24)
                .gap(12)
                .padding_horiz(8)
                .items_center()
                .font_size(11.0)
                .color(rgb(ui.foreground))
                .background(rgb(ui.raised))
        }),
    ))
    .style(move |style| style.size_full().background(rgb(ui.background)))
    .on_event(EventListener::KeyDown, move |event| {
        let Event::KeyDown(event) = event else {
            return EventPropagation::Continue;
        };
        if handle_app_shortcut(
            event,
            shortcut_state.clone(),
            revision,
            selection_revision,
            status,
            sidebar_visible,
            terminal_visible,
            find_visible,
            shortcut_terminal.clone(),
            shortcut_root.clone(),
            true,
        ) {
            EventPropagation::Stop
        } else {
            EventPropagation::Continue
        }
    })
    .window_title(move || {
        revision.get();
        format!("Nokin - {}", tab_title(title_state.borrow().active()))
    })
    .on_cleanup(move || {
        window_state.borrow_mut().sync_editor();
        if let Some(running) = &terminal_running {
            running.store(false, Ordering::Relaxed);
        }
        let _ = cleanup_terminal.as_ref();
    })
}

fn open_file_dialog(
    state: Rc<RefCell<NativeState>>,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    status: RwSignal<String>,
) {
    let root = state.borrow().root.clone();
    open_file(
        FileDialogOptions::new()
            .title("Open File")
            .force_starting_directory(root),
        move |file| {
            let Some(path) = file.and_then(|file| file.path.into_iter().next()) else {
                return;
            };
            match state.borrow_mut().open_path(&path) {
                Ok(()) => status.set(String::new()),
                Err(error) => status.set(format!("Open failed: {error}")),
            }
            revision.update(|value| *value += 1);
            selection_revision.update(|value| *value += 1);
        },
    );
}

fn open_folder_dialog(
    state: Rc<RefCell<NativeState>>,
    settings: Settings,
    rows: RwSignal<Vec<SidebarRow>>,
    expanded: RwSignal<BTreeSet<PathBuf>>,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    status: RwSignal<String>,
) {
    let root = state.borrow().root.clone();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            status.set(format!("Open folder failed: {error}"));
            return;
        }
    };
    let Some(path) = runtime.block_on(async move {
        rfd::AsyncFileDialog::new()
            .set_title("Open Folder")
            .set_directory(root)
            .pick_folder()
            .await
            .map(|folder| folder.path().to_path_buf())
    }) else {
        return;
    };
    if !path.is_dir() {
        status.set(format!("Not a folder: {}", path.display()));
        return;
    }
    let mut next_expanded = BTreeSet::new();
    next_expanded.insert(path.clone());
    match sidebar_rows(&path, &next_expanded) {
        Ok(next_rows) => {
            state.borrow_mut().open_folder(path.clone(), &settings);
            expanded.set(next_expanded);
            rows.set(next_rows);
            status.set(format!("Opened folder {}", path.display()));
            revision.update(|value| *value += 1);
            selection_revision.update(|value| *value += 1);
        }
        Err(error) => status.set(format!("Open folder failed: {error}")),
    }
}

fn open_settings_file(
    state: Rc<RefCell<NativeState>>,
    settings: Settings,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    status: RwSignal<String>,
) {
    let Some(path) = Settings::file_path() else {
        status.set("Settings file path is unavailable on this platform".into());
        return;
    };
    if !path.exists() {
        if let Err(error) = settings.save() {
            status.set(format!("Create settings failed: {error}"));
            return;
        }
    }
    match state.borrow_mut().open_path(&path) {
        Ok(()) => status.set(format!("Opened settings {}", path.display())),
        Err(error) => status.set(format!("Open settings failed: {error}")),
    }
    revision.update(|value| *value += 1);
    selection_revision.update(|value| *value += 1);
}

fn save_current(
    state: Rc<RefCell<NativeState>>,
    revision: RwSignal<u64>,
    status: RwSignal<String>,
) {
    let path = state.borrow().active().path.clone();
    if let Some(path) = path {
        save_path(state, revision, status, path);
    } else {
        save_as_dialog(state, revision, status);
    }
}

fn save_as_dialog(
    state: Rc<RefCell<NativeState>>,
    revision: RwSignal<u64>,
    status: RwSignal<String>,
) {
    let (root, title) = {
        let state = state.borrow();
        (state.root.clone(), state.active().title.clone())
    };
    save_as(
        FileDialogOptions::new()
            .title("Save File")
            .default_name(title)
            .force_starting_directory(root),
        move |file| {
            if let Some(path) = file.and_then(|file| file.path.into_iter().next()) {
                save_path(state.clone(), revision, status, path);
            }
        },
    );
}

fn save_path(
    state: Rc<RefCell<NativeState>>,
    revision: RwSignal<u64>,
    status: RwSignal<String>,
    path: PathBuf,
) {
    match state.borrow_mut().save_to(&path) {
        Ok(()) => status.set(format!("Saved {}", path.display())),
        Err(error) => status.set(format!("Save failed: {error}")),
    }
    revision.update(|value| *value += 1);
}

#[allow(clippy::too_many_arguments)]
fn handle_app_shortcut(
    event: &floem::keyboard::KeyEvent,
    state: Rc<RefCell<NativeState>>,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    status: RwSignal<String>,
    sidebar_visible: RwSignal<bool>,
    terminal_visible: RwSignal<bool>,
    find_visible: RwSignal<bool>,
    terminal: Option<Rc<NativeTerminal>>,
    root: PathBuf,
    include_editor_shortcuts: bool,
) -> bool {
    if find_visible.get_untracked()
        && matches!(event.key.logical_key, Key::Named(NamedKey::Escape))
        && !event.modifiers.control()
        && !event.modifiers.meta()
        && !event.modifiers.alt()
    {
        find_visible.set(false);
        return true;
    }
    if event.modifiers.alt() || !(event.modifiers.control() || event.modifiers.meta()) {
        return matches!(event.key.logical_key, Key::Named(NamedKey::F5))
            .then(|| execute_active_file(state, terminal, status, root))
            .is_some();
    }
    let Some(key) = shortcut_character(event) else {
        return false;
    };
    match key {
        'n' => {
            state.borrow_mut().new_tab();
            revision.update(|value| *value += 1);
            selection_revision.update(|value| *value += 1);
        }
        'o' => open_file_dialog(state, revision, selection_revision, status),
        's' if event.modifiers.shift() => save_as_dialog(state, revision, status),
        's' => save_current(state, revision, status),
        'w' => close_current_tab(state, revision, selection_revision, status),
        'b' => sidebar_visible.update(|visible| *visible = !*visible),
        'j' => terminal_visible.update(|visible| *visible = !*visible),
        'f' => find_visible.set(true),
        '`' => terminal_visible.update(|visible| *visible = !*visible),
        'r' => execute_active_file(state, terminal, status, root),
        'd' if include_editor_shortcuts => {
            let Some(editor) = state.borrow().current_editor.clone() else {
                status.set("No active editor".into());
                return true;
            };
            select_next_occurrence(&editor, status);
        }
        'z' if include_editor_shortcuts && event.modifiers.shift() => run_editor_command(
            state,
            EditorCommand::Edit(EditCommand::Redo),
            true,
            revision,
            status,
        ),
        'z' if include_editor_shortcuts => run_editor_command(
            state,
            EditorCommand::Edit(EditCommand::Undo),
            true,
            revision,
            status,
        ),
        'y' if include_editor_shortcuts => run_editor_command(
            state,
            EditorCommand::Edit(EditCommand::Redo),
            true,
            revision,
            status,
        ),
        'x' if include_editor_shortcuts => run_editor_command(
            state,
            EditorCommand::Edit(EditCommand::ClipboardCut),
            true,
            revision,
            status,
        ),
        'c' if include_editor_shortcuts => run_editor_command(
            state,
            EditorCommand::Edit(EditCommand::ClipboardCopy),
            false,
            revision,
            status,
        ),
        'v' if include_editor_shortcuts => run_editor_command(
            state,
            EditorCommand::Edit(EditCommand::ClipboardPaste),
            true,
            revision,
            status,
        ),
        'a' if include_editor_shortcuts => run_editor_command(
            state,
            EditorCommand::MultiSelection(MultiSelectionCommand::SelectAll),
            false,
            revision,
            status,
        ),
        _ => return false,
    }
    true
}

fn shortcut_character(event: &floem::keyboard::KeyEvent) -> Option<char> {
    let Key::Character(character) = &event.key.logical_key else {
        return None;
    };
    character
        .chars()
        .next()
        .map(|character| character.to_ascii_lowercase())
}

fn close_current_tab(
    state: Rc<RefCell<NativeState>>,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    status: RwSignal<String>,
) {
    let id = state.borrow().active().id;
    if !state.borrow_mut().close(id) {
        status.set("Save changes before closing the tab".into());
    }
    revision.update(|value| *value += 1);
    selection_revision.update(|value| *value += 1);
}

fn run_lsp_action(
    state: &Rc<RefCell<NativeState>>,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    status: RwSignal<String>,
    rebuild_editor: bool,
    action: impl FnOnce(&mut NativeState) -> io::Result<String>,
) {
    let message = action(&mut state.borrow_mut())
        .unwrap_or_else(|error| format!("Language server unavailable: {error}"));
    status.set(message);
    revision.update(|value| *value += 1);
    if rebuild_editor {
        selection_revision.update(|value| *value += 1);
    }
}

fn file_menu(
    state: Rc<RefCell<NativeState>>,
    settings: Settings,
    rows: RwSignal<Vec<SidebarRow>>,
    expanded: RwSignal<BTreeSet<PathBuf>>,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    status: RwSignal<String>,
) -> Menu {
    let new_state = state.clone();
    let open_state = state.clone();
    let open_folder_state = state.clone();
    let save_state = state.clone();
    let settings_state = state.clone();
    let folder_settings = settings.clone();
    let settings_file_settings = settings.clone();
    Menu::new("File")
        .entry(MenuItem::new("New File").action(move || {
            new_state.borrow_mut().new_tab();
            revision.update(|value| *value += 1);
            selection_revision.update(|value| *value += 1);
        }))
        .entry(MenuItem::new("Open File...").action(move || {
            open_file_dialog(open_state.clone(), revision, selection_revision, status)
        }))
        .entry(MenuItem::new("Open Folder...").action(move || {
            open_folder_dialog(
                open_folder_state.clone(),
                folder_settings.clone(),
                rows,
                expanded,
                revision,
                selection_revision,
                status,
            )
        }))
        .separator()
        .entry(
            MenuItem::new("Save")
                .action(move || save_current(save_state.clone(), revision, status)),
        )
        .entry(
            MenuItem::new("Save As...")
                .action(move || save_as_dialog(state.clone(), revision, status)),
        )
        .separator()
        .entry(MenuItem::new("Settings").action(move || {
            open_settings_file(
                settings_state.clone(),
                settings_file_settings.clone(),
                revision,
                selection_revision,
                status,
            );
        }))
}

fn edit_menu(
    state: Rc<RefCell<NativeState>>,
    revision: RwSignal<u64>,
    status: RwSignal<String>,
) -> Menu {
    let undo_state = state.clone();
    let redo_state = state.clone();
    let cut_state = state.clone();
    let copy_state = state.clone();
    let paste_state = state.clone();
    Menu::new("Edit")
        .entry(MenuItem::new("Undo").action(move || {
            run_editor_command(
                undo_state.clone(),
                EditorCommand::Edit(EditCommand::Undo),
                true,
                revision,
                status,
            );
        }))
        .entry(MenuItem::new("Redo").action(move || {
            run_editor_command(
                redo_state.clone(),
                EditorCommand::Edit(EditCommand::Redo),
                true,
                revision,
                status,
            );
        }))
        .separator()
        .entry(MenuItem::new("Cut").action(move || {
            run_editor_command(
                cut_state.clone(),
                EditorCommand::Edit(EditCommand::ClipboardCut),
                true,
                revision,
                status,
            );
        }))
        .entry(MenuItem::new("Copy").action(move || {
            run_editor_command(
                copy_state.clone(),
                EditorCommand::Edit(EditCommand::ClipboardCopy),
                false,
                revision,
                status,
            );
        }))
        .entry(MenuItem::new("Paste").action(move || {
            run_editor_command(
                paste_state.clone(),
                EditorCommand::Edit(EditCommand::ClipboardPaste),
                true,
                revision,
                status,
            );
        }))
        .separator()
        .entry(MenuItem::new("Select All").action(move || {
            run_editor_command(
                state.clone(),
                EditorCommand::MultiSelection(MultiSelectionCommand::SelectAll),
                false,
                revision,
                status,
            );
        }))
}

fn run_editor_command(
    state: Rc<RefCell<NativeState>>,
    command: EditorCommand,
    mutates_text: bool,
    revision: RwSignal<u64>,
    status: RwSignal<String>,
) {
    let Some(editor) = state.borrow().current_editor.clone() else {
        status.set("No active editor".into());
        return;
    };
    let executed = editor
        .doc()
        .run_command(&editor, &command, Some(1), Modifiers::empty());
    if executed == CommandExecuted::No {
        status.set("Edit command unavailable".into());
        return;
    }
    if mutates_text {
        let text = editor.text().to_string();
        state.borrow_mut().update_text(text);
        revision.update(|value| *value += 1);
    }
}

fn select_next_occurrence(editor: &Editor, status: RwSignal<String>) {
    let contents = editor.text().to_string();
    if contents.is_empty() {
        return;
    }
    let mut cursor = editor.cursor.get_untracked();
    let mut selection = match &cursor.mode {
        CursorMode::Insert(selection) => selection.clone(),
        CursorMode::Normal(offset) => Selection::caret((*offset).min(contents.len())),
        CursorMode::Visual { start, end, .. } => Selection::region(*start, *end),
    };
    let selected = selected_occurrence_text(&contents, &selection);
    let Some((query, search_from)) = selected.or_else(|| {
        let caret = selection.max_offset().min(contents.len());
        word_at_offset(&contents, caret).map(|(start, end)| {
            selection = Selection::region(start, end);
            (contents[start..end].to_string(), end)
        })
    }) else {
        status.set("No word under cursor".into());
        return;
    };
    if query.is_empty() {
        return;
    }
    if let Some(start) = find_next_unselected_occurrence(&contents, &query, search_from, &selection)
    {
        selection.add_region(SelRegion::new(start, start + query.len(), None));
    }
    cursor.set_insert(selection);
    editor.cursor.set(cursor);
}

fn selected_occurrence_text(contents: &str, selection: &Selection) -> Option<(String, usize)> {
    selection
        .regions()
        .iter()
        .rev()
        .find(|region| !region.is_caret())
        .map(|region| {
            let start = region.min();
            let end = region.max();
            (contents[start..end].to_string(), end)
        })
}

fn find_next_unselected_occurrence(
    contents: &str,
    query: &str,
    search_from: usize,
    selection: &Selection,
) -> Option<usize> {
    let search_from = previous_char_boundary(contents, search_from.min(contents.len()));
    find_unselected_from(contents, query, search_from, selection)
        .or_else(|| find_unselected_from(contents, query, 0, selection))
}

fn find_unselected_from(
    contents: &str,
    query: &str,
    from: usize,
    selection: &Selection,
) -> Option<usize> {
    let mut offset = from;
    while let Some(relative) = contents[offset..].find(query) {
        let start = offset + relative;
        let end = start + query.len();
        if !selection
            .regions()
            .iter()
            .any(|region| region.min() == start && region.max() == end)
        {
            return Some(start);
        }
        offset = end;
        if offset >= contents.len() {
            return None;
        }
    }
    None
}

fn word_at_offset(contents: &str, offset: usize) -> Option<(usize, usize)> {
    let offset = previous_char_boundary(contents, offset.min(contents.len()));
    let mut start = offset;
    while start > 0 {
        let previous = contents[..start].chars().next_back()?;
        if !is_word_char(previous) {
            break;
        }
        start -= previous.len_utf8();
    }
    let mut end = offset;
    while end < contents.len() {
        let current = contents[end..].chars().next()?;
        if !is_word_char(current) {
            break;
        }
        end += current.len_utf8();
    }
    (start < end).then_some((start, end))
}

fn is_word_char(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn find_next_match(state: Rc<RefCell<NativeState>>, query: String, status: RwSignal<String>) {
    if query.is_empty() {
        status.set("Enter text to find".into());
        return;
    }
    let (contents, editor) = {
        let mut state = state.borrow_mut();
        state.sync_editor();
        let contents = state.active().contents.clone();
        let Some(editor) = state.current_editor.clone() else {
            status.set("No active editor".into());
            return;
        };
        (contents, editor)
    };
    let start = match editor.cursor.get_untracked().mode {
        CursorMode::Insert(selection) => selection.max_offset(),
        CursorMode::Normal(offset) => offset,
        CursorMode::Visual { end, .. } => end,
    }
    .min(contents.len());
    let start = previous_char_boundary(&contents, start);
    let range = contents[start..]
        .find(&query)
        .map(|offset| start + offset)
        .or_else(|| contents[..start].find(&query));
    let Some(start) = range else {
        status.set(format!("No matches for {query}"));
        return;
    };
    let end = start + query.len();
    let mut cursor = editor.cursor.get_untracked();
    cursor.set_insert(Selection::region(start, end));
    editor.cursor.set(cursor);
    status.set(format!("Found match at byte {start}"));
}

fn previous_char_boundary(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn view_menu(sidebar_visible: RwSignal<bool>, terminal_visible: RwSignal<bool>) -> Menu {
    Menu::new("View")
        .entry(MenuItem::new("Toggle Explorer").action(move || {
            sidebar_visible.update(|visible| *visible = !*visible);
        }))
        .entry(MenuItem::new("Toggle Terminal").action(move || {
            terminal_visible.update(|visible| *visible = !*visible);
        }))
}

#[allow(clippy::too_many_arguments)]
fn build_menu(
    state: Rc<RefCell<NativeState>>,
    terminal: Option<Rc<NativeTerminal>>,
    status: RwSignal<String>,
    root: PathBuf,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    config_state: Rc<RefCell<NativeState>>,
    config_root: PathBuf,
) -> Menu {
    Menu::new("Build")
        .entry(MenuItem::new("Execute Active File").action(move || {
            execute_active_file(state.clone(), terminal.clone(), status, root.clone());
        }))
        .entry(MenuItem::new("Set Build Commands...").action(move || {
            open_project_config(
                config_state.clone(),
                config_root.clone(),
                revision,
                selection_revision,
                status,
            );
        }))
}

fn execute_active_file(
    state: Rc<RefCell<NativeState>>,
    terminal: Option<Rc<NativeTerminal>>,
    status: RwSignal<String>,
    root: PathBuf,
) {
    let Some(terminal) = terminal else {
        status.set("Terminal is unavailable".into());
        return;
    };
    let file = {
        let mut state = state.borrow_mut();
        state.sync_editor();
        state.active().path.clone()
    };
    let config = match crate::config::ProjectConfig::load(&root) {
        Ok(config) => config,
        Err(error) => {
            status.set(format!("Build config unavailable: {error}"));
            return;
        }
    };
    let Some(command) = crate::run::command_for(&config, &root, file.as_deref()) else {
        status.set("No build command configured. Use Build > Set Build Commands...".into());
        return;
    };
    match terminal.send_command(&command) {
        Ok(()) => status.set("Build command sent to terminal".into()),
        Err(error) => status.set(format!("Build command failed: {error}")),
    }
}

fn open_project_config(
    state: Rc<RefCell<NativeState>>,
    root: PathBuf,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    status: RwSignal<String>,
) {
    let path = root.join(".nokin.toml");
    if !path.exists() {
        let template = "[run]\nworkspace = \"cargo run\"\n\n[run.files]\nrs = \"cargo run -- ${file}\"\n\n[c]\ncompiler = \"cc\"\ninclude_dirs = []\n";
        if let Err(error) = fs::write(&path, template) {
            status.set(format!("Build config creation failed: {error}"));
            return;
        }
    }
    match state.borrow_mut().open_path(&path) {
        Ok(()) => status.set(format!("Opened {}", path.display())),
        Err(error) => status.set(format!("Build config open failed: {error}")),
    }
    revision.update(|value| *value += 1);
    selection_revision.update(|value| *value += 1);
}

fn lsp_menu(
    state: Rc<RefCell<NativeState>>,
    revision: RwSignal<u64>,
    selection_revision: RwSignal<u64>,
    status: RwSignal<String>,
) -> Menu {
    let definition_state = state.clone();
    let hover_state = state.clone();
    let completion_state = state.clone();
    let references_state = state.clone();
    let signature_state = state.clone();
    let diagnostics_state = state.clone();
    Menu::new("Code")
        .entry(MenuItem::new("Go to Definition").action(move || {
            run_lsp_action(
                &definition_state,
                revision,
                selection_revision,
                status,
                true,
                NativeState::lsp_definition,
            )
        }))
        .entry(MenuItem::new("Hover").action(move || {
            run_lsp_action(
                &hover_state,
                revision,
                selection_revision,
                status,
                false,
                NativeState::lsp_hover,
            )
        }))
        .entry(MenuItem::new("Completion").action(move || {
            run_lsp_action(
                &completion_state,
                revision,
                selection_revision,
                status,
                false,
                NativeState::lsp_completion,
            )
        }))
        .entry(MenuItem::new("References").action(move || {
            run_lsp_action(
                &references_state,
                revision,
                selection_revision,
                status,
                false,
                NativeState::lsp_references,
            )
        }))
        .entry(MenuItem::new("Signature Help").action(move || {
            run_lsp_action(
                &signature_state,
                revision,
                selection_revision,
                status,
                false,
                NativeState::lsp_signature_help,
            )
        }))
        .separator()
        .entry(MenuItem::new("Diagnostics").action(move || {
            run_lsp_action(
                &diagnostics_state,
                revision,
                selection_revision,
                status,
                false,
                NativeState::lsp_diagnostics,
            )
        }))
        .entry(MenuItem::new("Format Document").action(move || {
            run_lsp_action(
                &state,
                revision,
                selection_revision,
                status,
                true,
                NativeState::lsp_format,
            )
        }))
}

fn menu_button(style: floem::style::Style, ui: UiPalette) -> floem::style::Style {
    style
        .height_full()
        .padding_horiz(10)
        .border(0)
        .border_radius(0)
        .color(rgb(ui.foreground))
        .background(rgb(ui.panel))
        .hover(|style| style.background(rgb(ui.raised)))
}

fn primary_shortcut(keypress: &KeyPress, key: &str) -> bool {
    let modifiers = keypress.mods;
    !modifiers.alt()
        && (modifiers.control() || modifiers.meta())
        && keypress.key.to_string().eq_ignore_ascii_case(key)
}

fn tab_title(document: &NativeDocument) -> String {
    format!(
        "{}{}",
        document.title,
        if document.dirty { "*" } else { "" }
    )
}

fn workspace_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                if !is_skipped_sidebar_path(&entry) {
                    pending.push(path);
                }
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SidebarRow {
    path: PathBuf,
    label: String,
    depth: usize,
    is_dir: bool,
    is_expanded: bool,
}

impl SidebarRow {
    fn file(path: PathBuf, depth: usize) -> Self {
        Self {
            label: file_title(&path),
            path,
            depth,
            is_dir: false,
            is_expanded: false,
        }
    }
}

fn sidebar_rows(root: &Path, expanded: &BTreeSet<PathBuf>) -> io::Result<Vec<SidebarRow>> {
    let mut rows = Vec::new();
    push_sidebar_children(root, 0, expanded, &mut rows)?;
    Ok(rows)
}

fn push_sidebar_children(
    directory: &Path,
    depth: usize,
    expanded: &BTreeSet<PathBuf>,
    rows: &mut Vec<SidebarRow>,
) -> io::Result<()> {
    let mut directories = Vec::new();
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if is_skipped_sidebar_path(&entry) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            directories.push(path);
        } else {
            files.push(path);
        }
    }
    directories.sort_by_key(|path| file_title(path).to_lowercase());
    files.sort_by_key(|path| file_title(path).to_lowercase());
    for path in directories {
        let is_expanded = expanded.contains(&path);
        rows.push(SidebarRow {
            label: file_title(&path),
            path: path.clone(),
            depth,
            is_dir: true,
            is_expanded,
        });
        if is_expanded {
            push_sidebar_children(&path, depth + 1, expanded, rows)?;
        }
    }
    rows.extend(files.into_iter().map(|path| SidebarRow::file(path, depth)));
    Ok(())
}

fn is_skipped_sidebar_path(entry: &fs::DirEntry) -> bool {
    let name = entry.file_name();
    let name = name.to_string_lossy();
    name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref())
}

fn save_text(path: &Path, contents: &str) -> io::Result<()> {
    fs::write(path, contents)
}

fn file_title(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let before_cursor = &source[..floor_char_boundary(source, offset)];
    let line = before_cursor.bytes().filter(|byte| *byte == b'\n').count();
    let column = before_cursor
        .rsplit_once('\n')
        .map(|(_, column)| column.len())
        .unwrap_or(before_cursor.len());
    (line, column)
}

fn line_column_offset(source: &str, line: usize, column: usize) -> Option<usize> {
    let mut line_start = 0;
    for _ in 0..line {
        line_start += source[line_start..].find('\n')? + 1;
    }
    let line_end = source[line_start..]
        .find('\n')
        .map(|end| line_start + end)
        .unwrap_or(source.len());
    Some(floor_char_boundary(
        source,
        (line_start + column).min(line_end),
    ))
}

fn floor_char_boundary(source: &str, mut offset: usize) -> usize {
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[derive(Clone)]
struct NativeStyling {
    id: Cell<u64>,
    palette: Palette,
    font_size: usize,
    font_family: Vec<FamilyOwned>,
    tab_width: usize,
    lines: RefCell<Vec<Vec<StyleSpan>>>,
}

#[derive(Clone)]
struct StyleSpan {
    start: usize,
    end: usize,
    color: usize,
}

impl NativeStyling {
    fn new(source: &str, palette: Palette, settings: &Settings) -> Self {
        Self {
            id: Cell::new(0),
            font_size: settings.editor.font_size.round().max(6.0) as usize,
            font_family: FamilyOwned::parse_list(&settings.editor.font_family).collect(),
            tab_width: settings.editor.tab_width,
            lines: RefCell::new(highlight(source, &palette)),
            palette,
        }
    }

    fn update(&self, source: &str) {
        *self.lines.borrow_mut() = highlight(source, &self.palette);
        self.id.set(self.id.get() + 1);
    }
}

impl Styling for NativeStyling {
    fn id(&self) -> u64 {
        self.id.get()
    }

    fn font_size(&self, _edid: EditorId, _line: usize) -> usize {
        self.font_size
    }

    fn font_family(&self, _edid: EditorId, _line: usize) -> Cow<'_, [FamilyOwned]> {
        Cow::Borrowed(&self.font_family)
    }

    fn tab_width(&self, _edid: EditorId, _line: usize) -> usize {
        self.tab_width
    }

    fn apply_attr_styles(
        &self,
        _edid: EditorId,
        _style: &EditorStyle,
        line: usize,
        default: Attrs,
        attrs: &mut AttrsList,
    ) {
        let lines = self.lines.borrow();
        for span in lines.get(line).into_iter().flatten() {
            attrs.add_span(span.start..span.end, default.color(rgb(span.color)));
        }
    }
}

fn highlight(source: &str, palette: &Palette) -> Vec<Vec<StyleSpan>> {
    source
        .lines()
        .map(|line| highlight_line(line, palette))
        .collect()
}

fn highlight_line(line: &str, palette: &Palette) -> Vec<StyleSpan> {
    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if (bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/')) || bytes[index] == b'#' {
            spans.push(StyleSpan {
                start: index,
                end: line.len(),
                color: palette.comment,
            });
            break;
        }
        if matches!(bytes[index], b'"' | b'\'') {
            let quote = bytes[index];
            let start = index;
            index += 1;
            while index < bytes.len() {
                if bytes[index] == quote && bytes[index.saturating_sub(1)] != b'\\' {
                    index += 1;
                    break;
                }
                index += 1;
            }
            spans.push(StyleSpan {
                start,
                end: index,
                color: palette.string,
            });
            continue;
        }
        if bytes[index].is_ascii_digit() {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'.')
            {
                index += 1;
            }
            spans.push(StyleSpan {
                start,
                end: index,
                color: palette.number,
            });
            continue;
        }
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            if is_keyword(&line[start..index]) {
                spans.push(StyleSpan {
                    start,
                    end: index,
                    color: palette.keyword,
                });
            }
            continue;
        }
        index += 1;
    }
    spans
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
    )
}

fn rgb(value: usize) -> Color {
    Color::rgb8(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, UNIX_EPOCH};

    #[test]
    fn scans_workspace_files_but_skips_build_directories() {
        let root = temporary_directory();
        fs::create_dir(root.join("src")).unwrap();
        fs::create_dir(root.join("target")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("target/generated.rs"), "ignored\n").unwrap();
        assert_eq!(
            workspace_files(&root).unwrap(),
            vec![root.join("src/main.rs")]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sidebar_rows_expand_folders_on_demand() {
        let root = temporary_directory();
        fs::create_dir(root.join("src")).unwrap();
        fs::create_dir(root.join("target")).unwrap();
        fs::write(root.join("README.md"), "readme\n").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("target/generated.rs"), "ignored\n").unwrap();

        let collapsed = sidebar_rows(&root, &BTreeSet::new()).unwrap();
        assert_eq!(
            collapsed
                .iter()
                .map(|row| (row.label.as_str(), row.depth, row.is_dir))
                .collect::<Vec<_>>(),
            vec![("src", 0, true), ("README.md", 0, false)]
        );

        let expanded = sidebar_rows(&root, &BTreeSet::from([root.join("src")])).unwrap();
        assert_eq!(
            expanded
                .iter()
                .map(|row| (row.label.as_str(), row.depth, row.is_dir))
                .collect::<Vec<_>>(),
            vec![
                ("src", 0, true),
                ("main.rs", 1, false),
                ("README.md", 0, false)
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opens_utf8_document_with_file_name_title() {
        let root = temporary_directory();
        let path = root.join("main.rs");
        fs::write(&path, "fn main() {}\n").unwrap();
        let document = NativeDocument::open(4, &path).unwrap();
        assert_eq!(document.path, Some(path));
        assert_eq!(document.title, "main.rs");
        assert_eq!(document.contents, "fn main() {}\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saves_document_text() {
        let root = temporary_directory();
        let path = root.join("notes.txt");
        save_text(&path, "updated\n").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "updated\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn highlights_keywords_strings_numbers_and_comments() {
        let palette = Palette::default();
        let spans = highlight_line("let value = \"text\"; // 42", &palette);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].color, palette.keyword);
        assert_eq!(spans[1].color, palette.string);
        assert_eq!(spans[2].color, palette.comment);
    }

    #[test]
    fn converts_offsets_to_lsp_line_columns_at_utf8_boundaries() {
        assert_eq!(line_column("one\ntext\n", 6), (1, 2));
        assert_eq!(line_column("a\néx", 4), (1, 2));
        assert_eq!(line_column("a\néx", 3), (1, 0));
    }

    #[test]
    fn converts_lsp_line_columns_to_clamped_utf8_offsets() {
        assert_eq!(line_column_offset("one\ntext\n", 1, 2), Some(6));
        assert_eq!(line_column_offset("a\néx", 1, 1), Some(2));
        assert_eq!(line_column_offset("a\néx", 1, 2), Some(4));
        assert_eq!(line_column_offset("one\ntext\n", 1, 99), Some(8));
        assert_eq!(line_column_offset("one\ntext\n", 3, 0), None);
    }

    #[test]
    fn caps_terminal_output_at_a_utf8_boundary() {
        let mut output = "é".repeat(TERMINAL_OUTPUT_LIMIT / 2 + 2);
        trim_terminal_output(&mut output);
        assert!(output.len() <= TERMINAL_OUTPUT_LIMIT);
        assert!(output.chars().all(|character| character == 'é'));
    }

    #[test]
    fn finds_next_unselected_occurrence_for_ctrl_d() {
        let source = "int helper(void);\nint main(void) {\n";
        assert_eq!(word_at_offset(source, 1), Some((0, 3)));
        let mut selection = Selection::region(0, 3);
        let next = find_next_unselected_occurrence(source, "int", 3, &selection).unwrap();
        assert_eq!(next, 18);
        selection.add_region(SelRegion::new(next, next + 3, None));
        assert_eq!(
            find_next_unselected_occurrence(source, "int", next + 3, &selection),
            None
        );
    }

    #[test]
    fn terminal_output_parser_strips_control_sequences() {
        let mut parser = TerminalOutputParser::default();
        assert_eq!(
            parser.clean(b"\x1b[32mgreen\x1b[0m\r\nplain\x08e\x1b]0;title\x07"),
            "green\nplaie"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn terminal_shell_executes_commands_in_workspace() {
        let root = temporary_directory();
        let terminal = NativeTerminal::spawn(&root, "/bin/sh").unwrap();
        terminal.send_command("printf 'nokin:%s' \"$PWD\"").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let output = terminal.pending_output.lock().unwrap().clone();
            if output.contains(&format!("nokin:{}", root.display())) {
                break;
            }
            assert!(Instant::now() < deadline, "terminal output was: {output:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(terminal);
        fs::remove_dir_all(root).unwrap();
    }

    fn temporary_directory() -> PathBuf {
        let id = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("nokin-native-ui-test-{id}"));
        fs::create_dir(&path).unwrap();
        path
    }
}
