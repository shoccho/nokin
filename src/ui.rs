use std::io;

use crate::workspace::Workspace;

#[cfg(all(feature = "gtk-ui", feature = "native-ui"))]
compile_error!("enable exactly one UI frontend: `gtk-ui` or `native-ui`");

#[cfg(not(any(feature = "gtk-ui", feature = "native-ui")))]
compile_error!("enable one UI frontend: `gtk-ui` or `native-ui`");

#[cfg(feature = "native-ui")]
mod native;

#[cfg(feature = "native-ui")]
pub fn choose_workspace() -> io::Result<Option<std::path::PathBuf>> {
    native::choose_workspace()
}

#[cfg(all(feature = "gtk-ui", not(feature = "native-ui"), target_os = "linux"))]
pub fn choose_workspace() -> io::Result<Option<std::path::PathBuf>> {
    linux::choose_workspace()
}

#[cfg(all(
    feature = "gtk-ui",
    not(feature = "native-ui"),
    not(target_os = "linux")
))]
pub fn choose_workspace() -> io::Result<Option<std::path::PathBuf>> {
    Err(io::Error::other("the GTK frontend supports Linux only"))
}

#[cfg(feature = "native-ui")]
pub fn run(workspace: &Workspace) -> io::Result<()> {
    native::run(workspace)
}

#[cfg(all(feature = "gtk-ui", not(feature = "native-ui"), target_os = "linux"))]
pub fn run(workspace: &Workspace) -> io::Result<()> {
    linux::run(workspace)
}

#[cfg(all(
    feature = "gtk-ui",
    not(feature = "native-ui"),
    not(target_os = "linux")
))]
pub fn run(_workspace: &Workspace) -> io::Result<()> {
    Err(io::Error::other("the GTK frontend supports Linux only"))
}

#[cfg(all(feature = "gtk-ui", not(feature = "native-ui"), target_os = "linux"))]
mod linux {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::fs;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::time::SystemTime;

    use crate::config::{ProjectConfig, Settings, UiThemeMode};
    use crate::index::Index;
    use crate::lsp;
    use crate::workspace::Workspace;

    type Widget = *mut c_void;
    type TreeStore = *mut c_void;
    type TreeModel = *mut c_void;
    type TreePath = *mut c_void;

    #[repr(C)]
    struct TreeIter {
        stamp: c_int,
        user_data: *mut c_void,
        user_data2: *mut c_void,
        user_data3: *mut c_void,
    }

    struct Tab {
        path: Option<PathBuf>,
        page: Widget,
        label: Widget,
        editor: scintilla::Editor,
        is_binary: bool,
    }

    struct CloseTabData {
        state: *mut AppState,
        page: Widget,
    }

    struct AppState {
        root: PathBuf,
        config: ProjectConfig,
        window: Widget,
        notebook: Widget,
        tree_store: TreeStore,
        tree: Widget,
        explorer: Widget,
        terminal_widget: Widget,
        css_provider: Widget,
        settings: Settings,
        index: Index,
        lsp: lsp::Manager,
        last_shortcut: Option<(u32, u32)>,
        pending_definition: Option<String>,
        pending_lsp_position: Option<(PathBuf, usize, usize)>,
        tabs: Vec<Tab>,
        close_data: Vec<Box<CloseTabData>>,
        welcome_page: Widget,
        explorer_context_dir: PathBuf,
        explorer_snapshot: Vec<(PathBuf, Option<SystemTime>)>,
        _terminal: Option<crate::vte::Terminal>,
    }

    const GTK_WINDOW_TOPLEVEL: c_int = 0;
    const GTK_FILE_CHOOSER_ACTION_OPEN: c_int = 0;
    const GTK_FILE_CHOOSER_ACTION_SAVE: c_int = 1;
    const GTK_FILE_CHOOSER_ACTION_SELECT_FOLDER: c_int = 2;
    const GTK_RESPONSE_ACCEPT: c_int = -3;
    const GTK_RESPONSE_CANCEL: c_int = -6;
    const GTK_ORIENTATION_HORIZONTAL: c_int = 0;
    const GTK_ORIENTATION_VERTICAL: c_int = 1;
    const GTK_POLICY_AUTOMATIC: c_int = 1;
    const GTK_RELIEF_NONE: c_int = 2;
    const G_TYPE_STRING: usize = 16 << 2;
    const GDK_CONTROL_MASK: u32 = 1 << 2;
    const GDK_SHIFT_MASK: u32 = 1 << 0;
    const GDK_KEY_F2: u32 = 0xffbf;
    const GDK_KEY_F5: u32 = 0xffc2;
    const GDK_KEY_F12: u32 = 0xffc9;
    const GDK_KEY_RETURN: u32 = 0xff0d;
    const GDK_KEY_KP_ENTER: u32 = 0xff8d;
    const GDK_KEY_LOWER_S: u32 = b's' as u32;
    const GDK_KEY_UPPER_S: u32 = b'S' as u32;
    const GDK_KEY_LOWER_N: u32 = b'n' as u32;
    const GDK_KEY_UPPER_N: u32 = b'N' as u32;
    const GDK_KEY_LOWER_B: u32 = b'b' as u32;
    const GDK_KEY_UPPER_B: u32 = b'B' as u32;
    const GDK_KEY_LOWER_D: u32 = b'd' as u32;
    const GDK_KEY_UPPER_D: u32 = b'D' as u32;
    const GDK_KEY_LOWER_J: u32 = b'j' as u32;
    const GDK_KEY_UPPER_J: u32 = b'J' as u32;
    const GDK_KEY_LOWER_I: u32 = b'i' as u32;
    const GDK_KEY_UPPER_I: u32 = b'I' as u32;
    const GDK_KEY_LOWER_K: u32 = b'k' as u32;
    const GDK_KEY_UPPER_K: u32 = b'K' as u32;
    const GDK_KEY_SPACE: u32 = b' ' as u32;
    const GDK_KEY_PERIOD: u32 = b'.' as u32;
    const GDK_KEY_CLOSING_BRACE: u32 = b'}' as u32;

    #[repr(C)]
    struct KeyEvent {
        event_type: c_int,
        window: Widget,
        send_event: i8,
        time: u32,
        state: u32,
        keyval: u32,
    }

    #[repr(C)]
    struct ButtonEvent {
        event_type: c_int,
        window: Widget,
        send_event: i8,
        time: u32,
        x: f64,
        y: f64,
        axes: *mut f64,
        state: u32,
        button: u32,
    }

    #[link(name = "gtk-3")]
    unsafe extern "C" {
        fn gtk_init_check(argc: *mut c_int, argv: *mut *mut *mut c_char) -> c_int;
        fn gtk_file_chooser_native_new(
            title: *const c_char,
            parent: Widget,
            action: c_int,
            accept_label: *const c_char,
            cancel_label: *const c_char,
        ) -> Widget;
        fn gtk_native_dialog_run(dialog: Widget) -> c_int;
        fn gtk_file_chooser_get_filename(chooser: Widget) -> *mut c_char;
        fn gtk_file_chooser_set_do_overwrite_confirmation(chooser: Widget, confirm: c_int);
        fn gtk_file_chooser_set_current_folder(chooser: Widget, folder: *const c_char) -> c_int;
        fn gtk_main();
        fn gtk_main_quit();
        fn gtk_window_new(kind: c_int) -> Widget;
        fn gtk_window_set_title(window: Widget, title: *const c_char);
        fn gtk_window_set_default_size(window: Widget, width: c_int, height: c_int);
        fn gtk_dialog_new() -> Widget;
        fn gtk_dialog_add_button(dialog: Widget, button_text: *const c_char, response_id: c_int);
        fn gtk_dialog_get_content_area(dialog: Widget) -> Widget;
        fn gtk_dialog_run(dialog: Widget) -> c_int;
        fn gtk_entry_new() -> Widget;
        fn gtk_entry_get_text(entry: Widget) -> *const c_char;
        fn gtk_entry_set_text(entry: Widget, text: *const c_char);
        fn gtk_event_box_new() -> Widget;
        fn gtk_paned_new(orientation: c_int) -> Widget;
        fn gtk_paned_pack1(paned: Widget, child: Widget, resize: c_int, shrink: c_int);
        fn gtk_paned_pack2(paned: Widget, child: Widget, resize: c_int, shrink: c_int);
        fn gtk_paned_set_position(paned: Widget, position: c_int);
        fn gtk_box_new(orientation: c_int, spacing: c_int) -> Widget;
        fn gtk_box_pack_start(
            container: Widget,
            child: Widget,
            expand: c_int,
            fill: c_int,
            padding: u32,
        );
        fn gtk_menu_bar_new() -> Widget;
        fn gtk_menu_new() -> Widget;
        fn gtk_menu_shell_append(menu: Widget, child: Widget);
        fn gtk_menu_item_new_with_mnemonic(label: *const c_char) -> Widget;
        fn gtk_menu_item_set_submenu(item: Widget, submenu: Widget);
        fn gtk_menu_popup_at_pointer(menu: Widget, event: *const ButtonEvent);
        fn gtk_separator_menu_item_new() -> Widget;
        fn gtk_notebook_new() -> Widget;
        fn gtk_notebook_append_page(notebook: Widget, child: Widget, label: Widget) -> c_int;
        fn gtk_notebook_get_current_page(notebook: Widget) -> c_int;
        fn gtk_notebook_page_num(notebook: Widget, child: Widget) -> c_int;
        fn gtk_notebook_set_current_page(notebook: Widget, page: c_int);
        fn gtk_notebook_set_scrollable(notebook: Widget, scrollable: c_int);
        fn gtk_label_new(text: *const c_char) -> Widget;
        fn gtk_label_set_text(label: Widget, text: *const c_char);
        fn gtk_container_add(container: Widget, widget: Widget);
        fn gtk_widget_grab_focus(widget: Widget);
        fn gtk_widget_show(widget: Widget);
        fn gtk_widget_show_all(widget: Widget);
        fn gtk_widget_hide(widget: Widget);
        fn gtk_widget_get_visible(widget: Widget) -> c_int;
        fn gtk_widget_destroy(widget: Widget);
        fn gtk_css_provider_new() -> Widget;
        fn gtk_css_provider_load_from_data(
            css_provider: Widget,
            data: *const c_char,
            length: isize,
            error: *mut Widget,
        ) -> c_int;
        fn gtk_style_context_add_provider_for_screen(
            screen: Widget,
            provider: Widget,
            priority: u32,
        );
        fn gtk_scrolled_window_new(hadjustment: Widget, vadjustment: Widget) -> Widget;
        fn gtk_scrolled_window_set_policy(scrolled: Widget, horizontal: c_int, vertical: c_int);
        fn gtk_scrolled_window_set_min_content_width(scrolled: Widget, width: c_int);
        fn gtk_tree_store_new(columns: c_int, ...) -> TreeStore;
        fn gtk_tree_store_append(store: TreeStore, iter: *mut TreeIter, parent: *const TreeIter);
        fn gtk_tree_store_set(store: TreeStore, iter: *mut TreeIter, ...);
        fn gtk_tree_store_remove(store: TreeStore, iter: *mut TreeIter) -> c_int;
        fn gtk_tree_store_clear(store: TreeStore);
        fn gtk_notebook_remove_page(notebook: Widget, page_num: c_int);
        fn gtk_button_new_with_label(label: *const c_char) -> Widget;
        fn gtk_button_set_relief(button: Widget, relief: c_int);
        fn gtk_check_button_new_with_label(label: *const c_char) -> Widget;
        fn gtk_toggle_button_set_active(button: Widget, is_active: c_int);
        fn gtk_toggle_button_get_active(button: Widget) -> c_int;
        fn gtk_combo_box_text_new() -> Widget;
        fn gtk_combo_box_text_append_text(combo_box: Widget, text: *const c_char);
        fn gtk_combo_box_text_get_active_text(combo_box: Widget) -> *mut c_char;
        fn gtk_combo_box_set_active(combo_box: Widget, index: c_int);
        fn gtk_tree_view_new_with_model(model: TreeModel) -> Widget;
        fn gtk_tree_view_get_selection(tree: Widget) -> Widget;
        fn gtk_tree_view_set_headers_visible(tree: Widget, visible: c_int);
        fn gtk_tree_view_column_new() -> Widget;
        fn gtk_tree_view_column_pack_start(column: Widget, renderer: Widget, expand: c_int);
        fn gtk_tree_view_column_add_attribute(
            column: Widget,
            renderer: Widget,
            attribute: *const c_char,
            model_column: c_int,
        );
        fn gtk_tree_view_append_column(tree: Widget, column: Widget) -> c_int;
        fn gtk_tree_view_expand_row(tree: Widget, path: TreePath, open_all: c_int) -> c_int;
        fn gtk_tree_view_set_enable_tree_lines(tree: Widget, enabled: c_int);
        fn gtk_tree_view_set_level_indentation(tree: Widget, indentation: c_int);
        fn gtk_tree_view_get_path_at_pos(
            tree: Widget,
            x: c_int,
            y: c_int,
            path: *mut TreePath,
            column: *mut Widget,
            cell_x: *mut c_int,
            cell_y: *mut c_int,
        ) -> c_int;
        fn gtk_tree_view_map_expanded_rows(
            tree: Widget,
            function: unsafe extern "C" fn(Widget, TreePath, *mut c_void),
            data: *mut c_void,
        );
        fn gtk_tree_path_new_from_string(path: *const c_char) -> TreePath;
        fn gtk_tree_path_to_string(path: TreePath) -> *mut c_char;
        fn gtk_tree_path_free(path: TreePath);
        fn gtk_cell_renderer_text_new() -> Widget;
        fn gtk_cell_renderer_pixbuf_new() -> Widget;
        fn gtk_tree_model_get_iter(model: TreeModel, iter: *mut TreeIter, path: TreePath) -> c_int;
        fn gtk_tree_model_iter_children(
            model: TreeModel,
            iter: *mut TreeIter,
            parent: *const TreeIter,
        ) -> c_int;
        fn gtk_tree_model_get(model: TreeModel, iter: *const TreeIter, ...);
        fn gtk_tree_selection_select_path(selection: Widget, path: TreePath);
    }

