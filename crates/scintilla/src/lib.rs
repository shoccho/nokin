use std::cell::Cell;
use std::ffi::{CString, c_void};
use std::io;
use std::ptr::NonNull;

use scintilla_sys as sys;

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub default_fg: usize,
    pub default_bg: usize,
    pub comment: usize,
    pub comment_bold: bool,
    pub comment_italic: bool,
    pub number: usize,
    pub number_bold: bool,
    pub number_italic: bool,
    pub keyword: usize,
    pub keyword_bold: bool,
    pub keyword_italic: bool,
    pub string: usize,
    pub string_bold: bool,
    pub string_italic: bool,
    pub string_eol_bg: usize,
    pub preprocessor: usize,
    pub preprocessor_bold: bool,
    pub preprocessor_italic: bool,
    pub type_color: usize,
    pub type_bold: bool,
    pub type_italic: bool,
    pub function: usize,
    pub function_bold: bool,
    pub function_italic: bool,
    pub selection_fg: usize,
    pub selection_bg: usize,
    pub margin_fg: usize,
    pub caret: usize,
    pub caret_width: usize,
    pub current_line_bg: usize,
    pub current_line_visible: bool,
    pub extra_ascent: i32,
    pub extra_descent: i32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            default_fg: 0xdbdbdb,
            default_bg: 0x1c1c1c,
            comment: 0xadadad,
            comment_bold: false,
            comment_italic: false,
            number: 0x8ad1ff,
            number_bold: false,
            number_italic: false,
            keyword: 0xbf6069,
            keyword_bold: true,
            keyword_italic: false,
            string: 0x6bb37c,
            string_bold: false,
            string_italic: false,
            string_eol_bg: 0x6e006e,
            preprocessor: 0x45bde6,
            preprocessor_bold: false,
            preprocessor_italic: false,
            type_color: 0x50aab3,
            type_bold: false,
            type_italic: false,
            function: 0xcc8ad4,
            function_bold: false,
            function_italic: false,
            selection_fg: 0x000000,
            selection_bg: 0xe7a96b,
            margin_fg: 0x6e6e6e,
            caret: 0xffffff,
            caret_width: 1,
            current_line_bg: 0x262626,
            current_line_visible: true,
            extra_ascent: 0,
            extra_descent: 0,
        }
    }
}

pub struct Editor {
    widget: NonNull<c_void>,
    function_style: Cell<(usize, bool, bool)>,
}

impl Editor {
    /// Creates a GTK Scintilla widget. GTK must already be initialized on this thread.
    pub fn new() -> io::Result<Self> {
        // SAFETY: GTK initialization is the caller's responsibility. The returned widget is
        // owned by GTK after it is inserted into a container.
        let widget = unsafe { sys::scintilla_new() };
        NonNull::new(widget)
            .map(|widget| Self {
                widget,
                function_style: Cell::new((
                    Palette::default().function,
                    Palette::default().function_bold,
                    Palette::default().function_italic,
                )),
            })
            .ok_or_else(|| io::Error::other("Scintilla widget creation failed"))
    }

    pub fn widget(&self) -> *mut c_void {
        self.widget.as_ptr()
    }

    pub fn configure_c_lexer(&self, palette: &Palette) -> io::Result<()> {
        self.set_lexer("cpp")?;
        self.set_property("styling.within.preprocessor", "1")?;
        self.set_property("lexer.cpp.track.preprocessor", "0")?;
        self.set_property("fold", "1")?;
        self.set_property("fold.compact", "0")?;
        self.set_property("fold.comment", "1")?;
        self.set_property("fold.preprocessor", "1")?;
        self.set_property("fold.at.else", "1")?;
        self.set_keywords(
            0,
            "alignas alignof auto bool break case char const constexpr continue default do \
             double else enum extern false float for goto if inline int long nullptr register \
             restrict return short signed sizeof static static_assert struct switch thread_local \
             true typedef typeof union unsigned void volatile while",
        )?;
        let function_style = self.send(sys::SCI_ALLOCATESUBSTYLES, 11, 1);
        if function_style >= 0 {
            self.set_style(
                function_style as usize,
                palette.function,
                None,
                palette.function_bold,
                palette.function_italic,
            );
        }
        self.configure_folding(palette);
        self.refresh_c_function_highlighting()?;
        self.send(sys::SCI_COLOURISE, 0, -1);
        Ok(())
    }

