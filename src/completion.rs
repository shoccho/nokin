use std::collections::BTreeSet;

pub const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while",
];

pub fn prefix_at(text: &str, cursor: usize) -> &str {
    let before = &text[..cursor];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, character)| !is_identifier_continue(*character))
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    &before[start..]
}

pub fn matches(prefix: &str, buffers: &[&str]) -> Vec<String> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut matches = BTreeSet::new();
    for keyword in C_KEYWORDS {
        if keyword.starts_with(prefix) && *keyword != prefix {
            matches.insert((*keyword).to_string());
        }
    }
    for buffer in buffers {
        for word in buffer.split(|character: char| !is_identifier_continue(character)) {
            if word.starts_with(prefix) && word != prefix && is_identifier(word) {
                matches.insert(word.into());
            }
        }
    }
    matches.into_iter().collect()
}

fn is_identifier(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_prefix_and_filters_sorted_unique_matches() {
        assert_eq!(prefix_at("return str", 10), "str");
        assert_eq!(
            matches("str", &["struct stripe; struct stripe;", "int strlen;"]),
            vec!["stripe", "strlen", "struct"]
        );
    }
}