    #[link(name = "gdk-3")]
    unsafe extern "C" {
        fn gdk_screen_get_default() -> Widget;
    }

    #[link(name = "gobject-2.0")]
    unsafe extern "C" {
        fn g_signal_connect_data(
            instance: Widget,
            detailed_signal: *const c_char,
            handler: *const c_void,
            data: *mut c_void,
            destroy_data: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
            connect_flags: c_int,
        ) -> usize;
        fn g_object_unref(object: Widget);
    }

    #[link(name = "glib-2.0")]
    unsafe extern "C" {
        fn g_free(memory: *mut c_void);
        fn g_idle_add(
            function: unsafe extern "C" fn(*mut c_void) -> c_int,
            data: *mut c_void,
        ) -> u32;
        fn g_timeout_add_seconds(
            interval: u32,
            function: unsafe extern "C" fn(*mut c_void) -> c_int,
            data: *mut c_void,
        ) -> u32;
    }

    pub fn choose_workspace() -> io::Result<Option<PathBuf>> {
        // SAFETY: the dialog is created, run, and released on this main thread.
        unsafe {
            initialize()?;
            let title = CString::new("Choose a Nokin workspace").unwrap();
            let accept = CString::new("Open").unwrap();
            let cancel = CString::new("Cancel").unwrap();
            let dialog = gtk_file_chooser_native_new(
                title.as_ptr(),
                ptr::null_mut(),
                GTK_FILE_CHOOSER_ACTION_SELECT_FOLDER,
                accept.as_ptr(),
                cancel.as_ptr(),
            );
            let response = gtk_native_dialog_run(dialog);
            let path = if response == GTK_RESPONSE_ACCEPT {
                let filename = gtk_file_chooser_get_filename(dialog);
                if filename.is_null() {
                    None
                } else {
                    let path = PathBuf::from(std::ffi::OsStr::from_bytes(
                        CStr::from_ptr(filename).to_bytes(),
                    ));
                    g_free(filename.cast());
                    Some(path)
                }
            } else {
                None
            };
            g_object_unref(dialog);
            Ok(path)
        }
    }

    pub fn run(workspace: &Workspace) -> io::Result<()> {
        // SAFETY: GTK is initialized and used only on this main thread. Widget pointers are
        // created by GTK and kept alive by GTK's container ownership until gtk_main exits.
        unsafe {
            initialize()?;
            let window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
            let title = CString::new(format!("Nokin - {}", workspace.root.display()))
                .map_err(|_| io::Error::other("workspace path contains a NUL byte"))?;
            gtk_window_set_title(window, title.as_ptr());
            gtk_window_set_default_size(window, 1100, 760);
            connect(
                window,
                "destroy",
                on_destroy as *const c_void,
                ptr::null_mut(),
            );

            let settings = Settings::load()?;
            let lsp_commands = lsp::ServerCommands {
                clangd: settings.lsp.clangd.clone(),
                rust_analyzer: settings.lsp.rust_analyzer.clone(),
            };
            let horizontal = gtk_paned_new(GTK_ORIENTATION_HORIZONTAL);
            let vertical = gtk_paned_new(GTK_ORIENTATION_VERTICAL);
            let notebook = gtk_notebook_new();
            gtk_notebook_set_scrollable(notebook, 1);
            let scheme = crate::theme::load_scheme(&settings.editor.theme);
            let terminal = crate::vte::Terminal::new(
                &workspace.root,
                &settings.terminal.shell,
                &scheme.terminal,
            )
            .ok();
            let terminal_widget = terminal
                .as_ref()
                .map(crate::vte::Terminal::widget)
                .unwrap_or_else(|| label("Terminal\n\nVTE 2.91 is not installed"));

            let mut state = Box::new(AppState {
                root: workspace.root.clone(),
                config: workspace.config.clone(),
                window: ptr::null_mut(),
                notebook,
                tree_store: ptr::null_mut(),
                tree: ptr::null_mut(),
                explorer: ptr::null_mut(),
                terminal_widget,
                css_provider: install_css_provider()?,
                settings,
                index: build_index(workspace)?,
                lsp: lsp::Manager::new(&workspace.root, lsp_commands),
                last_shortcut: None,
                pending_definition: None,
                pending_lsp_position: None,
                tabs: Vec::new(),
                close_data: Vec::new(),
                welcome_page: ptr::null_mut(),
                explorer_context_dir: workspace.root.clone(),
                explorer_snapshot: Vec::new(),
                _terminal: terminal,
            });
            let state_ptr = (&mut *state) as *mut AppState;
            connect(
                window,
                "key-press-event",
                on_key_press as *const c_void,
                state_ptr.cast(),
            );
            connect(
                terminal_widget,
                "key-press-event",
                on_key_press as *const c_void,
                state_ptr.cast(),
            );
            let explorer = build_explorer(state_ptr)?;
            state.explorer = explorer;
            state.explorer_snapshot = explorer_snapshot(&state.root)?;
            state.window = window;
            state.apply_appearance_settings()?;
            state.restore_welcome_page();

            let menu = build_menu(state_ptr);
            let content = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
            gtk_paned_pack1(horizontal, explorer, 0, 0);
            gtk_paned_pack2(horizontal, vertical, 1, 0);
            gtk_paned_pack1(vertical, notebook, 1, 0);
            gtk_paned_pack2(vertical, terminal_widget, 0, 0);
            gtk_paned_set_position(vertical, 520);
            gtk_box_pack_start(content, menu, 0, 0, 0);
            gtk_box_pack_start(content, horizontal, 1, 1, 0);
            gtk_container_add(window, content);

            if let Some(path) = &workspace.initial_file {
                state.open_file(path)?;
            }
            gtk_widget_show_all(window);
            g_timeout_add_seconds(2, on_refresh_explorer_timeout, state_ptr.cast());
            gtk_main();
        }
        Ok(())
    }

    impl AppState {
        unsafe fn open_file(&mut self, path: &Path) -> io::Result<()> {
            let path = fs::canonicalize(path)?;
            if let Some(tab) = self
                .tabs
                .iter()
                .find(|tab| tab.path.as_ref() == Some(&path))
            {
                // SAFETY: notebook and tab editor widget remain alive for the GTK loop.
                unsafe {
                    let page = gtk_notebook_page_num(self.notebook, tab.page);
                    gtk_notebook_set_current_page(self.notebook, page);
                    gtk_widget_grab_focus(tab.editor.widget());
                }
                return Ok(());
            }
            let bytes = fs::read(&path)?;
            let (contents, is_binary) = match String::from_utf8(bytes) {
                Ok(text) => (text, false),
                Err(_) => (
                    "Not valid UTF-8 — cannot display this file.".to_string(),
                    true,
                ),
            };
            unsafe { self.open_tab(Some(path), &contents, is_binary) }
        }

        unsafe fn new_tab(&mut self) -> io::Result<()> {
            unsafe { self.open_tab(None, "", false) }
        }