    pub fn configure_basic_lexer(&self, lexer_name: &str, palette: &Palette) -> io::Result<()> {
        self.set_lexer(lexer_name)?;
        self.set_property("fold", "1")?;
        self.set_property("fold.compact", "0")?;
        if lexer_name == "rust" {
            self.configure_rust_lexer(palette)?;
        } else {
            self.apply_basic_lexer_styles(palette);
        }
        self.configure_folding(palette);
        self.send(sys::SCI_COLOURISE, 0, -1);
        Ok(())
    }

    pub fn refresh_c_function_highlighting(&self) -> io::Result<()> {
        let function_style = self.send(sys::SCI_GETSUBSTYLESSTART, 11, 0);
        if function_style < 0 {
            return Ok(());
        }
        let (color, bold, italic) = self.function_style.get();
        self.set_style(
            function_style as usize,
            color,
            None,
            bold,
            italic,
        );
        let identifiers = c_function_identifiers(&self.text_bytes()).join(" ");
        let identifiers = CString::new(identifiers)
            .map_err(|_| io::Error::other("function identifier contains a NUL byte"))?;
        self.send(
            sys::SCI_SETIDENTIFIERS,
            function_style as usize,
            identifiers.as_ptr() as isize,
        );
        Ok(())
    }

    pub fn apply_palette(&self, palette: &Palette) {
        self.function_style
            .set((palette.function, palette.function_bold, palette.function_italic));

        const DEFAULT: usize = 32;
        self.set_style(DEFAULT, palette.default_fg, Some(palette.default_bg), false, false);
        self.send(sys::SCI_STYLECLEARALL, 0, 0);
        self.send(sys::SCI_SETCARETFORE, bgr(palette.caret) as usize, 0);
        self.send(sys::SCI_SETCARETWIDTH, palette.caret_width, 0);
        self.send(sys::SCI_SETCARETLINEBACK, bgr(palette.current_line_bg) as usize, 0);
        self.send(sys::SCI_SETCARETLINEVISIBLE, usize::from(palette.current_line_visible), 0);
        self.send(sys::SCI_SETEXTRAASCENT, palette.extra_ascent as usize, 0);
        self.send(sys::SCI_SETEXTRADESCENT, palette.extra_descent as usize, 0);
        self.send(sys::SCI_SETSELFORE, 1, bgr(palette.selection_fg));
        self.send(sys::SCI_SETSELBACK, 1, bgr(palette.selection_bg));
        self.set_style(33, palette.margin_fg, Some(palette.default_bg), false, false);
        self.send(sys::SCI_SETMARGINBACKN, 0, bgr(palette.default_bg));
        self.send(sys::SCI_SETMARGINBACKN, 1, bgr(palette.default_bg));
        self.send(sys::SCI_SETMARGINBACKN, 2, bgr(palette.default_bg));

        // C/generic language style mapping (overridden by basic/rust lexer styles for non-C files)
        for style in [1, 2, 3, 15, 23, 24, 26] {
            self.set_style(
                style,
                palette.comment,
                None,
                palette.comment_bold,
                palette.comment_italic,
            );
        }
        self.set_style(4, palette.number, None, palette.number_bold, palette.number_italic);
        self.set_style(
            5,
            palette.keyword,
            None,
            palette.keyword_bold,
            palette.keyword_italic,
        );
        for style in [6, 7, 13, 20, 21, 22, 27] {
            self.set_style(
                style,
                palette.string,
                None,
                palette.string_bold,
                palette.string_italic,
            );
        }
        self.set_style(
            9,
            palette.preprocessor,
            None,
            palette.preprocessor_bold,
            palette.preprocessor_italic,
        );
        self.set_style(
            12,
            palette.string,
            Some(palette.string_eol_bg),
            palette.string_bold,
            palette.string_italic,
        );
        self.set_style(
            16,
            palette.keyword,
            None,
            palette.keyword_bold,
            palette.keyword_italic,
        );
        self.set_style(17, palette.comment, None, true, false);
        self.set_style(18, palette.comment, None, false, true);
        self.set_style(19, palette.type_color, None, palette.type_bold, palette.type_italic);
    }

