#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::{CStr, CString, c_char, c_int, c_long, c_void};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    type Widget = *mut c_void;
    type TerminalNew = unsafe extern "C" fn() -> Widget;
    type TerminalFeedChild = unsafe extern "C" fn(Widget, *const c_char, c_long);
    type TerminalSetColor = unsafe extern "C" fn(Widget, *const Rgba);
    type TerminalSetColors =
        unsafe extern "C" fn(Widget, *const Rgba, *const Rgba, *const Rgba, usize);
    type TerminalSpawnAsync = unsafe extern "C" fn(
        Widget,
        c_int,
        *const c_char,
        *mut *mut c_char,
        *mut *mut c_char,
        c_int,
        Option<unsafe extern "C" fn(*mut c_void)>,
        *mut c_void,
        Option<unsafe extern "C" fn(*mut c_void)>,
        c_int,
        *mut c_void,
        Option<unsafe extern "C" fn(Widget, c_long, *mut c_void, *mut c_void)>,
        *mut c_void,
    );

    #[repr(C)]
    struct Rgba {
        red: f64,
        green: f64,
        blue: f64,
        alpha: f64,
    }

    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
        fn dlerror() -> *const c_char;
    }

    const RTLD_NOW: c_int = 2;

    pub struct Terminal {
        widget: Widget,
        feed_child: TerminalFeedChild,
        set_foreground: TerminalSetColor,
        set_background: TerminalSetColor,
        set_colors: TerminalSetColors,
        _library: Library,
    }

    impl Terminal {
        pub fn new(
            workspace: &Path,
            shell: &str,
            colors: &crate::theme::TerminalPalette,
        ) -> io::Result<Self> {
            // SAFETY: symbols are loaded from VTE's stable GTK3 ABI and retained for the
            // lifetime of the returned terminal.
            unsafe {
                let library = Library::open("libvte-2.91.so.0")?;
                let terminal_new: TerminalNew = library.symbol("vte_terminal_new")?;
                let feed_child = library.symbol("vte_terminal_feed_child")?;
                let spawn_async: TerminalSpawnAsync = library.symbol("vte_terminal_spawn_async")?;
                let set_foreground: TerminalSetColor =
                    library.symbol("vte_terminal_set_color_foreground")?;
                let set_background: TerminalSetColor =
                    library.symbol("vte_terminal_set_color_background")?;
                let set_colors: TerminalSetColors = library.symbol("vte_terminal_set_colors")?;
                let widget = terminal_new();
                if widget.is_null() {
                    return Err(io::Error::other("VTE terminal creation failed"));
                }
                let workspace = cstring_path(workspace)?;
                let shell = CString::new(shell)
                    .map_err(|_| io::Error::other("terminal shell contains a NUL byte"))?;
                let mut argv = [shell.as_ptr().cast_mut(), ptr::null_mut()];
                spawn_async(
                    widget,
                    0,
                    workspace.as_ptr(),
                    argv.as_mut_ptr(),
                    ptr::null_mut(),
                    0,
                    None,
                    ptr::null_mut(),
                    None,
                    -1,
                    ptr::null_mut(),
                    None,
                    ptr::null_mut(),
                );
                let terminal = Self {
                    widget,
                    feed_child,
                    set_foreground,
                    set_background,
                    set_colors,
                    _library: library,
                };
                terminal.apply_palette(colors);
                Ok(terminal)
            }
        }

        pub fn widget(&self) -> Widget {
            self.widget
        }

        pub fn feed_command(&self, command: &str) -> io::Result<()> {
            let command = CString::new(format!("{command}\n"))
                .map_err(|_| io::Error::other("terminal command contains a NUL byte"))?;
            // SAFETY: widget is a live VTE terminal and VTE copies the specified bytes.
            unsafe { (self.feed_child)(self.widget, command.as_ptr(), -1) }
            Ok(())
        }

        pub fn apply_palette(&self, colors: &crate::theme::TerminalPalette) {
            let foreground = rgba(colors.foreground);
            let background = rgba(colors.background);
            let palette = colors.ansi.map(rgba);
            // SAFETY: widget and function pointers remain valid while the VTE library is retained.
            unsafe {
                (self.set_foreground)(self.widget, &foreground);
                (self.set_background)(self.widget, &background);
                (self.set_colors)(
                    self.widget,
                    &foreground,
                    &background,
                    palette.as_ptr(),
                    palette.len(),
                );
            }
        }
    }

    fn rgba(rgb: usize) -> Rgba {
        Rgba {
            red: ((rgb >> 16) & 0xff) as f64 / 255.0,
            green: ((rgb >> 8) & 0xff) as f64 / 255.0,
            blue: (rgb & 0xff) as f64 / 255.0,
            alpha: 1.0,
        }
    }

    struct Library(*mut c_void);

    impl Library {
        unsafe fn open(filename: &str) -> io::Result<Self> {
            let filename = CString::new(filename).unwrap();
            // SAFETY: filename is NUL-terminated and flags are valid for dlopen.
            let handle = unsafe { dlopen(filename.as_ptr(), RTLD_NOW) };
            if handle.is_null() {
                Err(last_dl_error())
            } else {
                Ok(Self(handle))
            }
        }

        unsafe fn symbol<T: Copy>(&self, name: &str) -> io::Result<T> {
            let name = CString::new(name).unwrap();
            // SAFETY: handle is live and name is NUL-terminated.
            let symbol = unsafe { dlsym(self.0, name.as_ptr()) };
            if symbol.is_null() {
                Err(last_dl_error())
            } else {
                // SAFETY: caller specifies the ABI type for the named VTE symbol.
                Ok(unsafe { std::mem::transmute_copy(&symbol) })
            }
        }
    }

    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: handle was returned by dlopen and is dropped once.
            unsafe { dlclose(self.0) };
        }
    }

    fn cstring_path(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::other("workspace path contains a NUL byte"))
    }

    fn last_dl_error() -> io::Error {
        // SAFETY: dlerror returns either NULL or a process-owned NUL-terminated error string.
        let message = unsafe {
            let error = dlerror();
            if error.is_null() {
                "dynamic library error".into()
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            }
        };
        io::Error::other(message)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn converts_theme_rgb_to_rgba() {
            let color = rgba(0x80_40_20);
            assert!((color.red - 128.0 / 255.0).abs() < f64::EPSILON);
            assert!((color.green - 64.0 / 255.0).abs() < f64::EPSILON);
            assert!((color.blue - 32.0 / 255.0).abs() < f64::EPSILON);
            assert_eq!(color.alpha, 1.0);
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::Terminal;