        unsafe fn open_tab(
            &mut self,
            path: Option<PathBuf>,
            contents: &str,
            is_binary: bool,
        ) -> io::Result<()> {
            let editor = scintilla::Editor::new()?;
            editor.set_line_number_margin(48);
            editor.set_font(
                32,
                &self.settings.editor.font_family,
                self.settings.editor.font_size,
            )?;
            editor.set_ligatures(self.settings.editor.ligatures);
            editor.configure_indentation(
                self.settings.editor.tab_width,
                self.settings.editor.insert_spaces,
            );
            editor.set_text(contents)?;
            let palette = crate::theme::load(&self.settings.editor.theme);
            let scintilla_palette = scintilla::Palette::from(&palette);
            editor.apply_palette(&scintilla_palette);
            if !is_binary && let Some(path) = &path {
                if is_c_file(&path) {
                    editor.configure_c_lexer(&scintilla_palette)?;
                } else if let Some(lexer) = lexer_for_path(&path) {
                    editor.configure_basic_lexer(lexer, &scintilla_palette)?;
                }
            }
            // SAFETY: the editor widget and AppState remain alive for the GTK loop.
            unsafe {
                connect(
                    editor.widget(),
                    "key-press-event",
                    on_key_press as *const c_void,
                    (self as *mut AppState).cast(),
                );
                connect(
                    editor.widget(),
                    "key-release-event",
                    on_editor_key_release as *const c_void,
                    (self as *mut AppState).cast(),
                );
                connect(
                    editor.widget(),
                    "button-press-event",
                    on_editor_button_press as *const c_void,
                    (self as *mut AppState).cast(),
                );
            }
            let tab_name = path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy())
                .unwrap_or_else(|| "Untitled".into());
            // SAFETY: editor is a GTK widget and notebook retains it after append.
            unsafe {
                let was_empty = self.tabs.is_empty();
                let page = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
                gtk_box_pack_start(page, editor.widget(), 1, 1, 0);
                let tab_label_box = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
                let tab_event_box = gtk_event_box_new();
                let tab_label = label(&tab_name);
                gtk_box_pack_start(tab_label_box, tab_label, 0, 0, 0);
                let close_text = CString::new("\u{00d7}").unwrap();
                let close_btn = gtk_button_new_with_label(close_text.as_ptr());
                gtk_button_set_relief(close_btn, GTK_RELIEF_NONE);
                gtk_box_pack_start(tab_label_box, close_btn, 0, 0, 0);
                gtk_container_add(tab_event_box, tab_label_box);
                gtk_widget_show_all(tab_event_box);
                connect(
                    tab_event_box,
                    "button-press-event",
                    on_tab_label_button_press as *const c_void,
                    (self as *mut AppState).cast(),
                );
                let close_data_box = Box::new(CloseTabData {
                    state: self as *mut AppState,
                    page,
                });
                let close_data_ptr = close_data_box.as_ref() as *const CloseTabData as *mut c_void;
                self.close_data.push(close_data_box);
                connect(
                    close_btn,
                    "clicked",
                    on_close_tab as *const c_void,
                    close_data_ptr,
                );
                let page_number = gtk_notebook_append_page(self.notebook, page, tab_event_box);
                gtk_widget_show(editor.widget());
                gtk_widget_show(page);
                gtk_notebook_set_current_page(self.notebook, page_number);
                gtk_widget_grab_focus(editor.widget());
                self.tabs.push(Tab {
                    path,
                    page,
                    label: tab_label,
                    editor,
                    is_binary,
                });
                if was_empty && !self.welcome_page.is_null() {
                    let welcome_num = gtk_notebook_page_num(self.notebook, self.welcome_page);
                    if welcome_num >= 0 {
                        gtk_notebook_remove_page(self.notebook, welcome_num);
                    }
                    self.welcome_page = ptr::null_mut();
                }
            }
            Ok(())
        }

        unsafe fn save_active(&mut self) -> io::Result<()> {
            let Some(index) = (unsafe { self.active_tab_index() }) else {
                return Ok(());
            };
            if self.tabs[index].is_binary {
                return Ok(());
            }
            let path = match self.tabs[index].path.clone() {
                Some(path) => path,
                None => {
                    let Some(path) = (unsafe { self.save_file_dialog() }) else {
                        return Ok(());
                    };
                    let tab_name = path
                        .file_name()
                        .map(|name| name.to_string_lossy())
                        .unwrap_or_else(|| "Untitled".into());
                    let tab_name = CString::new(tab_name.as_bytes())
                        .map_err(|_| io::Error::other("file name contains a NUL byte"))?;
                    unsafe { gtk_label_set_text(self.tabs[index].label, tab_name.as_ptr()) };
                    self.tabs[index].path = Some(path.clone());
                    let palette = crate::theme::load(&self.settings.editor.theme);
                    let scintilla_palette = scintilla::Palette::from(&palette);
                    if is_c_file(&path) {
                        self.tabs[index]
                            .editor
                            .configure_c_lexer(&scintilla_palette)?;
                    } else if let Some(lexer) = lexer_for_path(&path) {
                        self.tabs[index]
                            .editor
                            .configure_basic_lexer(lexer, &scintilla_palette)?;
                    }
                    path
                }
            };
            let tab = &self.tabs[index];
            let bytes = tab.editor.text_bytes();
            fs::write(&path, &bytes)?;
            tab.editor.set_save_point();
            if path == self.root.join(".nokin.toml") {
                self.config = ProjectConfig::load(&self.root)?;
            }
            if is_c_file(&path) {
                self.index.update(&path, &String::from_utf8_lossy(&bytes));
            }
            let source = String::from_utf8_lossy(&bytes);
            if self.lsp.sync(&path, &source).is_ok() {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let diagnostics = self.lsp.diagnostics(&path);
                if let Some(tab) = self
                    .tabs
                    .iter()
                    .find(|tab| tab.path.as_ref() == Some(&path))
                {
                    tab.editor.show_diagnostics(
                        &diagnostics
                            .iter()
                            .map(|diagnostic| (diagnostic.line, diagnostic.column))
                            .collect::<Vec<_>>(),
                    );
                }
                self.refresh_semantic_tokens(&path, &source)?;
            }
            unsafe { self.refresh_explorer()? };
            Ok(())
        }

        unsafe fn run_active(&mut self) -> io::Result<()> {
            self.config = ProjectConfig::load(&self.root)?;
            let file = unsafe { self.active_tab() }.and_then(|tab| tab.path.as_deref());
            let command = crate::run::command_for(&self.config, &self.root, file)
                .unwrap_or_else(|| "printf 'nokin: no run command configured\\n'".into());
            if let Some(terminal) = &self._terminal {
                terminal.feed_command(&command)
            } else {
                Err(io::Error::other("VTE 2.91 is not installed"))
            }
        }

        unsafe fn active_tab(&self) -> Option<&Tab> {
            // SAFETY: notebook and editor widgets remain live for the GTK loop.
            let page = unsafe { gtk_notebook_get_current_page(self.notebook) };
            self.tabs
                .iter()
                .find(|tab| unsafe { gtk_notebook_page_num(self.notebook, tab.page) == page })
        }

        unsafe fn active_tab_index(&self) -> Option<usize> {
            // SAFETY: notebook and editor widgets remain live for the GTK loop.
            let page = unsafe { gtk_notebook_get_current_page(self.notebook) };
            self.tabs
                .iter()
                .position(|tab| unsafe { gtk_notebook_page_num(self.notebook, tab.page) == page })
        }

        unsafe fn edit_active(&self, edit: EditAction) {
            let Some(tab) = (unsafe { self.active_tab() }) else {
                return;
            };
            match edit {
                EditAction::Undo => tab.editor.undo(),
                EditAction::Redo => tab.editor.redo(),
                EditAction::Cut => tab.editor.cut(),
                EditAction::Copy => tab.editor.copy(),
                EditAction::Paste => tab.editor.paste(),
            }
        }

        unsafe fn toggle_panel(&self, panel: Widget) {
            if panel.is_null() {
                return;
            }
            if unsafe { gtk_widget_get_visible(panel) } != 0 {
                unsafe { gtk_widget_hide(panel) };
            } else {
                unsafe { gtk_widget_show(panel) };
            }
        }

        unsafe fn toggle_explorer(&self) {
            unsafe { self.toggle_panel(self.explorer) };
        }

        unsafe fn toggle_terminal(&self) {
            unsafe { self.toggle_panel(self.terminal_widget) };
        }