    pub fn set_line_number_margin(&self, pixels: usize) {
        self.send(sys::SCI_SETMARGINWIDTHN, 0, pixels as isize);
    }

    pub fn configure_indentation(&self, width: usize, insert_spaces: bool) {
        self.send(sys::SCI_SETTABWIDTH, width, 0);
        self.send(sys::SCI_SETINDENT, width, 0);
        self.send(sys::SCI_SETUSETABS, usize::from(!insert_spaces), 0);
        self.send(sys::SCI_SETTABINDENTS, 1, 0);
        self.send(sys::SCI_SETBACKSPACEUNINDENTS, usize::from(insert_spaces), 0);
        self.send(sys::SCI_SETINDENTATIONGUIDES, 1, 0);
    }

    pub fn set_font(&self, style: usize, family: &str, fractional_points: f64) -> io::Result<()> {
        let family = CString::new(family)
            .map_err(|_| io::Error::other("font family contains a NUL byte"))?;
        self.send(sys::SCI_STYLESETFONT, style, family.as_ptr() as isize);
        self.send(
            sys::SCI_STYLESETSIZEFRACTIONAL,
            style,
            (fractional_points * 100.0).round() as isize,
        );
        self.send(sys::SCI_STYLECLEARALL, 0, 0);
        Ok(())
    }

    pub fn set_ligatures(&self, enabled: bool) {
        // SAFETY: this updates the GTK Scintilla backend's process-wide Pango shaping option.
        unsafe { sys::scintilla_set_ligatures(i32::from(enabled)) };
        self.send(sys::SCI_STYLECLEARALL, 0, 0);
    }

    pub fn set_text(&self, text: &str) -> io::Result<()> {
        self.replace_text(text)?;
        self.set_save_point();
        Ok(())
    }

    pub fn replace_text(&self, text: &str) -> io::Result<()> {
        let text = CString::new(text).map_err(|_| io::Error::other("text contains a NUL byte"))?;
        self.send(sys::SCI_SETTEXT, 0, text.as_ptr() as isize);
        Ok(())
    }

    pub fn text_bytes(&self) -> Vec<u8> {
        let length = self.send(sys::SCI_GETTEXTLENGTH, 0, 0).max(0) as usize;
        let mut bytes = vec![0; length + 1];
        self.send(sys::SCI_GETTEXT, bytes.len(), bytes.as_mut_ptr() as isize);
        bytes.truncate(length);
        bytes
    }

    pub fn set_save_point(&self) {
        self.send(sys::SCI_SETSAVEPOINT, 0, 0);
    }

    pub fn undo(&self) {
        self.send(sys::SCI_UNDO, 0, 0);
    }

    pub fn redo(&self) {
        self.send(sys::SCI_REDO, 0, 0);
    }

    pub fn cut(&self) {
        self.send(sys::SCI_CUT, 0, 0);
    }

    pub fn copy(&self) {
        self.send(sys::SCI_COPY, 0, 0);
    }

    pub fn paste(&self) {
        self.send(sys::SCI_PASTE, 0, 0);
    }

