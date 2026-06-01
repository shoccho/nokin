#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub cursor: usize,
}

pub fn type_character(before: &str, after: &str, typed: char) -> Edit {
    if ")]}'\"".contains(typed) && after.starts_with(typed) {
        return Edit {
            text: format!("{before}{after}"),
            cursor: before.len() + typed.len_utf8(),
        };
    }
    let close = match typed {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '\'' => Some('\''),
        '"' => Some('"'),
        _ => None,
    };
    let insertion = close
        .map(|close| format!("{typed}{close}"))
        .unwrap_or_else(|| typed.to_string());
    Edit {
        text: format!("{before}{insertion}{after}"),
        cursor: before.len() + typed.len_utf8(),
    }
}

pub fn newline_indent(line_before_cursor: &str, tab_width: usize, insert_spaces: bool) -> String {
    let unit = if insert_spaces {
        " ".repeat(tab_width)
    } else {
        "\t".into()
    };
    let prefix: String = line_before_cursor
        .chars()
        .take_while(|character| character.is_whitespace())
        .collect();
    if line_before_cursor.trim_end().ends_with('{') {
        format!("{prefix}{unit}")
    } else {
        prefix
    }
}

pub fn dedent_before_closing_brace(line_before_cursor: &str, tab_width: usize) -> usize {
    if !line_before_cursor.trim().is_empty() {
        return 0;
    }
    if line_before_cursor.ends_with('\t') {
        1
    } else {
        line_before_cursor
            .chars()
            .rev()
            .take(tab_width)
            .take_while(|character| *character == ' ')
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_and_skips_delimiters() {
        assert_eq!(
            type_character("call", "", '('),
            Edit {
                text: "call()".into(),
                cursor: 5
            }
        );
        assert_eq!(
            type_character("call(", ")", ')'),
            Edit {
                text: "call()".into(),
                cursor: 6
            }
        );
    }

    #[test]
    fn indents_after_opening_brace() {
        assert_eq!(newline_indent("    if (ok) {", 4, true), "        ");
        assert_eq!(dedent_before_closing_brace("        ", 4), 4);
    }
}