        unsafe fn goto_definition(&mut self) -> io::Result<()> {
            let lsp_position = self.pending_lsp_position.take().or_else(|| {
                let tab = unsafe { self.active_tab() }?;
                let path = tab.path.clone()?;
                let (line, column) = tab.editor.cursor_line_column();
                Some((path, line, column))
            });
            if let Some((file, line, column)) = lsp_position {
                let source = self
                    .tabs
                    .iter()
                    .find(|tab| tab.path.as_ref() == Some(&file))
                    .map(|tab| String::from_utf8_lossy(&tab.editor.text_bytes()).into_owned())
                    .unwrap_or_else(|| fs::read_to_string(&file).unwrap_or_default());
                match self.lsp.definition(&file, &source, line, column) {
                    Ok(Some(location)) => {
                        unsafe { self.open_file(&location.file)? };
                        if let Some(tab) = unsafe { self.active_tab() } {
                            tab.editor.goto_line(location.line);
                        }
                        return Ok(());
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("nokin: LSP definition unavailable: {error}"),
                }
            }
            let Some(name) = self.pending_definition.take().or_else(|| {
                (unsafe { self.active_tab() }).and_then(|tab| tab.editor.current_word())
            }) else {
                return Ok(());
            };
            let Some(symbol) = self.index.find(&name).first().cloned() else {
                return Err(io::Error::other(format!(
                    "definition not found for `{name}`"
                )));
            };
            unsafe { self.open_file(&symbol.file)? };
            if let Some(tab) = unsafe { self.active_tab() } {
                tab.editor.goto_line(symbol.line);
            }
            Ok(())
        }

        unsafe fn lsp_completion(&mut self) -> io::Result<()> {
            let Some((file, source, line, column)) = (unsafe { self.active_lsp_context() }) else {
                return Ok(());
            };
            let entries = self.lsp.completion(&file, &source, line, column)?;
            if !entries.is_empty()
                && let Some(tab) = unsafe { self.active_tab() }
            {
                tab.editor.show_completion(&entries)?;
            }
            Ok(())
        }

        unsafe fn lsp_hover(&mut self) -> io::Result<()> {
            let Some((file, source, line, column)) = (unsafe { self.active_lsp_context() }) else {
                return Ok(());
            };
            if let Some(hover) = self.lsp.hover(&file, &source, line, column)?
                && let Some(tab) = unsafe { self.active_tab() }
            {
                tab.editor.show_calltip(&hover)?;
            }
            Ok(())
        }

        unsafe fn lsp_signature_help(&mut self) -> io::Result<()> {
            let Some((file, source, line, column)) = (unsafe { self.active_lsp_context() }) else {
                return Ok(());
            };
            if let Some(signature) = self.lsp.signature_help(&file, &source, line, column)?
                && let Some(tab) = unsafe { self.active_tab() }
            {
                tab.editor.show_calltip(&signature)?;
            }
            Ok(())
        }

        unsafe fn lsp_code_actions(&mut self) -> io::Result<()> {
            let Some((file, source, line, column)) = (unsafe { self.active_lsp_context() }) else {
                return Ok(());
            };
            let actions = self.lsp.code_actions(&file, &source, line, column)?;
            let Some(action) = (match actions.as_slice() {
                [] => {
                    unsafe { information_dialog("Code Actions", "No code actions available.") };
                    None
                }
                [action] => Some(action),
                actions => {
                    let choices = actions
                        .iter()
                        .enumerate()
                        .map(|(index, action)| format!("{}. {}", index + 1, action.title))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let prompt = format!("{choices}\n\nAction number:");
                    let selected = unsafe { prompt_dialog("Code Actions", &prompt)? }
                        .and_then(|value| value.parse::<usize>().ok())
                        .and_then(|index| actions.get(index.saturating_sub(1)));
                    selected
                }
            }) else {
                return Ok(());
            };
            if action.edits.is_empty() {
                unsafe {
                    information_dialog(
                        "Code Actions",
                        "This action has no directly applicable workspace edit.",
                    )
                };
                return Ok(());
            }
            self.apply_lsp_edits(&action.edits)
        }

        unsafe fn lsp_references(&mut self) -> io::Result<()> {
            let Some((file, source, line, column)) = (unsafe { self.active_lsp_context() }) else {
                return Ok(());
            };
            let references = self.lsp.references(&file, &source, line, column)?;
            let text = if references.is_empty() {
                "No references found.".into()
            } else {
                references
                    .iter()
                    .map(|location| format!("{}:{}", location.file.display(), location.line))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            unsafe { information_dialog("References", &text) };
            Ok(())
        }

        unsafe fn lsp_rename(&mut self) -> io::Result<()> {
            let Some((file, source, line, column)) = (unsafe { self.active_lsp_context() }) else {
                return Ok(());
            };
            let Some(new_name) = (unsafe { prompt_dialog("Rename Symbol", "New name:")? }) else {
                return Ok(());
            };
            let edits = self.lsp.rename(&file, &source, line, column, &new_name)?;
            self.apply_lsp_edits(&edits)
        }

        unsafe fn lsp_format(&mut self) -> io::Result<()> {
            let Some((file, source, _, _)) = (unsafe { self.active_lsp_context() }) else {
                return Ok(());
            };
            let edits = self.lsp.formatting(&file, &source)?;
            self.apply_lsp_edits(&edits)
        }

        unsafe fn lsp_diagnostics(&mut self) -> io::Result<()> {
            let Some((file, source, _, _)) = (unsafe { self.active_lsp_context() }) else {
                return Ok(());
            };
            self.lsp.sync(&file, &source)?;
            std::thread::sleep(std::time::Duration::from_millis(250));
            let diagnostics = self.lsp.diagnostics(&file);
            if let Some(tab) = unsafe { self.active_tab() } {
                tab.editor.show_diagnostics(
                    &diagnostics
                        .iter()
                        .map(|diagnostic| (diagnostic.line, diagnostic.column))
                        .collect::<Vec<_>>(),
                );
            }
            let text = if diagnostics.is_empty() {
                "No diagnostics.".into()
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
            };
            unsafe { information_dialog("Diagnostics", &text) };
            Ok(())
        }

        unsafe fn lsp_semantic_tokens(&mut self) -> io::Result<()> {
            let Some((file, source, _, _)) = (unsafe { self.active_lsp_context() }) else {
                return Ok(());
            };
            self.refresh_semantic_tokens(&file, &source)
        }

        fn refresh_semantic_tokens(&mut self, file: &Path, source: &str) -> io::Result<()> {
            let tokens = self.lsp.semantic_tokens(file, source)?;
            if let Some(tab) = self
                .tabs
                .iter()
                .find(|tab| tab.path.as_deref() == Some(file))
            {
                tab.editor.show_semantic_tokens(
                    &tokens
                        .iter()
                        .map(|token| (token.line, token.column, token.length, token.kind.as_str()))
                        .collect::<Vec<_>>(),
                );
            }
            Ok(())
        }

        unsafe fn active_lsp_context(&self) -> Option<(PathBuf, String, usize, usize)> {
            let tab = unsafe { self.active_tab() }?;
            let path = tab.path.clone()?;
            let (line, column) = tab.editor.cursor_line_column();
            Some((
                path,
                String::from_utf8_lossy(&tab.editor.text_bytes()).into_owned(),
                line,
                column,
            ))
        }

        fn apply_lsp_edits(&mut self, edits: &[lsp::TextEdit]) -> io::Result<()> {
            let mut files = edits
                .iter()
                .map(|edit| edit.file.clone())
                .collect::<Vec<_>>();
            files.sort();
            files.dedup();
            for file in files {
                let source = self
                    .tabs
                    .iter()
                    .find(|tab| tab.path.as_ref() == Some(&file))
                    .map(|tab| String::from_utf8_lossy(&tab.editor.text_bytes()).into_owned())
                    .unwrap_or_else(|| fs::read_to_string(&file).unwrap_or_default());
                let relevant = edits
                    .iter()
                    .filter(|edit| edit.file == file)
                    .cloned()
                    .collect::<Vec<_>>();
                let updated = lsp::apply_text_edits(&source, &relevant);
                if let Some(tab) = self
                    .tabs
                    .iter()
                    .find(|tab| tab.path.as_ref() == Some(&file))
                {
                    tab.editor.replace_text(&updated)?;
                } else {
                    fs::write(file, updated)?;
                }
            }
            Ok(())
        }

        unsafe fn configure_build_commands(&mut self) -> io::Result<()> {
            // SAFETY: dialog widgets remain live until the modal dialog is destroyed below.
            unsafe {
                self.config = ProjectConfig::load(&self.root)?;
                let dialog = gtk_dialog_new();
                set_window_title(dialog, "Set Build Commands")?;
                add_dialog_button(dialog, "_Cancel", GTK_RESPONSE_CANCEL);
                add_dialog_button(dialog, "_Save", GTK_RESPONSE_ACCEPT);
                let content = gtk_dialog_get_content_area(dialog);
                let workspace_entry = command_row(
                    content,
                    "Workspace command:",
                    self.config.workspace_command.as_deref().unwrap_or(""),
                )?;
                let extension = self.active_extension().map(str::to_owned);
                let extension_entry = if let Some(extension) = &extension {
                    Some(command_row(
                        content,
                        &format!("Active file command (.{extension}):"),
                        self.config
                            .command_for_extension(extension)
                            .unwrap_or_default(),
                    )?)
                } else {
                    None
                };
                gtk_widget_show_all(dialog);
                if gtk_dialog_run(dialog) == GTK_RESPONSE_ACCEPT {
                    let workspace = entry_text(workspace_entry);
                    self.config.workspace_command = (!workspace.is_empty()).then_some(workspace);
                    if let (Some(extension), Some(entry)) = (extension, extension_entry) {
                        self.config
                            .set_command_for_extension(&extension, &entry_text(entry));
                    }
                    self.config.save(&self.root)?;
                }
                gtk_widget_destroy(dialog);
            }
            Ok(())
        }

        unsafe fn configure_settings(&mut self) -> io::Result<()> {
            // SAFETY: dialog widgets remain live until the modal dialog is destroyed below.
            unsafe {
                self.settings = Settings::load()?;
                let dialog = gtk_dialog_new();
                set_window_title(dialog, "Settings")?;
                add_dialog_button(dialog, "_Cancel", GTK_RESPONSE_CANCEL);
                add_dialog_button(dialog, "_Save", GTK_RESPONSE_ACCEPT);
                let content = gtk_dialog_get_content_area(dialog);
                section_row(content, "Appearance");
                let ui_modes = ["system", "color-scheme"];
                let ui_theme_mode = combo_row(
                    content,
                    "UI theme:",
                    &ui_modes,
                    self.settings.ui.theme_mode.as_str(),
                );
                let ui_font_family =
                    command_row(content, "UI font family:", &self.settings.ui.font_family)?;
                let ui_font_size = command_row(
                    content,
                    "UI font size:",
                    &self.settings.ui.font_size.to_string(),
                )?;
                let ui_scale = command_row(
                    content,
                    "UI scale (0.75 - 2.0):",
                    &self.settings.ui.scale.to_string(),
                )?;
                section_row(content, "Editor");
                let font_family =
                    command_row(content, "Font family:", &self.settings.editor.font_family)?;
                let font_size = command_row(
                    content,
                    "Font size:",
                    &self.settings.editor.font_size.to_string(),
                )?;
                let tab_width = command_row(
                    content,
                    "Tab width:",
                    &self.settings.editor.tab_width.to_string(),
                )?;
                let insert_spaces = check_row(
                    content,
                    "Use spaces for indentation (instead of tabs)",
                    self.settings.editor.insert_spaces,
                );
                let ligatures = check_row(
                    content,
                    "Enable editor font ligatures",
                    self.settings.editor.ligatures,
                );
                let theme_names = crate::theme::list();
                let theme_refs: Vec<&str> = theme_names.iter().map(String::as_str).collect();
                let theme = combo_row(content, "Theme:", &theme_refs, &self.settings.editor.theme);
                section_row(content, "Workspace");
                let close_tabs_on_open = check_row(
                    content,
                    "Close open files when switching folders",
                    self.settings.workspace.close_tabs_on_folder_open,
                );
                section_row(content, "Terminal");
                let shell = command_row(
                    content,
                    "Shell (next launch):",
                    &self.settings.terminal.shell,
                )?;
                section_row(content, "Language Servers");
                let clangd = command_row(content, "clangd command:", &self.settings.lsp.clangd)?;
                let rust_analyzer = command_row(
                    content,
                    "rust-analyzer command:",
                    &self.settings.lsp.rust_analyzer,
                )?;
                gtk_widget_show_all(dialog);
                if gtk_dialog_run(dialog) == GTK_RESPONSE_ACCEPT {
                    if let Some(mode) =
                        combo_active_text(ui_theme_mode).and_then(|mode| UiThemeMode::parse(&mode))
                    {
                        self.settings.ui.theme_mode = mode;
                    }
                    self.settings.ui.font_family = entry_text(ui_font_family);
                    self.settings.ui.font_size =
                        parse_bounded_entry(ui_font_size, "UI font size", 6.0, 32.0)?;
                    self.settings.ui.scale = parse_bounded_entry(ui_scale, "UI scale", 0.75, 2.0)?;
                    self.settings.editor.font_family = entry_text(font_family);
                    self.settings.editor.font_size = parse_entry(font_size, "font size")?;
                    self.settings.editor.tab_width = parse_entry(tab_width, "tab width")?;
                    self.settings.editor.insert_spaces =
                        gtk_toggle_button_get_active(insert_spaces) != 0;
                    self.settings.editor.ligatures = gtk_toggle_button_get_active(ligatures) != 0;
                    if let Some(t) = combo_active_text(theme) {
                        self.settings.editor.theme = t;
                    }
                    self.settings.workspace.close_tabs_on_folder_open =
                        gtk_toggle_button_get_active(close_tabs_on_open) != 0;
                    self.settings.terminal.shell = entry_text(shell);
                    self.settings.lsp.clangd = entry_text(clangd);
                    self.settings.lsp.rust_analyzer = entry_text(rust_analyzer);
                    self.settings.save()?;
                    self.lsp = lsp::Manager::new(
                        &self.root,
                        lsp::ServerCommands {
                            clangd: self.settings.lsp.clangd.clone(),
                            rust_analyzer: self.settings.lsp.rust_analyzer.clone(),
                        },
                    );
                    self.apply_editor_settings()?;
                    self.apply_appearance_settings()?;
                }
                gtk_widget_destroy(dialog);
            }
            Ok(())
        }

        fn apply_editor_settings(&self) -> io::Result<()> {
            let palette = crate::theme::load(&self.settings.editor.theme);
            let scintilla_palette = scintilla::Palette::from(&palette);
            for tab in &self.tabs {
                tab.editor.set_font(
                    32,
                    &self.settings.editor.font_family,
                    self.settings.editor.font_size,
                )?;
                tab.editor.set_ligatures(self.settings.editor.ligatures);
                tab.editor.configure_indentation(
                    self.settings.editor.tab_width,
                    self.settings.editor.insert_spaces,
                );
                tab.editor.apply_palette(&scintilla_palette);
                if let Some(path) = &tab.path {
                    if is_c_file(path) {
                        tab.editor.refresh_c_function_highlighting()?;
                    } else if let Some(lexer) = lexer_for_path(path) {
                        tab.editor
                            .configure_basic_lexer(lexer, &scintilla_palette)?;
                    }
                }
            }
            Ok(())
        }

        fn apply_appearance_settings(&self) -> io::Result<()> {
            let scheme = crate::theme::load_scheme(&self.settings.editor.theme);
            let css = appearance_css(&self.settings, &scheme.ui);
            let css =
                CString::new(css).map_err(|_| io::Error::other("UI CSS contains a NUL byte"))?;
            // SAFETY: provider, explorer, and terminal remain live during the GTK loop.
            unsafe {
                if gtk_css_provider_load_from_data(
                    self.css_provider,
                    css.as_ptr(),
                    -1,
                    ptr::null_mut(),
                ) == 0
                {
                    return Err(io::Error::other("failed to apply UI theme CSS"));
                }
                gtk_scrolled_window_set_min_content_width(
                    self.explorer,
                    (220.0 * self.settings.ui.scale).round() as c_int,
                );
            }
            if let Some(terminal) = &self._terminal {
                terminal.apply_palette(&scheme.terminal);
            }
            Ok(())
        }

        unsafe fn open_file_dialog(&mut self) -> io::Result<()> {
            // SAFETY: the dialog is created, run, and released on this main thread.
            unsafe {
                let title = CString::new("Open File").unwrap();
                let accept = CString::new("Open").unwrap();
                let cancel = CString::new("Cancel").unwrap();
                let dialog = gtk_file_chooser_native_new(
                    title.as_ptr(),
                    ptr::null_mut(),
                    GTK_FILE_CHOOSER_ACTION_OPEN,
                    accept.as_ptr(),
                    cancel.as_ptr(),
                );
                let response = gtk_native_dialog_run(dialog);
                let path = if response == GTK_RESPONSE_ACCEPT {
                    let filename = gtk_file_chooser_get_filename(dialog);
                    if filename.is_null() {
                        None
                    } else {
                        let path = PathBuf::from(std::ffi::OsStr::from_bytes(
                            CStr::from_ptr(filename).to_bytes(),
                        ));
                        g_free(filename.cast());
                        Some(path)
                    }
                } else {
                    None
                };
                g_object_unref(dialog);
                if let Some(path) = path {
                    self.open_file(&path)?;
                }
            }
            Ok(())
        }

        unsafe fn save_file_dialog(&self) -> Option<PathBuf> {
            // SAFETY: the dialog is created, run, and released on this main thread.
            unsafe {
                let title = CString::new("Save File").unwrap();
                let accept = CString::new("Save").unwrap();
                let cancel = CString::new("Cancel").unwrap();
                let dialog = gtk_file_chooser_native_new(
                    title.as_ptr(),
                    self.window,
                    GTK_FILE_CHOOSER_ACTION_SAVE,
                    accept.as_ptr(),
                    cancel.as_ptr(),
                );
                let root = CString::new(self.root.as_os_str().as_bytes()).ok()?;
                gtk_file_chooser_set_current_folder(dialog, root.as_ptr());
                gtk_file_chooser_set_do_overwrite_confirmation(dialog, 1);
                let response = gtk_native_dialog_run(dialog);
                let path = if response == GTK_RESPONSE_ACCEPT {
                    let filename = gtk_file_chooser_get_filename(dialog);
                    if filename.is_null() {
                        None
                    } else {
                        let path = PathBuf::from(std::ffi::OsStr::from_bytes(
                            CStr::from_ptr(filename).to_bytes(),
                        ));
                        g_free(filename.cast());
                        Some(path)
                    }
                } else {
                    None
                };
                g_object_unref(dialog);
                path
            }
        }

        unsafe fn open_folder_dialog(&mut self) -> io::Result<()> {
            // SAFETY: the dialog is created, run, and released on this main thread.
            unsafe {
                let title = CString::new("Open Folder").unwrap();
                let accept = CString::new("Open").unwrap();
                let cancel = CString::new("Cancel").unwrap();
                let dialog = gtk_file_chooser_native_new(
                    title.as_ptr(),
                    ptr::null_mut(),
                    GTK_FILE_CHOOSER_ACTION_SELECT_FOLDER,
                    accept.as_ptr(),
                    cancel.as_ptr(),
                );
                let response = gtk_native_dialog_run(dialog);
                let path = if response == GTK_RESPONSE_ACCEPT {
                    let filename = gtk_file_chooser_get_filename(dialog);
                    if filename.is_null() {
                        None
                    } else {
                        let path = PathBuf::from(std::ffi::OsStr::from_bytes(
                            CStr::from_ptr(filename).to_bytes(),
                        ));
                        g_free(filename.cast());
                        Some(path)
                    }
                } else {
                    None
                };
                g_object_unref(dialog);
                if let Some(folder) = path {
                    self.open_folder(folder)?;
                }
            }
            Ok(())
        }

        unsafe fn open_folder(&mut self, path: PathBuf) -> io::Result<()> {
            if self.settings.workspace.close_tabs_on_folder_open {
                unsafe { self.close_all_tabs() };
            }
            let path = fs::canonicalize(path)?;
            self.root = path;
            self.config = crate::config::ProjectConfig::load(&self.root)?;
            let title = CString::new(format!("Nokin - {}", self.root.display()))
                .map_err(|_| io::Error::other("workspace path contains a NUL byte"))?;
            // SAFETY: window remains live for the duration of gtk_main.
            unsafe { gtk_window_set_title(self.window, title.as_ptr()) };
            self.explorer_context_dir = self.root.clone();
            self.explorer_snapshot.clear();
            unsafe { self.refresh_explorer()? };
            let workspace = crate::workspace::Workspace {
                root: self.root.clone(),
                initial_file: None,
                config: self.config.clone(),
            };
            self.index = build_index(&workspace)?;
            self.lsp = lsp::Manager::new(
                &self.root,
                lsp::ServerCommands {
                    clangd: self.settings.lsp.clangd.clone(),
                    rust_analyzer: self.settings.lsp.rust_analyzer.clone(),
                },
            );
            if let Some(terminal) = &self._terminal {
                let path_str = self.root.to_string_lossy();
                let escaped = path_str.replace('\'', "'\\''");
                let _ = terminal.feed_command(&format!("cd '{escaped}'"));
            }
            Ok(())
        }

        unsafe fn refresh_explorer(&mut self) -> io::Result<()> {
            let snapshot = explorer_snapshot(&self.root)?;
            if snapshot == self.explorer_snapshot {
                return Ok(());
            }
            let mut expanded = Vec::<String>::new();
            let result = unsafe {
                gtk_tree_view_map_expanded_rows(
                    self.tree,
                    collect_expanded_path,
                    (&mut expanded as *mut Vec<String>).cast(),
                );
                gtk_tree_store_clear(self.tree_store);
                append_directory_children(self.tree_store, ptr::null(), &self.root)?;
                for path in expanded {
                    let path = CString::new(path).unwrap();
                    let tree_path = gtk_tree_path_new_from_string(path.as_ptr());
                    if !tree_path.is_null() {
                        gtk_tree_view_expand_row(self.tree, tree_path, 0);
                        gtk_tree_path_free(tree_path);
                    }
                }
                Ok::<(), io::Error>(())
            };
            result?;
            self.explorer_snapshot = snapshot;
            Ok(())
        }

        unsafe fn create_explorer_entry(&mut self, directory: bool) -> io::Result<()> {
            let kind = if directory { "Folder" } else { "File" };
            let Some(name) = (unsafe { prompt_dialog(&format!("New {kind}"), "Name:")? }) else {
                return Ok(());
            };
            if Path::new(&name).components().count() != 1 || name == "." || name == ".." {
                return Err(io::Error::other(
                    "name must be a single file or folder name",
                ));
            }
            let path = self.explorer_context_dir.join(name);
            if directory {
                fs::create_dir(&path)?;
            } else {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
            }
            unsafe { self.refresh_explorer()? };
            if !directory {
                unsafe { self.open_file(&path)? };
            }
            Ok(())
        }

        unsafe fn close_all_tabs(&mut self) {
            // SAFETY: notebook and tab pages remain live during gtk_main.
            for tab in &self.tabs {
                unsafe {
                    let page_num = gtk_notebook_page_num(self.notebook, tab.page);
                    if page_num >= 0 {
                        gtk_notebook_remove_page(self.notebook, page_num);
                    }
                }
            }
            self.tabs.clear();
            self.close_data.clear();
            unsafe { self.restore_welcome_page() };
        }

        unsafe fn restore_welcome_page(&mut self) {
            if !self.welcome_page.is_null() {
                return;
            }
            // SAFETY: notebook remains live during gtk_main.
            unsafe {
                let welcome_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
                gtk_box_pack_start(
                    welcome_box,
                    label(
                        "Open a file or folder to get started\n\nFile  \u{2192}  Open File...\nFile  \u{2192}  Open Folder...",
                    ),
                    1,
                    1,
                    0,
                );
                gtk_notebook_append_page(self.notebook, welcome_box, ptr::null_mut());
                gtk_widget_show_all(welcome_box);
                self.welcome_page = welcome_box;
            }
        }

        unsafe fn active_extension(&self) -> Option<&str> {
            unsafe { self.active_tab() }?
                .path
                .as_deref()?
                .extension()
                .and_then(|extension| extension.to_str())
        }
    }

    #[derive(Clone, Copy)]
    enum EditAction {
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
    }

    unsafe fn build_menu(state: *mut AppState) -> Widget {
        // SAFETY: menu widgets are retained by GTK and state remains live during gtk_main.
        unsafe {
            let bar = gtk_menu_bar_new();
            let file = submenu(bar, "_File");
            menu_action(
                file,
                "_New File",
                on_new_file_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                file,
                "Open _File...",
                on_open_file_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                file,
                "Open Fol_der...",
                on_open_folder_activate as *const c_void,
                state.cast(),
            );
            gtk_menu_shell_append(file, gtk_separator_menu_item_new());
            menu_action(
                file,
                "_Save",
                on_save_activate as *const c_void,
                state.cast(),
            );
            gtk_menu_shell_append(file, gtk_separator_menu_item_new());
            menu_action(
                file,
                "_Quit",
                on_quit_activate as *const c_void,
                ptr::null_mut(),
            );

            let edit = submenu(bar, "_Edit");
            menu_action(
                edit,
                "_Undo",
                on_undo_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                edit,
                "_Redo",
                on_redo_activate as *const c_void,
                state.cast(),
            );
            gtk_menu_shell_append(edit, gtk_separator_menu_item_new());
            menu_action(edit, "Cu_t", on_cut_activate as *const c_void, state.cast());
            menu_action(
                edit,
                "_Copy",
                on_copy_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                edit,
                "_Paste",
                on_paste_activate as *const c_void,
                state.cast(),
            );
            gtk_menu_shell_append(edit, gtk_separator_menu_item_new());
            menu_action(
                edit,
                "_Settings...",
                on_configure_settings_activate as *const c_void,
                state.cast(),
            );

            let view = submenu(bar, "_View");
            menu_action(
                view,
                "_Explorer",
                on_toggle_explorer_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                view,
                "_Terminal",
                on_toggle_terminal_activate as *const c_void,
                state.cast(),
            );

            let run = submenu(bar, "_Build");
            menu_action(
                run,
                "_Execute Active File",
                on_run_activate as *const c_void,
                state.cast(),
            );
            gtk_menu_shell_append(run, gtk_separator_menu_item_new());
            menu_action(
                run,
                "_Set Build Commands...",
                on_configure_build_commands_activate as *const c_void,
                state.cast(),
            );
            let navigate = submenu(bar, "_Navigate");
            menu_action(
                navigate,
                "_Go to Definition",
                on_goto_definition_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                navigate,
                "Find _References",
                on_lsp_references_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                navigate,
                "_Hover Information",
                on_lsp_hover_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                navigate,
                "_Signature Help",
                on_lsp_signature_help_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                navigate,
                "_Code Actions...",
                on_lsp_code_actions_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                navigate,
                "_Rename Symbol...",
                on_lsp_rename_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                navigate,
                "_Format Document",
                on_lsp_format_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                navigate,
                "Refresh _Diagnostics",
                on_lsp_diagnostics_activate as *const c_void,
                state.cast(),
            );
            menu_action(
                navigate,
                "Refresh _Semantic Tokens",
                on_lsp_semantic_tokens_activate as *const c_void,
                state.cast(),
            );
            bar
        }
    }

    unsafe fn submenu(bar: Widget, name: &str) -> Widget {
        // SAFETY: GTK copies menu labels and retains appended widgets.
        unsafe {
            let root = menu_item(bar, name);
            let menu = gtk_menu_new();
            gtk_menu_item_set_submenu(root, menu);
            menu
        }
    }

    unsafe fn menu_item(menu: Widget, name: &str) -> Widget {
        let name = CString::new(name).unwrap();
        // SAFETY: GTK copies the label and retains the item after append.
        unsafe {
            let item = gtk_menu_item_new_with_mnemonic(name.as_ptr());
            gtk_menu_shell_append(menu, item);
            item
        }
    }

    unsafe fn menu_action(menu: Widget, name: &str, callback: *const c_void, data: *mut c_void) {
        // SAFETY: callback ABI matches GtkMenuItem::activate.
        unsafe {
            let item = menu_item(menu, name);
            connect(item, "activate", callback, data);
        }
    }

    unsafe fn install_css_provider() -> io::Result<Widget> {
        unsafe {
            let screen = gdk_screen_get_default();
            if screen.is_null() {
                return Err(io::Error::other("GTK screen is not available"));
            }
            let provider = gtk_css_provider_new();
            if provider.is_null() {
                return Err(io::Error::other("GTK CSS provider creation failed"));
            }
            gtk_style_context_add_provider_for_screen(screen, provider, 600);
            Ok(provider)
        }
    }

    fn appearance_css(settings: &Settings, palette: &crate::theme::UiPalette) -> String {
        let scale = settings.ui.scale;
        let font_size = settings.ui.font_size * scale;
        let compact = (3.0 * scale).round() as usize;
        let regular = (6.0 * scale).round() as usize;
        let wide = (10.0 * scale).round() as usize;
        let font_family = css_string(&settings.ui.font_family);
        let mut css = format!(
            "* {{ font-family: \"{font_family}\"; font-size: {font_size:.2}pt; }}\n\
             button, entry, combobox {{ padding: {compact}px {regular}px; }}\n\
             menuitem {{ padding: {compact}px {wide}px; }}\n\
             notebook tab {{ padding: {compact}px {regular}px; }}\n\
             treeview {{ padding: {compact}px; }}\n"
        );
        if settings.ui.theme_mode == UiThemeMode::ColorScheme {
            css.push_str(&format!(
                "window, dialog, menubar, menu, notebook, treeview, .view {{ color: {}; background-color: {}; }}\n\
                 menubar, menu, notebook header, treeview, entry, combobox, button {{ color: {}; background-color: {}; }}\n\
                 button:hover, menuitem:hover, notebook tab:hover {{ background-color: {}; }}\n\
                 button:checked, notebook tab:checked, treeview:selected, menuitem:hover {{ color: {}; background-color: {}; }}\n\
                 entry, combobox, button, notebook header, scrolledwindow {{ border-color: {}; }}\n\
                 separator {{ background-color: {}; }}\n",
                css_color(palette.foreground),
                css_color(palette.background),
                css_color(palette.foreground),
                css_color(palette.panel),
                css_color(palette.raised),
                css_color(palette.selection_foreground),
                css_color(palette.selection_background),
                css_color(palette.border),
                css_color(palette.border),
            ));
        }
        css
    }

    fn css_color(color: usize) -> String {
        format!("#{:06x}", color & 0xffffff)
    }

    fn css_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    unsafe fn section_row(container: Widget, title: &str) {
        unsafe { gtk_box_pack_start(container, label(title), 0, 1, 8) };
    }

    unsafe fn command_row(container: Widget, title: &str, value: &str) -> io::Result<Widget> {
        let row = unsafe { gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8) };
        unsafe { gtk_box_pack_start(container, row, 0, 1, 4) };
        unsafe { gtk_box_pack_start(row, label(title), 0, 0, 4) };
        let entry = unsafe { gtk_entry_new() };
        let value = CString::new(value)
            .map_err(|_| io::Error::other("build command contains a NUL byte"))?;
        unsafe {
            gtk_entry_set_text(entry, value.as_ptr());
            gtk_box_pack_start(row, entry, 1, 1, 4);
        }
        Ok(entry)
    }