    pub fn show_completion(&self, entries: &[String]) -> io::Result<()> {
        let list = CString::new(entries.join(" "))
            .map_err(|_| io::Error::other("completion label contains a NUL byte"))?;
        let prefix = self.current_word().map_or(0, |word| word.len());
        self.send(sys::SCI_AUTOCSHOW, prefix, list.as_ptr() as isize);
        Ok(())
    }

    pub fn show_calltip(&self, text: &str) -> io::Result<()> {
        let text =
            CString::new(text).map_err(|_| io::Error::other("calltip contains a NUL byte"))?;
        let position = self.send(sys::SCI_GETCURRENTPOS, 0, 0).max(0) as usize;
        self.send(sys::SCI_CALLTIPSHOW, position, text.as_ptr() as isize);
        Ok(())
    }

    pub fn show_diagnostics(&self, positions: &[(usize, usize)]) {
        const INDICATOR: usize = 0;
        const INDIC_SQUIGGLE: isize = 1;
        self.send(sys::SCI_INDICSETSTYLE, INDICATOR, INDIC_SQUIGGLE);
        self.send(sys::SCI_INDICSETFORE, INDICATOR, bgr(0xbf6069));
        self.send(sys::SCI_SETINDICATORCURRENT, INDICATOR, 0);
        self.send(sys::SCI_INDICATORCLEARRANGE, 0, self.text_bytes().len() as isize);
        for &(line, column) in positions {
            let position = self.position_from_line_column(line, column);
            self.send(sys::SCI_INDICATORFILLRANGE, position, 1);
        }
    }

    pub fn show_semantic_tokens(&self, tokens: &[(usize, usize, usize, &str)]) {
        const INDIC_TEXTFORE: isize = 17;
        const FIRST_INDICATOR: usize = 1;
        const COLORS: &[(&str, usize)] = &[
            ("function", 0xcc8ad4),
            ("method", 0xcc8ad4),
            ("type", 0x50aab3),
            ("class", 0x50aab3),
            ("struct", 0x50aab3),
            ("enum", 0x50aab3),
            ("parameter", 0x8ad1ff),
            ("variable", 0xdbdbdb),
            ("macro", 0x45bde6),
        ];
        let length = self.text_bytes().len() as isize;
        for (offset, &(_, color)) in COLORS.iter().enumerate() {
            let indicator = FIRST_INDICATOR + offset;
            self.send(sys::SCI_INDICSETSTYLE, indicator, INDIC_TEXTFORE);
            self.send(sys::SCI_INDICSETFORE, indicator, bgr(color));
            self.send(sys::SCI_INDICSETUNDER, indicator, 0);
            self.send(sys::SCI_SETINDICATORCURRENT, indicator, 0);
            self.send(sys::SCI_INDICATORCLEARRANGE, 0, length);
        }
        for &(line, column, length, kind) in tokens {
            let Some(offset) = COLORS.iter().position(|&(candidate, _)| candidate == kind) else {
                continue;
            };
            self.send(sys::SCI_SETINDICATORCURRENT, FIRST_INDICATOR + offset, 0);
            self.send(
                sys::SCI_INDICATORFILLRANGE,
                self.position_from_line_column(line, column),
                length as isize,
            );
        }
    }

    pub fn select_next_occurrence(&self) {
        self.send(sys::SCI_SETMULTIPLESELECTION, 1, 0);
        self.send(sys::SCI_SETADDITIONALSELECTIONTYPING, 1, 0);
        let mut start = self.send(sys::SCI_GETSELECTIONSTART, 0, 0).max(0) as usize;
        let mut end = self.send(sys::SCI_GETSELECTIONEND, 0, 0).max(0) as usize;
        if start == end {
            let cursor = self.send(sys::SCI_GETCURRENTPOS, 0, 0).max(0) as usize;
            start = self.send(sys::SCI_WORDSTARTPOSITION, cursor, 1).max(0) as usize;
            end = self.send(sys::SCI_WORDENDPOSITION, cursor, 1).max(0) as usize;
            if start < end {
                self.send(sys::SCI_SETSELECTION, end, start as isize);
            }
            return;
        }

        let bytes = self.text_bytes();
        let selections = self.selection_ranges();
        let main = self.send(sys::SCI_GETMAINSELECTION, 0, 0).max(0) as usize;
        let search_from = selections.get(main).map(|(_, end)| *end).unwrap_or(end);
        if let Some((start, end)) =
            next_unselected_match(&bytes, &bytes[start..end], search_from, &selections)
        {
            self.send(sys::SCI_ADDSELECTION, end, start as isize);
        }
    }

