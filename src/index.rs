use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SymbolKind {
    Function,
    Struct,
    Union,
    Enum,
    Trait,
    Typedef,
    Global,
    Macro,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub line: usize,
}

#[derive(Debug, Default)]
pub struct Index {
    symbols: BTreeMap<String, Vec<Symbol>>,
}

impl Index {
    pub fn update(&mut self, file: &Path, source: &str) {
        self.remove(file);
        for symbol in extract_symbols(file, source) {
            self.symbols
                .entry(symbol.name.clone())
                .or_default()
                .push(symbol);
        }
    }

    pub fn remove(&mut self, file: &Path) {
        self.symbols.retain(|_, matches| {
            matches.retain(|symbol| symbol.file != file);
            !matches.is_empty()
        });
    }

    pub fn find(&self, name: &str) -> &[Symbol] {
        self.symbols
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

pub fn extract_symbols(file: &Path, source: &str) -> Vec<Symbol> {
    match file.extension().and_then(|e| e.to_str()) {
        Some("rs") => extract_rust_symbols(file, source),
        _ => extract_c_symbols(file, source),
    }
}

fn extract_c_symbols(file: &Path, source: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .is_err()
    {
        return symbols;
    }
    let Some(tree) = parser.parse(source, None) else {
        return symbols;
    };
    let root = tree.root_node();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        extract_top_level_node(&mut symbols, file, source, node);
    }
    symbols
}

fn extract_rust_symbols(file: &Path, source: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return symbols;
    }
    let Some(tree) = parser.parse(source, None) else {
        return symbols;
    };
    let root = tree.root_node();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        extract_rust_node(&mut symbols, file, source, node);
    }
    symbols
}

fn extract_rust_node(symbols: &mut Vec<Symbol>, file: &Path, source: &str, node: Node<'_>) {
    let kind = match node.kind() {
        "function_item" => SymbolKind::Function,
        "struct_item" => SymbolKind::Struct,
        "enum_item" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "type_alias" => SymbolKind::Typedef,
        "const_item" | "static_item" => SymbolKind::Global,
        "macro_definition" => SymbolKind::Macro,
        _ => return,
    };
    if let Some(name) = node.child_by_field_name("name") {
        push_node(symbols, file, source, name, kind);
    }
}

fn extract_top_level_node(symbols: &mut Vec<Symbol>, file: &Path, source: &str, node: Node<'_>) {
    match node.kind() {
        "function_definition" => {
            push_declarator(
                symbols,
                file,
                source,
                node,
                "declarator",
                SymbolKind::Function,
            );
        }
        "type_definition" => {
            extract_record_children(symbols, file, source, node);
            push_declarator(
                symbols,
                file,
                source,
                node,
                "declarator",
                SymbolKind::Typedef,
            );
        }
        "declaration" => {
            extract_record_children(symbols, file, source, node);
            let mut cursor = node.walk();
            for declarator in node.children_by_field_name("declarator", &mut cursor) {
                if declarator.kind() != "function_declarator"
                    && let Some(identifier) = declarator_identifier(declarator)
                {
                    push_node(symbols, file, source, identifier, SymbolKind::Global);
                }
            }
        }
        "struct_specifier" | "union_specifier" | "enum_specifier" => {
            push_record(symbols, file, source, node);
        }
        "preproc_def" | "preproc_function_def" => {
            if let Some(name) = node.child_by_field_name("name") {
                push_node(symbols, file, source, name, SymbolKind::Macro);
            }
        }
        _ => {}
    }
}

fn extract_record_children(symbols: &mut Vec<Symbol>, file: &Path, source: &str, node: Node<'_>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "struct_specifier" | "union_specifier" | "enum_specifier"
        ) {
            push_record(symbols, file, source, child);
        }
    }
}

fn push_record(symbols: &mut Vec<Symbol>, file: &Path, source: &str, node: Node<'_>) {
    let kind = match node.kind() {
        "struct_specifier" => SymbolKind::Struct,
        "union_specifier" => SymbolKind::Union,
        "enum_specifier" => SymbolKind::Enum,
        _ => return,
    };
    if let Some(name) = node.child_by_field_name("name") {
        push_node(symbols, file, source, name, kind);
    }
}

fn push_declarator(
    symbols: &mut Vec<Symbol>,
    file: &Path,
    source: &str,
    node: Node<'_>,
    field: &str,
    kind: SymbolKind,
) {
    if let Some(identifier) = node
        .child_by_field_name(field)
        .and_then(declarator_identifier)
    {
        push_node(symbols, file, source, identifier, kind);
    }
}

fn declarator_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(declarator_identifier)
}

fn push_node(
    symbols: &mut Vec<Symbol>,
    file: &Path,
    source: &str,
    node: Node<'_>,
    kind: SymbolKind,
) {
    let Ok(name) = node.utf8_text(source.as_bytes()) else {
        return;
    };
    symbols.push(Symbol {
        name: name.into(),
        kind,
        file: file.into(),
        line: node.start_position().row + 1,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_top_level_c_symbols() {
        let symbols = extract_symbols(
            Path::new("demo.c"),
            "#define LIMIT 4\nstruct User { int id; };\ntypedef int Count;\nint total;\nint add(int a, int b) {\n return a + b;\n}\n",
        );
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "LIMIT" && symbol.kind == SymbolKind::Macro)
        );
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "User" && symbol.kind == SymbolKind::Struct)
        );
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "Count" && symbol.kind == SymbolKind::Typedef)
        );
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "total" && symbol.kind == SymbolKind::Global)
        );
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "add" && symbol.kind == SymbolKind::Function)
        );
        assert!(!symbols.iter().any(|symbol| symbol.name == "id"));
    }

    #[test]
    fn refresh_removes_stale_symbols() {
        let mut index = Index::default();
        index.update(Path::new("a.c"), "int before;\n");
        index.update(Path::new("a.c"), "int after;\n");
        assert!(index.find("before").is_empty());
        assert_eq!(index.find("after").len(), 1);
    }

    #[test]
    fn extracts_multiline_function_without_local_variables() {
        let symbols = extract_symbols(
            Path::new("demo.c"),
            "int\nadd(\n int left,\n int right\n) {\n int local = left + right;\n return local;\n}\n",
        );
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "add" && symbol.kind == SymbolKind::Function)
        );
        assert!(!symbols.iter().any(|symbol| symbol.name == "local"));
    }
}