    unsafe fn combo_row(container: Widget, title: &str, options: &[&str], current: &str) -> Widget {
        unsafe {
            let row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
            gtk_box_pack_start(container, row, 0, 1, 4);
            gtk_box_pack_start(row, label(title), 0, 0, 4);
            let combo = gtk_combo_box_text_new();
            for (i, option) in options.iter().enumerate() {
                let text = CString::new(*option).unwrap();
                gtk_combo_box_text_append_text(combo, text.as_ptr());
                if *option == current {
                    gtk_combo_box_set_active(combo, i as c_int);
                }
            }
            gtk_box_pack_start(row, combo, 1, 1, 4);
            combo
        }
    }

    unsafe fn combo_active_text(combo: Widget) -> Option<String> {
        unsafe {
            let text = gtk_combo_box_text_get_active_text(combo);
            if text.is_null() {
                return None;
            }
            let result = CStr::from_ptr(text).to_string_lossy().into_owned();
            g_free(text.cast());
            Some(result)
        }
    }

    unsafe fn check_row(container: Widget, title: &str, active: bool) -> Widget {
        let text = CString::new(title).unwrap();
        unsafe {
            let check = gtk_check_button_new_with_label(text.as_ptr());
            gtk_toggle_button_set_active(check, active as c_int);
            gtk_box_pack_start(container, check, 0, 1, 4);
            check
        }
    }