    pub fn current_word(&self) -> Option<String> {
        let cursor = self.send(sys::SCI_GETCURRENTPOS, 0, 0).max(0) as usize;
        self.word_at_position(cursor)
    }

    pub fn word_at_point(&self, x: f64, y: f64) -> Option<String> {
        let position = self.send(
            sys::SCI_POSITIONFROMPOINTCLOSE,
            x.max(0.0) as usize,
            y.max(0.0) as isize,
        );
        (position >= 0)
            .then_some(position as usize)
            .and_then(|position| self.word_at_position(position))
    }

    fn word_at_position(&self, position: usize) -> Option<String> {
        let bytes = self.text_bytes();
        let cursor = position.min(bytes.len());
        let mut start = cursor;
        let mut end = cursor;
        while start > 0 && is_identifier_byte(bytes[start - 1]) {
            start -= 1;
        }
        while end < bytes.len() && is_identifier_byte(bytes[end]) {
            end += 1;
        }
        (start < end)
            .then(|| String::from_utf8_lossy(&bytes[start..end]).into_owned())
    }

    fn selection_ranges(&self) -> Vec<(usize, usize)> {
        let count = self.send(sys::SCI_GETSELECTIONS, 0, 0).max(0) as usize;
        (0..count)
            .map(|selection| {
                let start = self
                    .send(sys::SCI_GETSELECTIONNSTART, selection, 0)
                    .max(0) as usize;
                let end = self
                    .send(sys::SCI_GETSELECTIONNEND, selection, 0)
                    .max(0) as usize;
                (start, end)
            })
            .collect()
    }

    pub fn goto_line(&self, one_based_line: usize) {
        self.send(sys::SCI_GOTOLINE, one_based_line.saturating_sub(1), 0);
    }

    pub fn cursor_line_column(&self) -> (usize, usize) {
        let position = self.send(sys::SCI_GETCURRENTPOS, 0, 0).max(0) as usize;
        self.line_column(position)
    }

    pub fn line_column_at_point(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let position = self.send(
            sys::SCI_POSITIONFROMPOINTCLOSE,
            x.max(0.0) as usize,
            y.max(0.0) as isize,
        );
        (position >= 0).then(|| self.line_column(position as usize))
    }

    pub fn indent_after_newline(&self, width: usize) {
        let line = self.current_line();
        if line == 0 {
            return;
        }
        let previous = line - 1;
        let mut indentation = self.line_indentation(previous);
        if self.last_non_whitespace(previous) == Some(b'{') {
            indentation += width;
        }
        if self.first_non_whitespace(line) == Some(b'}') {
            indentation = indentation.saturating_sub(width);
        }
        self.send(sys::SCI_SETLINEINDENTATION, line, indentation as isize);
    }

    pub fn dedent_closing_brace(&self, width: usize) {
        let line = self.current_line();
        let position = self.send(sys::SCI_GETCURRENTPOS, 0, 0).max(0) as usize;
        let indent_position = self
            .send(sys::SCI_GETLINEINDENTPOSITION, line, 0)
            .max(0) as usize;
        if position == indent_position + 1 && self.character_at(indent_position) == Some(b'}') {
            let indentation = self.line_indentation(line).saturating_sub(width);
            self.send(sys::SCI_SETLINEINDENTATION, line, indentation as isize);
        }
    }