    unsafe fn add_dialog_button(dialog: Widget, title: &str, response: c_int) {
        let title = CString::new(title).unwrap();
        unsafe { gtk_dialog_add_button(dialog, title.as_ptr(), response) };
    }

    unsafe fn set_window_title(window: Widget, title: &str) -> io::Result<()> {
        let title = CString::new(title)
            .map_err(|_| io::Error::other("window title contains a NUL byte"))?;
        unsafe { gtk_window_set_title(window, title.as_ptr()) };
        Ok(())
    }

    unsafe fn entry_text(entry: Widget) -> String {
        unsafe { CStr::from_ptr(gtk_entry_get_text(entry)) }
            .to_string_lossy()
            .trim()
            .to_owned()
    }

    unsafe fn information_dialog(title: &str, text: &str) {
        unsafe {
            let dialog = gtk_dialog_new();
            if set_window_title(dialog, title).is_err() {
                return;
            }
            add_dialog_button(dialog, "_Close", GTK_RESPONSE_ACCEPT);
            gtk_box_pack_start(gtk_dialog_get_content_area(dialog), label(text), 1, 1, 8);
            gtk_widget_show_all(dialog);
            gtk_dialog_run(dialog);
            gtk_widget_destroy(dialog);
        }
    }

    unsafe fn prompt_dialog(title: &str, prompt: &str) -> io::Result<Option<String>> {
        unsafe {
            let dialog = gtk_dialog_new();
            set_window_title(dialog, title)?;
            add_dialog_button(dialog, "_Cancel", GTK_RESPONSE_CANCEL);
            add_dialog_button(dialog, "_Apply", GTK_RESPONSE_ACCEPT);
            let entry = command_row(gtk_dialog_get_content_area(dialog), prompt, "")?;
            gtk_widget_show_all(dialog);
            let value = (gtk_dialog_run(dialog) == GTK_RESPONSE_ACCEPT).then(|| entry_text(entry));
            gtk_widget_destroy(dialog);
            Ok(value.filter(|value| !value.is_empty()))
        }
    }

    unsafe fn parse_entry<T: std::str::FromStr>(entry: Widget, name: &str) -> io::Result<T> {
        unsafe { entry_text(entry) }
            .parse()
            .map_err(|_| io::Error::other(format!("invalid {name}")))
    }

    unsafe fn parse_bounded_entry(
        entry: Widget,
        name: &str,
        min: f64,
        max: f64,
    ) -> io::Result<f64> {
        let value: f64 = unsafe { parse_entry(entry, name) }?;
        if (min..=max).contains(&value) {
            Ok(value)
        } else {
            Err(io::Error::other(format!(
                "{name} must be between {min} and {max}"
            )))
        }
    }

    unsafe fn build_explorer(state: *mut AppState) -> io::Result<Widget> {
        // SAFETY: GTK copies model values; state remains boxed for the GTK loop.
        unsafe {
            let store = gtk_tree_store_new(3, G_TYPE_STRING, G_TYPE_STRING, G_TYPE_STRING);
            (*state).tree_store = store;
            append_directory_children(store, ptr::null(), &(*state).root)?;
            let tree = gtk_tree_view_new_with_model(store);
            (*state).tree = tree;
            gtk_tree_view_set_headers_visible(tree, 0);
            gtk_tree_view_set_enable_tree_lines(tree, 1);
            gtk_tree_view_set_level_indentation(tree, 12);
            let icon_renderer = gtk_cell_renderer_pixbuf_new();
            let renderer = gtk_cell_renderer_text_new();
            let text = CString::new("text").unwrap();
            let icon_name = CString::new("icon-name").unwrap();
            let column = gtk_tree_view_column_new();
            gtk_tree_view_column_pack_start(column, icon_renderer, 0);
            gtk_tree_view_column_add_attribute(column, icon_renderer, icon_name.as_ptr(), 0);
            gtk_tree_view_column_pack_start(column, renderer, 1);
            gtk_tree_view_column_add_attribute(column, renderer, text.as_ptr(), 1);
            gtk_tree_view_append_column(tree, column);
            connect(
                tree,
                "row-activated",
                on_row_activated as *const c_void,
                state.cast(),
            );
            connect(
                tree,
                "test-expand-row",
                on_test_expand_row as *const c_void,
                state.cast(),
            );
            connect(
                tree,
                "button-press-event",
                on_explorer_button_press as *const c_void,
                state.cast(),
            );
            let scrolled = gtk_scrolled_window_new(ptr::null_mut(), ptr::null_mut());
            gtk_scrolled_window_set_policy(scrolled, GTK_POLICY_AUTOMATIC, GTK_POLICY_AUTOMATIC);
            gtk_scrolled_window_set_min_content_width(scrolled, 220);
            gtk_container_add(scrolled, tree);
            Ok(scrolled)
        }
    }

    unsafe fn append_directory_children(
        store: TreeStore,
        parent: *const TreeIter,
        directory: &Path,
    ) -> io::Result<()> {
        let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| {
            let is_file = entry.file_type().map(|kind| kind.is_file()).unwrap_or(true);
            (is_file, entry.file_name())
        });
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name();
            let mut iter = empty_iter();
            // SAFETY: store and parent belong to the live tree model.
            unsafe {
                gtk_tree_store_append(store, &mut iter, parent);
                set_tree_row(store, &mut iter, &name.to_string_lossy(), &path)?;
            }
            if entry.file_type()?.is_dir() {
                let mut placeholder = empty_iter();
                // SAFETY: placeholder is inserted below the live directory row.
                unsafe {
                    gtk_tree_store_append(store, &mut placeholder, &iter);
                    set_tree_row(store, &mut placeholder, "", Path::new(""))?;
                }
            }
        }
        Ok(())
    }

    fn explorer_snapshot(root: &Path) -> io::Result<Vec<(PathBuf, Option<SystemTime>)>> {
        let mut snapshot = Vec::new();
        append_directory_snapshot(root, &mut snapshot)?;
        snapshot.sort_by(|(left, _), (right, _)| left.cmp(right));
        Ok(snapshot)
    }

    fn append_directory_snapshot(
        directory: &Path,
        snapshot: &mut Vec<(PathBuf, Option<SystemTime>)>,
    ) -> io::Result<()> {
        snapshot.push((
            directory.to_path_buf(),
            fs::metadata(directory)?.modified().ok(),
        ));
        for entry in fs::read_dir(directory)?.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let _ = append_directory_snapshot(&entry.path(), snapshot);
            }
        }
        Ok(())
    }

    unsafe fn set_tree_row(
        store: TreeStore,
        iter: *mut TreeIter,
        display: &str,
        path: &Path,
    ) -> io::Result<()> {
        let display =
            CString::new(display).map_err(|_| io::Error::other("file name contains a NUL byte"))?;
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::other("file path contains a NUL byte"))?;
        let icon = if path.as_bytes().is_empty() {
            ""
        } else if Path::new(std::ffi::OsStr::from_bytes(path.as_bytes())).is_dir() {
            "folder-symbolic"
        } else {
            "text-x-generic-symbolic"
        };
        let icon = CString::new(icon).unwrap();
        // SAFETY: column indexes and value types match the GtkTreeStore declaration.
        unsafe {
            gtk_tree_store_set(
                store,
                iter,
                0,
                icon.as_ptr(),
                1,
                display.as_ptr(),
                2,
                path.as_ptr(),
                -1,
            )
        };
        Ok(())
    }

    unsafe extern "C" fn on_row_activated(
        tree: Widget,
        path: TreePath,
        _column: Widget,
        data: *mut c_void,
    ) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        unsafe {
            let state = &mut *data.cast::<AppState>();
            let Some(file) = model_path(state.tree_store, path) else {
                return;
            };
            if file.is_dir() {
                gtk_tree_view_expand_row(tree, path, 0);
            } else if let Err(error) = state.open_file(&file) {
                eprintln!("nokin: could not open {}: {error}", file.display());
            }
        }
    }

    unsafe extern "C" fn collect_expanded_path(_tree: Widget, path: TreePath, data: *mut c_void) {
        // SAFETY: GTK owns path during this callback and data points to the temporary vector.
        unsafe {
            let text = gtk_tree_path_to_string(path);
            if !text.is_null() {
                (*data.cast::<Vec<String>>()).push(CStr::from_ptr(text).to_string_lossy().into());
                g_free(text.cast());
            }
        }
    }

    unsafe extern "C" fn on_refresh_explorer_timeout(data: *mut c_void) -> c_int {
        // SAFETY: AppState remains boxed for the GTK loop.
        let state = unsafe { &mut *data.cast::<AppState>() };
        if let Err(error) = unsafe { state.refresh_explorer() } {
            eprintln!("nokin: could not refresh explorer: {error}");
        }
        1
    }

    unsafe extern "C" fn on_explorer_button_press(
        tree: Widget,
        event: *const ButtonEvent,
        data: *mut c_void,
    ) -> c_int {
        // SAFETY: callback data, tree, and event remain valid for this signal.
        unsafe {
            let event = &*event;
            if event.button == 1 {
                let mut tree_path = ptr::null_mut();
                if gtk_tree_view_get_path_at_pos(
                    tree,
                    event.x as c_int,
                    event.y as c_int,
                    &mut tree_path,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                ) != 0
                {
                    let state = &mut *data.cast::<AppState>();
                    if let Some(path) = model_path(state.tree_store, tree_path)
                        && path.is_file()
                        && let Err(error) = state.open_file(&path)
                    {
                        eprintln!("nokin: could not open {}: {error}", path.display());
                    }
                    gtk_tree_path_free(tree_path);
                }
                return 0;
            }
            if event.button != 3 {
                return 0;
            }
            let state = &mut *data.cast::<AppState>();
            let mut tree_path = ptr::null_mut();
            if gtk_tree_view_get_path_at_pos(
                tree,
                event.x as c_int,
                event.y as c_int,
                &mut tree_path,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            ) != 0
            {
                let selection = gtk_tree_view_get_selection(tree);
                gtk_tree_selection_select_path(selection, tree_path);
                if let Some(path) = model_path(state.tree_store, tree_path) {
                    state.explorer_context_dir = if path.is_dir() {
                        path
                    } else {
                        path.parent().unwrap_or(&state.root).to_path_buf()
                    };
                }
                gtk_tree_path_free(tree_path);
            } else {
                state.explorer_context_dir = state.root.clone();
            }
            let menu = gtk_menu_new();
            menu_action(
                menu,
                "New _File...",
                on_create_file_activate as *const c_void,
                data,
            );
            menu_action(
                menu,
                "New Fol_der...",
                on_create_folder_activate as *const c_void,
                data,
            );
            gtk_menu_shell_append(menu, gtk_separator_menu_item_new());
            menu_action(
                menu,
                "_Refresh",
                on_refresh_explorer_activate as *const c_void,
                data,
            );
            gtk_widget_show_all(menu);
            gtk_menu_popup_at_pointer(menu, event);
            1
        }
    }

    unsafe extern "C" fn on_tab_label_button_press(
        _label: Widget,
        event: *const ButtonEvent,
        data: *mut c_void,
    ) -> c_int {
        // SAFETY: callback data and event remain valid for this signal.
        unsafe {
            let event = &*event;
            if event.button != 3 {
                return 0;
            }
            let menu = gtk_menu_new();
            menu_action(
                menu,
                "_Close All Files",
                on_close_all_tabs_activate as *const c_void,
                data,
            );
            gtk_widget_show_all(menu);
            gtk_menu_popup_at_pointer(menu, event);
            1
        }
    }

    unsafe extern "C" fn on_key_press(
        widget: Widget,
        event: *const KeyEvent,
        data: *mut c_void,
    ) -> c_int {
        // SAFETY: callback data and event are valid for the duration of this signal.
        unsafe {
            let state = &mut *data.cast::<AppState>();
            let event = &*event;
            if event.state & GDK_CONTROL_MASK != 0
                && matches!(event.keyval, GDK_KEY_LOWER_D | GDK_KEY_UPPER_D)
                && let Some(tab) = state.tabs.iter().find(|tab| tab.editor.widget() == widget)
            {
                tab.editor.select_next_occurrence();
                return 1;
            }
            if state.last_shortcut.is_some_and(|(time, key)| {
                key == event.keyval && event.time.wrapping_sub(time) < 250
            }) {
                return 1;
            }
            let result = if event.state & (GDK_CONTROL_MASK | GDK_SHIFT_MASK)
                == (GDK_CONTROL_MASK | GDK_SHIFT_MASK)
                && event.keyval == GDK_KEY_SPACE
            {
                state.lsp_signature_help()
            } else if event.state & GDK_CONTROL_MASK != 0 && event.keyval == GDK_KEY_PERIOD {
                state.lsp_code_actions()
            } else if event.state & GDK_CONTROL_MASK != 0 && event.keyval == GDK_KEY_SPACE {
                state.lsp_completion()
            } else if event.state & GDK_CONTROL_MASK != 0
                && matches!(event.keyval, GDK_KEY_LOWER_K | GDK_KEY_UPPER_K)
            {
                state.lsp_hover()
            } else if event.state & (GDK_CONTROL_MASK | GDK_SHIFT_MASK)
                == (GDK_CONTROL_MASK | GDK_SHIFT_MASK)
                && matches!(event.keyval, GDK_KEY_LOWER_I | GDK_KEY_UPPER_I)
            {
                state.lsp_format()
            } else if event.keyval == GDK_KEY_F2 {
                state.lsp_rename()
            } else if event.state & GDK_CONTROL_MASK != 0
                && matches!(event.keyval, GDK_KEY_LOWER_B | GDK_KEY_UPPER_B)
            {
                state.last_shortcut = Some((event.time, event.keyval));
                state.toggle_explorer();
                Ok(())
            } else if event.state & GDK_CONTROL_MASK != 0
                && matches!(event.keyval, GDK_KEY_LOWER_J | GDK_KEY_UPPER_J)
            {
                state.last_shortcut = Some((event.time, event.keyval));
                state.toggle_terminal();
                Ok(())
            } else if event.state & GDK_CONTROL_MASK != 0
                && matches!(event.keyval, GDK_KEY_LOWER_N | GDK_KEY_UPPER_N)
            {
                state.last_shortcut = Some((event.time, event.keyval));
                state.new_tab()
            } else if event.state & GDK_CONTROL_MASK != 0
                && matches!(event.keyval, GDK_KEY_LOWER_S | GDK_KEY_UPPER_S)
            {
                state.last_shortcut = Some((event.time, event.keyval));
                state.save_active()
            } else if event.keyval == GDK_KEY_F5 {
                state.last_shortcut = Some((event.time, event.keyval));
                state.run_active()
            } else if event.keyval == GDK_KEY_F12 {
                state.last_shortcut = Some((event.time, event.keyval));
                g_idle_add(on_goto_definition_idle, data);
                Ok(())
            } else {
                return 0;
            };
            if let Err(error) = result {
                eprintln!("nokin: {error}");
            }
            1
        }
    }

    unsafe extern "C" fn on_editor_key_release(
        _widget: Widget,
        event: *const KeyEvent,
        data: *mut c_void,
    ) -> c_int {
        // SAFETY: callback data and event are valid for the duration of this signal.
        unsafe {
            let state = &*data.cast::<AppState>();
            let Some(tab) = state.active_tab() else {
                return 0;
            };
            match (*event).keyval {
                GDK_KEY_RETURN | GDK_KEY_KP_ENTER => {
                    tab.editor
                        .indent_after_newline(state.settings.editor.tab_width);
                }
                GDK_KEY_CLOSING_BRACE => {
                    tab.editor
                        .dedent_closing_brace(state.settings.editor.tab_width);
                }
                _ => {}
            }
            if tab.path.as_deref().is_some_and(is_c_file)
                && let Err(error) = tab.editor.refresh_c_function_highlighting()
            {
                eprintln!("nokin: {error}");
            }
            0
        }
    }

    unsafe extern "C" fn on_editor_button_press(
        widget: Widget,
        event: *const ButtonEvent,
        data: *mut c_void,
    ) -> c_int {
        // SAFETY: callback data and event are valid for the duration of this signal.
        let event = unsafe { &*event };
        if event.button == 1 && event.state & GDK_CONTROL_MASK != 0 {
            let state = unsafe { &mut *data.cast::<AppState>() };
            state.pending_definition = state
                .tabs
                .iter()
                .find(|tab| tab.editor.widget() == widget)
                .and_then(|tab| tab.editor.word_at_point(event.x, event.y));
            state.pending_lsp_position = state
                .tabs
                .iter()
                .find(|tab| tab.editor.widget() == widget)
                .and_then(|tab| {
                    let (line, column) = tab.editor.line_column_at_point(event.x, event.y)?;
                    Some((tab.path.clone()?, line, column))
                });
            if state.pending_definition.is_some() {
                unsafe { g_idle_add(on_goto_definition_idle, data) };
            }
            return 1;
        }
        0
    }

    unsafe extern "C" fn on_open_file_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).open_file_dialog() } {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_new_file_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).new_tab() } {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_create_file_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).create_explorer_entry(false) }
        {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_create_folder_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).create_explorer_entry(true) } {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_refresh_explorer_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).refresh_explorer() } {
            eprintln!("nokin: could not refresh explorer: {error}");
        }
    }

    unsafe extern "C" fn on_open_folder_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).open_folder_dialog() } {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_close_tab(_button: Widget, data: *mut c_void) {
        // SAFETY: close_data remains in AppState.close_data for the tab's lifetime.
        unsafe {
            let (page, state) = {
                let d = &*data.cast::<CloseTabData>();
                (d.page, &mut *d.state)
            };
            let page_num = gtk_notebook_page_num(state.notebook, page);
            if page_num >= 0 {
                gtk_notebook_remove_page(state.notebook, page_num);
            }
            state.tabs.retain(|tab| tab.page != page);
            state
                .close_data
                .retain(|d| d.as_ref() as *const CloseTabData as *mut c_void != data);
            if state.tabs.is_empty() {
                state.restore_welcome_page();
            }
        }
    }

    unsafe extern "C" fn on_close_all_tabs_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        unsafe { (&mut *data.cast::<AppState>()).close_all_tabs() };
    }

    unsafe extern "C" fn on_save_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).save_active() } {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_quit_activate(_item: Widget, _data: *mut c_void) {
        // SAFETY: called by GTK on its main thread.
        unsafe { gtk_main_quit() }
    }

    unsafe extern "C" fn on_run_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).run_active() } {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_configure_build_commands_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).configure_build_commands() } {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_configure_settings_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).configure_settings() } {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_toggle_explorer_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        unsafe { (&*data.cast::<AppState>()).toggle_explorer() };
    }

    unsafe extern "C" fn on_toggle_terminal_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        unsafe { (&*data.cast::<AppState>()).toggle_terminal() };
    }

    unsafe extern "C" fn on_goto_definition_activate(_item: Widget, data: *mut c_void) {
        // SAFETY: callback data points to AppState for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).goto_definition() } {
            eprintln!("nokin: {error}");
        }
    }

    unsafe extern "C" fn on_goto_definition_idle(data: *mut c_void) -> c_int {
        // SAFETY: AppState remains boxed for the duration of gtk_main.
        if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).goto_definition() } {
            eprintln!("nokin: {error}");
        }
        0
    }

    macro_rules! lsp_callback {
        ($name:ident, $method:ident) => {
            unsafe extern "C" fn $name(_item: Widget, data: *mut c_void) {
                // SAFETY: callback data points to AppState for the duration of gtk_main.
                if let Err(error) = unsafe { (&mut *data.cast::<AppState>()).$method() } {
                    eprintln!("nokin: {error}");
                }
            }
        };
    }

    lsp_callback!(on_lsp_references_activate, lsp_references);
    lsp_callback!(on_lsp_hover_activate, lsp_hover);
    lsp_callback!(on_lsp_signature_help_activate, lsp_signature_help);
    lsp_callback!(on_lsp_code_actions_activate, lsp_code_actions);
    lsp_callback!(on_lsp_rename_activate, lsp_rename);
    lsp_callback!(on_lsp_format_activate, lsp_format);
    lsp_callback!(on_lsp_diagnostics_activate, lsp_diagnostics);
    lsp_callback!(on_lsp_semantic_tokens_activate, lsp_semantic_tokens);

    macro_rules! edit_callback {
        ($name:ident, $action:expr) => {
            unsafe extern "C" fn $name(_item: Widget, data: *mut c_void) {
                // SAFETY: callback data points to AppState for the duration of gtk_main.
                unsafe { (&*data.cast::<AppState>()).edit_active($action) };
            }
        };
    }

    edit_callback!(on_undo_activate, EditAction::Undo);
    edit_callback!(on_redo_activate, EditAction::Redo);
    edit_callback!(on_cut_activate, EditAction::Cut);
    edit_callback!(on_copy_activate, EditAction::Copy);
    edit_callback!(on_paste_activate, EditAction::Paste);

    unsafe extern "C" fn on_test_expand_row(
        _tree: Widget,
        iter: *mut TreeIter,
        _path: TreePath,
        data: *mut c_void,
    ) -> c_int {
        // SAFETY: callback data and iter belong to the live explorer model.
        unsafe {
            let state = &mut *data.cast::<AppState>();
            let Some(directory) = iter_path(state.tree_store, iter) else {
                return 0;
            };
            let mut child = empty_iter();
            while gtk_tree_model_iter_children(state.tree_store, &mut child, iter) != 0 {
                gtk_tree_store_remove(state.tree_store, &mut child);
            }
            if let Err(error) = append_directory_children(state.tree_store, iter, &directory) {
                eprintln!("nokin: could not read {}: {error}", directory.display());
            }
            0
        }
    }

    unsafe fn model_path(model: TreeModel, path: TreePath) -> Option<PathBuf> {
        let mut iter = empty_iter();
        // SAFETY: path belongs to model during the signal callback.
        if unsafe { gtk_tree_model_get_iter(model, &mut iter, path) } == 0 {
            None
        } else {
            // SAFETY: iter was initialized by GTK for this model.
            unsafe { iter_path(model, &iter) }
        }
    }

    unsafe fn iter_path(model: TreeModel, iter: *const TreeIter) -> Option<PathBuf> {
        let mut text: *mut c_char = ptr::null_mut();
        // SAFETY: column 2 is a G_TYPE_STRING and GTK allocates the returned string.
        unsafe { gtk_tree_model_get(model, iter, 2, &mut text, -1) };
        if text.is_null() {
            return None;
        }
        // SAFETY: text is a GTK-allocated NUL-terminated path copied before release.
        let path =
            unsafe { PathBuf::from(std::ffi::OsStr::from_bytes(CStr::from_ptr(text).to_bytes())) };
        // SAFETY: GTK allocated text for gtk_tree_model_get.
        unsafe { g_free(text.cast()) };
        (!path.as_os_str().is_empty()).then_some(path)
    }

    unsafe fn initialize() -> io::Result<()> {
        // SAFETY: called on the GTK main thread before creating widgets.
        if unsafe { gtk_init_check(ptr::null_mut(), ptr::null_mut()) } == 0 {
            Err(io::Error::other(
                "GTK3 initialization failed; ensure a display is available",
            ))
        } else {
            Ok(())
        }
    }

    unsafe fn connect(widget: Widget, signal: &str, callback: *const c_void, data: *mut c_void) {
        let signal = CString::new(signal).unwrap();
        // SAFETY: callback ABI matches the named GObject signal and data remains live.
        unsafe { g_signal_connect_data(widget, signal.as_ptr(), callback, data, None, 0) };
    }

    unsafe extern "C" fn on_destroy() {
        // SAFETY: called by GTK on its main thread.
        unsafe { gtk_main_quit() }
    }

    unsafe fn label(text: &str) -> Widget {
        let text = CString::new(text).unwrap_or_else(|_| CString::new("invalid text").unwrap());
        // SAFETY: GTK copies the label text during this call.
        unsafe { gtk_label_new(text.as_ptr()) }
    }

    fn empty_iter() -> TreeIter {
        TreeIter {
            stamp: 0,
            user_data: ptr::null_mut(),
            user_data2: ptr::null_mut(),
            user_data3: ptr::null_mut(),
        }
    }

    fn is_c_file(path: &Path) -> bool {
        matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("c" | "h")
        )
    }

    fn lexer_for_path(path: &Path) -> Option<&'static str> {
        let file_name = path.file_name()?.to_str()?;
        match file_name {
            "Makefile" | "makefile" | "GNUmakefile" => return Some("makefile"),
            "CMakeLists.txt" => return Some("cmake"),
            _ => {}
        }
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "java" | "js" | "jsx" | "ts" | "tsx"
            | "cs" => Some("cpp"),
            "rs" => Some("rust"),
            "py" | "pyw" => Some("python"),
            "sh" | "bash" | "zsh" => Some("bash"),
            "html" | "htm" => Some("hypertext"),
            "xml" | "svg" => Some("xml"),
            "json" => Some("json"),
            "yaml" | "yml" => Some("yaml"),
            "toml" => Some("toml"),
            "md" | "markdown" => Some("markdown"),
            "css" => Some("css"),
            "sql" => Some("sql"),
            "lua" => Some("lua"),
            "rb" => Some("ruby"),
            "pl" | "pm" => Some("perl"),
            "php" => Some("phpscript"),
            "go" => Some("cpp"),
            "nix" => Some("nix"),
            "zig" => Some("zig"),
            "dart" => Some("dart"),
            "pas" => Some("pascal"),
            "asm" | "s" => Some("asm"),
            "tex" => Some("latex"),
            "diff" | "patch" => Some("diff"),
            "cmake" => Some("cmake"),
            _ => None,
        }
    }

    fn build_index(workspace: &Workspace) -> io::Result<Index> {
        let mut index = Index::default();
        for file in workspace
            .c_files()?
            .into_iter()
            .chain(workspace.rs_files()?)
        {
            if let Ok(source) = fs::read_to_string(&file) {
                index.update(&file, &source);
            }
        }
        Ok(index)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn maps_common_file_extensions_to_lexilla_lexers() {
            assert_eq!(lexer_for_path(Path::new("main.rs")), Some("rust"));
            assert_eq!(lexer_for_path(Path::new("script.py")), Some("python"));
            assert_eq!(lexer_for_path(Path::new("Makefile")), Some("makefile"));
            assert_eq!(lexer_for_path(Path::new("notes.txt")), None);
        }

        #[test]
        fn system_css_keeps_gtk_colors_and_scheme_css_overrides_them() {
            let mut settings = Settings::default();
            settings.ui.font_family = "Font \"Quoted\"".into();
            settings.ui.scale = 1.25;
            let palette = crate::theme::load_scheme("tango-dark").ui;

            let system = appearance_css(&settings, &palette);
            assert!(system.contains("Font \\\"Quoted\\\""));
            assert!(system.contains("12.50pt"));
            assert!(!system.contains("background-color"));

            settings.ui.theme_mode = UiThemeMode::ColorScheme;
            let themed = appearance_css(&settings, &palette);
            assert!(themed.contains("background-color"));
            assert!(themed.contains(&css_color(palette.background)));
        }
    }
}