    fn set_keywords(&self, set: usize, keywords: &str) -> io::Result<()> {
        let keywords =
            CString::new(keywords).map_err(|_| io::Error::other("keywords contain a NUL byte"))?;
        self.send(sys::SCI_SETKEYWORDS, set, keywords.as_ptr() as isize);
        Ok(())
    }

    fn set_lexer(&self, lexer_name: &str) -> io::Result<()> {
        let lexer_name = CString::new(lexer_name)
            .map_err(|_| io::Error::other("lexer name contains a NUL byte"))?;
        // SAFETY: Lexilla reads the NUL-terminated lexer name during the call and returns a
        // lexer whose ownership is transferred to Scintilla by SCI_SETILEXER.
        let lexer = unsafe { sys::CreateLexer(lexer_name.as_ptr()) };
        let lexer = NonNull::new(lexer)
            .ok_or_else(|| io::Error::other(format!("{lexer_name:?} lexer unavailable")))?;
        self.send(sys::SCI_SETILEXER, 0, lexer.as_ptr() as isize);
        Ok(())
    }

    fn set_property(&self, name: &str, value: &str) -> io::Result<()> {
        let name = CString::new(name).map_err(|_| io::Error::other("property contains a NUL"))?;
        let value =
            CString::new(value).map_err(|_| io::Error::other("property value contains a NUL"))?;
        self.send(
            sys::SCI_SETPROPERTY,
            name.as_ptr() as usize,
            value.as_ptr() as isize,
        );
        Ok(())
    }

    fn configure_rust_lexer(&self, palette: &Palette) -> io::Result<()> {
        self.set_keywords(
            0,
            "abstract alignof as async await become box break const continue crate do dyn else \
             enum extern false final fn for if impl in let loop macro match mod move mut offsetof \
             override priv proc pub pure ref return self sizeof static struct super trait true type \
             typeof unsafe unsized use virtual where while yield",
        )?;
        self.set_keywords(
            1,
            "bool char f32 f64 i128 i16 i32 i64 i8 isize str u128 u16 u32 u64 u8 usize",
        )?;
        self.set_keywords(2, "Self")?;
        self.apply_rust_lexer_styles(palette);
        Ok(())
    }

    fn current_line(&self) -> usize {
        let position = self.send(sys::SCI_GETCURRENTPOS, 0, 0);
        self.send(sys::SCI_LINEFROMPOSITION, position.max(0) as usize, 0)
            .max(0) as usize
    }

    fn line_column(&self, position: usize) -> (usize, usize) {
        let line = self
            .send(sys::SCI_LINEFROMPOSITION, position, 0)
            .max(0) as usize;
        let start = self.send(sys::SCI_POSITIONFROMLINE, line, 0).max(0) as usize;
        (line, position.saturating_sub(start))
    }

    fn position_from_line_column(&self, line: usize, column: usize) -> usize {
        self.send(sys::SCI_POSITIONFROMLINE, line, 0).max(0) as usize + column
    }

    fn line_indentation(&self, line: usize) -> usize {
        self.send(sys::SCI_GETLINEINDENTATION, line, 0).max(0) as usize
    }

    fn first_non_whitespace(&self, line: usize) -> Option<u8> {
        let position = self
            .send(sys::SCI_GETLINEINDENTPOSITION, line, 0)
            .max(0) as usize;
        self.character_at(position)
    }

    fn last_non_whitespace(&self, line: usize) -> Option<u8> {
        let start = self.send(sys::SCI_POSITIONFROMLINE, line, 0).max(0) as usize;
        let mut position = self
            .send(sys::SCI_GETLINEENDPOSITION, line, 0)
            .max(0) as usize;
        while position > start {
            position -= 1;
            let character = self.character_at(position)?;
            if !character.is_ascii_whitespace() {
                return Some(character);
            }
        }
        None
    }

    fn character_at(&self, position: usize) -> Option<u8> {
        let character = self.send(sys::SCI_GETCHARAT, position, 0);
        (character > 0).then_some(character as u8)
    }

    fn apply_basic_lexer_styles(&self, palette: &Palette) {
        for style in [1, 2, 3, 12, 15, 23, 24] {
            self.set_style(
                style,
                palette.comment,
                None,
                palette.comment_bold,
                palette.comment_italic,
            );
        }
        for style in [4, 8, 13] {
            self.set_style(style, palette.number, None, palette.number_bold, palette.number_italic);
        }
        for style in [5, 10, 16] {
            self.set_style(
                style,
                palette.keyword,
                None,
                palette.keyword_bold,
                palette.keyword_italic,
            );
        }
        for style in [6, 7, 11, 14, 20, 21, 22] {
            self.set_style(
                style,
                palette.string,
                None,
                palette.string_bold,
                palette.string_italic,
            );
        }
        self.set_style(
            9,
            palette.preprocessor,
            None,
            palette.preprocessor_bold,
            palette.preprocessor_italic,
        );
        self.set_style(
            17,
            palette.function,
            None,
            palette.function_bold,
            palette.function_italic,
        );
        self.set_style(18, palette.type_color, None, palette.type_bold, palette.type_italic);
    }

    fn apply_rust_lexer_styles(&self, palette: &Palette) {
        for style in [1, 2, 3, 4] {
            self.set_style(
                style,
                palette.comment,
                None,
                palette.comment_bold,
                palette.comment_italic,
            );
        }
        self.set_style(5, palette.number, None, palette.number_bold, palette.number_italic);
        for style in [6, 7, 8, 9, 10, 11, 12] {
            self.set_style(
                style,
                palette.keyword,
                None,
                palette.keyword_bold,
                palette.keyword_italic,
            );
        }
        for style in [13, 14, 15, 21, 22, 23] {
            self.set_style(
                style,
                palette.string,
                None,
                palette.string_bold,
                palette.string_italic,
            );
        }
        for style in [18, 19] {
            self.set_style(
                style,
                palette.preprocessor,
                None,
                palette.preprocessor_bold,
                palette.preprocessor_italic,
            );
        }
        self.set_style(
            20,
            palette.string,
            Some(palette.string_eol_bg),
            palette.string_bold,
            palette.string_italic,
        );
    }

    fn configure_folding(&self, palette: &Palette) {
        const SC_MARGIN_SYMBOL: usize = 0;
        const SC_MASK_FOLDERS: usize = 0xfe00_0000;
        const SC_AUTOMATICFOLD_SHOW_CLICK_CHANGE: usize = 0x0001 | 0x0002 | 0x0004;
        const MARKERS: &[(usize, isize)] = &[
            (25, 13),
            (26, 15),
            (27, 11),
            (28, 10),
            (29, 9),
            (30, 12),
            (31, 14),
        ];
        self.send(sys::SCI_SETMARGINTYPEN, 2, SC_MARGIN_SYMBOL as isize);
        self.send(sys::SCI_SETMARGINMASKN, 2, SC_MASK_FOLDERS as isize);
        self.send(sys::SCI_SETMARGINWIDTHN, 1, 0);
        self.send(sys::SCI_SETMARGINWIDTHN, 2, 14);
        self.send(sys::SCI_SETMARGINSENSITIVEN, 2, 1);
        self.send(sys::SCI_SETFOLDMARGINCOLOUR, 1, bgr(palette.default_bg));
        self.send(sys::SCI_SETFOLDMARGINHICOLOUR, 1, bgr(palette.default_bg));
        self.send(sys::SCI_SETFOLDFLAGS, 16, 0);
        self.send(
            sys::SCI_SETAUTOMATICFOLD,
            SC_AUTOMATICFOLD_SHOW_CLICK_CHANGE,
            0,
        );
        for &(marker, symbol) in MARKERS {
            self.send(sys::SCI_MARKERDEFINE, marker, symbol);
            self.send(sys::SCI_MARKERSETFORE, marker, bgr(palette.default_bg));
            self.send(sys::SCI_MARKERSETBACK, marker, bgr(palette.margin_fg));
        }
    }

    fn set_style(
        &self,
        style: usize,
        foreground: usize,
        background: Option<usize>,
        bold: bool,
        italic: bool,
    ) {
        self.send(sys::SCI_STYLESETFORE, style, bgr(foreground));
        if let Some(background) = background {
            self.send(sys::SCI_STYLESETBACK, style, bgr(background));
        }
        self.send(sys::SCI_STYLESETBOLD, style, isize::from(bold));
        self.send(sys::SCI_STYLESETITALIC, style, isize::from(italic));
    }

    fn send(&self, message: u32, w_param: usize, l_param: isize) -> isize {
        // SAFETY: widget is a live Scintilla GTK widget and parameters follow the message API.
        unsafe { sys::scintilla_send_message(self.widget.as_ptr(), message, w_param, l_param) }
    }
}

fn bgr(rgb: usize) -> isize {
    (((rgb & 0xff) << 16) | (rgb & 0x00ff00) | ((rgb >> 16) & 0xff)) as isize
}

fn is_identifier_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn c_function_identifiers(bytes: &[u8]) -> Vec<String> {
    let mut identifiers = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'_' || bytes[index].is_ascii_alphabetic() {
            let start = index;
            index += 1;
            while index < bytes.len() && is_identifier_byte(bytes[index]) {
                index += 1;
            }
            let mut after = index;
            while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                after += 1;
            }
            let identifier = &bytes[start..index];
            if after < bytes.len()
                && bytes[after] == b'('
                && !is_c_control_keyword(identifier)
            {
                identifiers.push(String::from_utf8_lossy(identifier).into_owned());
            }
        } else {
            index += 1;
        }
    }
    identifiers.sort();
    identifiers.dedup();
    identifiers
}

fn is_c_control_keyword(identifier: &[u8]) -> bool {
    matches!(
        identifier,
        b"if" | b"for" | b"while" | b"switch" | b"sizeof" | b"_Alignof" | b"alignof"
    )
}

fn next_unselected_match(
    bytes: &[u8],
    needle: &[u8],
    search_from: usize,
    selections: &[(usize, usize)],
) -> Option<(usize, usize)> {
    if needle.is_empty() || needle.len() > bytes.len() {
        return None;
    }
    let last_start = bytes.len() - needle.len();
    (search_from.min(bytes.len())..=last_start)
        .chain(0..search_from.min(last_start + 1))
        .find_map(|start| {
            let end = start + needle.len();
            (&bytes[start..end] == needle && !selections.contains(&(start, end)))
                .then_some((start, end))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_rgb_theme_colors_to_scintilla_bgr() {
        assert_eq!(bgr(0x12_34_56), 0x56_34_12);
    }

    #[test]
    fn identifies_c_word_bytes() {
        assert!(is_identifier_byte(b'_'));
        assert!(is_identifier_byte(b'9'));
        assert!(!is_identifier_byte(b'('));
    }

    #[test]
    fn extracts_local_c_function_names_without_control_keywords() {
        assert_eq!(
            c_function_identifiers(b"int helper(void) { if (ready()) return helper(); }"),
            vec!["helper", "ready"]
        );
    }

    #[test]
    fn finds_next_unselected_match_with_wraparound() {
        let bytes = b"one two one";
        assert_eq!(
            next_unselected_match(bytes, b"one", 3, &[(0, 3)]),
            Some((8, 11))
        );
        assert_eq!(
            next_unselected_match(bytes, b"one", 11, &[(8, 11)]),
            Some((0, 3))
        );
        assert_eq!(
            next_unselected_match(bytes, b"one", 11, &[(0, 3), (8, 11)]),
            None
        );
    }
}
