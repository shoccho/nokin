use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub file: PathBuf,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticToken {
    pub line: usize,
    pub column: usize,
    pub length: usize,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeAction {
    pub title: String,
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone)]
pub struct ServerCommands {
    pub clangd: String,
    pub rust_analyzer: String,
}

pub struct Manager {
    root: PathBuf,
    commands: ServerCommands,
    clients: HashMap<Server, Client>,
    failed: HashSet<Server>,
    diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<Diagnostic>>>>,
}

impl Manager {
    pub fn new(root: &Path, commands: ServerCommands) -> Self {
        Self {
            root: root.to_path_buf(),
            commands,
            clients: HashMap::new(),
            failed: HashSet::new(),
            diagnostics: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn definition(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Option<Location>> {
        let Some(server) = Server::for_path(file) else {
            return Ok(None);
        };
        if self.failed.contains(&server) {
            return Ok(None);
        }
        let client = self.client(file)?;
        client.definition(file, source, line, column)
    }

    pub fn completion(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Vec<String>> {
        self.client(file)?.completion(file, source, line, column)
    }

    pub fn hover(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Option<String>> {
        self.client(file)?.hover(file, source, line, column)
    }

    pub fn references(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Vec<Location>> {
        self.client(file)?.references(file, source, line, column)
    }

    pub fn rename(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
        new_name: &str,
    ) -> io::Result<Vec<TextEdit>> {
        self.client(file)?
            .rename(file, source, line, column, new_name)
    }

    pub fn formatting(&mut self, file: &Path, source: &str) -> io::Result<Vec<TextEdit>> {
        self.client(file)?.formatting(file, source)
    }

    pub fn signature_help(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Option<String>> {
        self.client(file)?
            .signature_help(file, source, line, column)
    }

    pub fn code_actions(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Vec<CodeAction>> {
        self.client(file)?.code_actions(file, source, line, column)
    }

    pub fn semantic_tokens(&mut self, file: &Path, source: &str) -> io::Result<Vec<SemanticToken>> {
        self.client(file)?.semantic_tokens(file, source)
    }

    pub fn sync(&mut self, file: &Path, source: &str) -> io::Result<()> {
        self.client(file)?.sync_document(file, source).map(drop)
    }

    pub fn diagnostics(&self, file: &Path) -> Vec<Diagnostic> {
        self.diagnostics
            .lock()
            .unwrap()
            .get(file)
            .cloned()
            .unwrap_or_default()
    }

    fn client(&mut self, file: &Path) -> io::Result<&mut Client> {
        let Some(server) = Server::for_path(file) else {
            return Err(io::Error::other(
                "no language server configured for this file type",
            ));
        };
        if self.failed.contains(&server) {
            return Err(io::Error::other(format!(
                "{} is unavailable",
                self.commands.command(server)
            )));
        }
        if !self.clients.contains_key(&server) {
            match Client::spawn(
                server,
                self.commands.command(server),
                &self.root,
                Arc::clone(&self.diagnostics),
            ) {
                Ok(client) => {
                    self.clients.insert(server, client);
                }
                Err(error) => {
                    self.failed.insert(server);
                    return Err(error);
                }
            }
        }
        Ok(self
            .clients
            .get_mut(&server)
            .expect("client inserted above"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Server {
    Clangd,
    RustAnalyzer,
}

impl Server {
    fn for_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Some(Self::Clangd),
            "rs" => Some(Self::RustAnalyzer),
            _ => None,
        }
    }

    fn language_id(self) -> &'static str {
        match self {
            Self::Clangd => "c",
            Self::RustAnalyzer => "rust",
        }
    }

    fn project_root(self, workspace: &Path) -> PathBuf {
        workspace
            .ancestors()
            .find(|candidate| match self {
                Self::RustAnalyzer => candidate.join("Cargo.toml").is_file(),
                Self::Clangd => {
                    candidate.join("compile_commands.json").is_file()
                        || candidate.join(".git").exists()
                }
            })
            .unwrap_or(workspace)
            .to_path_buf()
    }
}

impl ServerCommands {
    fn command(&self, server: Server) -> &str {
        match server {
            Server::Clangd => &self.clangd,
            Server::RustAnalyzer => &self.rust_analyzer,
        }
    }
}

struct Client {
    _child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    responses: Arc<(Mutex<HashMap<i64, Value>>, Condvar)>,
    next_id: AtomicI64,
    versions: HashMap<PathBuf, i64>,
    language_id: &'static str,
    semantic_token_types: Vec<String>,
}

impl Client {
    fn spawn(
        server: Server,
        command: &str,
        root: &Path,
        diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<Diagnostic>>>>,
    ) -> io::Result<Self> {
        let mut child = Command::new(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = Arc::new(Mutex::new(
            child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("LSP stdin unavailable"))?,
        ));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("LSP stdout unavailable"))?;
        let responses = Arc::new((Mutex::new(HashMap::new()), Condvar::new()));
        spawn_reader(stdout, Arc::clone(&responses), diagnostics);
        let mut client = Self {
            _child: child,
            stdin,
            responses,
            next_id: AtomicI64::new(1),
            versions: HashMap::new(),
            language_id: server.language_id(),
            semantic_token_types: Vec::new(),
        };
        let root_uri = file_uri(&server.project_root(root))?;
        let initialize = client.request_with_timeout(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {
                    "textDocument": {
                        "semanticTokens": {
                            "requests": {"full": true},
                            "tokenTypes": [],
                            "tokenModifiers": [],
                            "formats": ["relative"]
                        },
                        "signatureHelp": {},
                        "codeAction": {}
                    }
                }
            }),
            Duration::from_secs(15),
        );
        let initialize = match initialize {
            Ok(initialize) => initialize,
            Err(error) => {
                if let Some(status) = client._child.try_wait()? {
                    return Err(io::Error::other(format!(
                        "{} exited during initialization with {status}",
                        command
                    )));
                }
                return Err(error);
            }
        };
        client.semantic_token_types = parse_semantic_legend(&initialize);
        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    fn definition(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Option<Location>> {
        let uri = self.sync_document(file, source)?;
        self.position_locations("textDocument/definition", &uri, line, column, false)
            .map(|locations| locations.into_iter().next())
    }

    fn sync_document(&mut self, file: &Path, source: &str) -> io::Result<String> {
        let uri = file_uri(file)?;
        match self.versions.get(file).copied() {
            Some(previous_version) => {
                let version = previous_version + 1;
                self.versions.insert(file.to_path_buf(), version);
                self.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": {"uri": uri, "version": version},
                        "contentChanges": [{"text": source}]
                    }),
                )?;
            }
            None => {
                self.versions.insert(file.to_path_buf(), 1);
                self.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": self.language_id,
                            "version": 1,
                            "text": source
                        }
                    }),
                )?;
            }
        }
        Ok(uri)
    }

    fn completion(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Vec<String>> {
        let uri = self.sync_document(file, source)?;
        let value =
            self.position_request("textDocument/completion", &uri, line, column, json!({}))?;
        Ok(parse_completions(&value))
    }

    fn hover(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Option<String>> {
        let uri = self.sync_document(file, source)?;
        let value = self.position_request("textDocument/hover", &uri, line, column, json!({}))?;
        Ok(parse_hover(&value))
    }

    fn references(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Vec<Location>> {
        let uri = self.sync_document(file, source)?;
        self.position_locations("textDocument/references", &uri, line, column, true)
    }

    fn rename(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
        new_name: &str,
    ) -> io::Result<Vec<TextEdit>> {
        let uri = self.sync_document(file, source)?;
        let value = self.position_request(
            "textDocument/rename",
            &uri,
            line,
            column,
            json!({"newName": new_name}),
        )?;
        Ok(parse_workspace_edits(&value))
    }

    fn formatting(&mut self, file: &Path, source: &str) -> io::Result<Vec<TextEdit>> {
        let uri = self.sync_document(file, source)?;
        let value = self.request(
            "textDocument/formatting",
            json!({
                "textDocument": {"uri": uri},
                "options": {"tabSize": 4, "insertSpaces": true}
            }),
        )?;
        Ok(parse_text_edits(file, &value))
    }

    fn signature_help(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Option<String>> {
        let uri = self.sync_document(file, source)?;
        let value =
            self.position_request("textDocument/signatureHelp", &uri, line, column, json!({}))?;
        Ok(parse_signature_help(&value))
    }

    fn code_actions(
        &mut self,
        file: &Path,
        source: &str,
        line: usize,
        column: usize,
    ) -> io::Result<Vec<CodeAction>> {
        let uri = self.sync_document(file, source)?;
        let value = self.request(
            "textDocument/codeAction",
            json!({
                "textDocument": {"uri": uri},
                "range": {
                    "start": {"line": line, "character": column},
                    "end": {"line": line, "character": column}
                },
                "context": {"diagnostics": []}
            }),
        )?;
        Ok(parse_code_actions(&value))
    }

    fn semantic_tokens(&mut self, file: &Path, source: &str) -> io::Result<Vec<SemanticToken>> {
        let uri = self.sync_document(file, source)?;
        let value = self.request(
            "textDocument/semanticTokens/full",
            json!({"textDocument": {"uri": uri}}),
        )?;
        Ok(parse_semantic_tokens(&value, &self.semantic_token_types))
    }

    fn position_locations(
        &self,
        method: &str,
        uri: &str,
        line: usize,
        column: usize,
        include_declaration: bool,
    ) -> io::Result<Vec<Location>> {
        let value = self.position_request(
            method,
            uri,
            line,
            column,
            json!({"context": {"includeDeclaration": include_declaration}}),
        )?;
        Ok(parse_locations(&value))
    }

    fn position_request(
        &self,
        method: &str,
        uri: &str,
        line: usize,
        column: usize,
        extra: Value,
    ) -> io::Result<Value> {
        for attempt in 0..6 {
            let mut params = json!({
                "textDocument": {"uri": uri},
                "position": {"line": line, "character": column}
            });
            if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
                params.extend(extra.clone());
            }
            match self.request(method, params) {
                Ok(response) if !response.is_null() => return Ok(response),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
            if attempt < 5 {
                thread::sleep(Duration::from_millis(300));
            }
        }
        Ok(Value::Null)
    }

    fn notify(&self, method: &str, params: Value) -> io::Result<()> {
        self.write_message(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(&self, method: &str, params: Value) -> io::Result<Value> {
        self.request_with_timeout(method, params, Duration::from_secs(4))
    }

    fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        duration: Duration,
    ) -> io::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.write_message(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )?;
        let (responses, available) = &*self.responses;
        let responses = responses.lock().unwrap();
        let (mut responses, timeout) = available
            .wait_timeout_while(responses, duration, |responses| {
                !responses.contains_key(&id)
            })
            .unwrap();
        if timeout.timed_out() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{method} timed out"),
            ));
        }
        let response = responses.remove(&id).unwrap_or(Value::Null);
        if let Some(error) = response.get("error") {
            let code = error.get("code").and_then(Value::as_i64);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("LSP request failed");
            let kind = if code == Some(-32801) {
                io::ErrorKind::WouldBlock
            } else {
                io::ErrorKind::Other
            };
            return Err(io::Error::new(kind, message));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    fn write_message(&self, message: &Value) -> io::Result<()> {
        let body = serde_json::to_vec(message).map_err(io::Error::other)?;
        let mut stdin = self.stdin.lock().unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
        stdin.write_all(&body)?;
        stdin.flush()
    }
}

fn spawn_reader(
    stdout: impl Read + Send + 'static,
    responses: Arc<(Mutex<HashMap<i64, Value>>, Condvar)>,
    diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<Diagnostic>>>>,
) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(message)) = read_message(&mut reader) {
            if message.get("method").and_then(Value::as_str)
                == Some("textDocument/publishDiagnostics")
            {
                store_diagnostics(&diagnostics, &message);
                continue;
            }
            let Some(id) = message.get("id").and_then(Value::as_i64) else {
                continue;
            };
            if message.get("method").is_some() {
                continue;
            }
            let (responses, available) = &*responses;
            responses.lock().unwrap().insert(id, message);
            available.notify_all();
        }
    });
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Ok(None);
        }
        if header == "\r\n" {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(content_length) = content_length else {
        return Err(io::Error::other("LSP response omitted Content-Length"));
    };
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(io::Error::other)
}

fn parse_location(value: &Value) -> Option<Location> {
    let value = value
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(value);
    let uri = value
        .get("uri")
        .or_else(|| value.get("targetUri"))?
        .as_str()?;
    let range = value
        .get("range")
        .or_else(|| value.get("targetSelectionRange"))?;
    let start = range.get("start")?;
    Some(Location {
        file: Url::parse(uri).ok()?.to_file_path().ok()?,
        line: start.get("line")?.as_u64()? as usize + 1,
        column: start.get("character")?.as_u64()? as usize,
    })
}

fn parse_locations(value: &Value) -> Vec<Location> {
    match value {
        Value::Array(items) => items.iter().filter_map(parse_location).collect(),
        Value::Null => Vec::new(),
        value => parse_location(value).into_iter().collect(),
    }
}

fn parse_completions(value: &Value) -> Vec<String> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| value.as_array());
    let mut labels = items
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("label").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    labels
}

fn parse_hover(value: &Value) -> Option<String> {
    let contents = value.get("contents")?;
    match contents {
        Value::String(text) => Some(text.clone()),
        Value::Object(markup) => markup.get("value")?.as_str().map(str::to_owned),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| match part {
                    Value::String(text) => Some(text.clone()),
                    Value::Object(markup) => markup.get("value")?.as_str().map(str::to_owned),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

fn parse_signature_help(value: &Value) -> Option<String> {
    value
        .get("signatures")?
        .as_array()?
        .first()?
        .get("label")?
        .as_str()
        .map(str::to_owned)
}

fn parse_semantic_legend(value: &Value) -> Vec<String> {
    value
        .pointer("/capabilities/semanticTokensProvider/legend/tokenTypes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn parse_semantic_tokens(value: &Value, legend: &[String]) -> Vec<SemanticToken> {
    let mut line = 0;
    let mut column = 0;
    value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect::<Vec<_>>()
        .chunks_exact(5)
        .filter_map(|token| {
            line += token[0] as usize;
            column = if token[0] == 0 {
                column + token[1] as usize
            } else {
                token[1] as usize
            };
            Some(SemanticToken {
                line,
                column,
                length: token[2] as usize,
                kind: legend.get(token[3] as usize)?.clone(),
            })
        })
        .collect()
}

fn parse_code_actions(value: &Value) -> Vec<CodeAction> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|action| {
            Some(CodeAction {
                title: action.get("title")?.as_str()?.to_owned(),
                edits: action
                    .get("edit")
                    .map(parse_workspace_edits)
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn parse_workspace_edits(value: &Value) -> Vec<TextEdit> {
    let Some(changes) = value.get("changes").and_then(Value::as_object) else {
        return Vec::new();
    };
    changes
        .iter()
        .flat_map(|(uri, edits)| {
            Url::parse(uri)
                .ok()
                .and_then(|uri| uri.to_file_path().ok())
                .map(|file| parse_text_edits(&file, edits))
                .unwrap_or_default()
        })
        .collect()
}

fn parse_text_edits(file: &Path, value: &Value) -> Vec<TextEdit> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|edit| {
            let range = edit.get("range")?;
            let start = range.get("start")?;
            let end = range.get("end")?;
            Some(TextEdit {
                file: file.to_path_buf(),
                start_line: start.get("line")?.as_u64()? as usize,
                start_column: start.get("character")?.as_u64()? as usize,
                end_line: end.get("line")?.as_u64()? as usize,
                end_column: end.get("character")?.as_u64()? as usize,
                text: edit.get("newText")?.as_str()?.to_owned(),
            })
        })
        .collect()
}

fn store_diagnostics(diagnostics: &Mutex<HashMap<PathBuf, Vec<Diagnostic>>>, message: &Value) {
    let Some(params) = message.get("params") else {
        return;
    };
    let Some(file) = params
        .get("uri")
        .and_then(Value::as_str)
        .and_then(|uri| Url::parse(uri).ok())
        .and_then(|uri| uri.to_file_path().ok())
    else {
        return;
    };
    let entries = params
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let start = entry.get("range")?.get("start")?;
            Some(Diagnostic {
                line: start.get("line")?.as_u64()? as usize,
                column: start.get("character")?.as_u64()? as usize,
                message: entry.get("message")?.as_str()?.to_owned(),
            })
        })
        .collect();
    diagnostics.lock().unwrap().insert(file, entries);
}

fn file_uri(path: &Path) -> io::Result<String> {
    Url::from_file_path(path)
        .map(String::from)
        .map_err(|_| io::Error::other(format!("cannot convert {} to file URI", path.display())))
}

pub fn apply_text_edits(source: &str, edits: &[TextEdit]) -> String {
    let mut output = source.to_owned();
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| {
        std::cmp::Reverse((
            edit.start_line,
            edit.start_column,
            edit.end_line,
            edit.end_column,
        ))
    });
    for edit in edits {
        let start = line_column_offset(&output, edit.start_line, edit.start_column);
        let end = line_column_offset(&output, edit.end_line, edit.end_column);
        if let (Some(start), Some(end)) = (start, end)
            && start <= end
        {
            output.replace_range(start..end, &edit.text);
        }
    }
    output
}

fn line_column_offset(source: &str, line: usize, column: usize) -> Option<usize> {
    let start = source
        .split_inclusive('\n')
        .take(line)
        .map(str::len)
        .sum::<usize>();
    (start + column <= source.len()).then_some(start + column)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_commands() -> ServerCommands {
        ServerCommands {
            clangd: "clangd".into(),
            rust_analyzer: "rust-analyzer".into(),
        }
    }

    #[test]
    fn recognizes_supported_language_servers() {
        assert_eq!(
            Server::for_path(Path::new("main.rs")),
            Some(Server::RustAnalyzer)
        );
        assert_eq!(Server::for_path(Path::new("main.c")), Some(Server::Clangd));
        assert_eq!(Server::for_path(Path::new("notes.txt")), None);
    }

    #[test]
    fn parses_location_and_location_link_results() {
        let location = parse_location(&json!({
            "uri": "file:///tmp/main.rs",
            "range": {"start": {"line": 4, "character": 2}}
        }))
        .unwrap();
        assert_eq!(location.file, PathBuf::from("/tmp/main.rs"));
        assert_eq!(location.line, 5);
        assert_eq!(location.column, 2);
        assert!(parse_location(&Value::Null).is_none());
    }

    #[test]
    fn finds_rust_project_root_from_src_directory() {
        let root = std::env::temp_dir().join(format!("nokin-lsp-root-{}", std::process::id()));
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(Server::RustAnalyzer.project_root(&src), root);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn applies_text_edits_from_bottom_to_top() {
        let file = PathBuf::from("demo.rs");
        let edits = vec![
            TextEdit {
                file: file.clone(),
                start_line: 0,
                start_column: 4,
                end_line: 0,
                end_column: 7,
                text: "value".into(),
            },
            TextEdit {
                file,
                start_line: 1,
                start_column: 0,
                end_line: 1,
                end_column: 3,
                text: "value".into(),
            },
        ];
        assert_eq!(
            apply_text_edits("let old = 1;\nold\n", &edits),
            "let value = 1;\nvalue\n"
        );
    }

    #[test]
    fn parses_completion_hover_and_workspace_edit_shapes() {
        assert_eq!(
            parse_completions(&json!({"items": [{"label": "beta"}, {"label": "alpha"}]})),
            vec!["alpha", "beta"]
        );
        assert_eq!(
            parse_hover(&json!({"contents": {"kind": "markdown", "value": "**value**"}})),
            Some("**value**".into())
        );
        let edits = parse_workspace_edits(&json!({
            "changes": {
                "file:///tmp/main.rs": [{
                    "range": {
                        "start": {"line": 1, "character": 2},
                        "end": {"line": 1, "character": 5}
                    },
                    "newText": "next"
                }]
            }
        }));
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].file, PathBuf::from("/tmp/main.rs"));
        assert_eq!(edits[0].text, "next");
    }

    #[test]
    fn parses_signature_semantic_tokens_and_code_actions() {
        assert_eq!(
            parse_signature_help(&json!({"signatures": [{"label": "helper(int value)"}]})),
            Some("helper(int value)".into())
        );
        let legend = parse_semantic_legend(&json!({
            "capabilities": {
                "semanticTokensProvider": {
                    "legend": {"tokenTypes": ["variable", "function"]}
                }
            }
        }));
        assert_eq!(legend, vec!["variable", "function"]);
        assert_eq!(
            parse_semantic_tokens(&json!({"data": [2, 4, 6, 1, 0, 0, 8, 3, 0, 0]}), &legend),
            vec![
                SemanticToken {
                    line: 2,
                    column: 4,
                    length: 6,
                    kind: "function".into(),
                },
                SemanticToken {
                    line: 2,
                    column: 12,
                    length: 3,
                    kind: "variable".into(),
                },
            ]
        );
        let actions = parse_code_actions(&json!([{
            "title": "Replace value",
            "edit": {"changes": {"file:///tmp/main.c": [{
                "range": {
                    "start": {"line": 1, "character": 2},
                    "end": {"line": 1, "character": 7}
                },
                "newText": "next"
            }]}}
        }]));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Replace value");
        assert_eq!(actions[0].edits[0].text, "next");
    }

    #[test]
    fn clangd_resolves_hover_definition_and_references_when_installed() {
        if Command::new("clangd").arg("--version").output().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "nokin-clangd-smoke-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("helper.h"), "int helper(void);\n").unwrap();
        std::fs::write(
            root.join("helper.c"),
            "#include \"helper.h\"\n\nint helper(void) {\n    return 42;\n}\n",
        )
        .unwrap();
        let main = root.join("main.c");
        let source = "#include \"helper.h\"\n\nint main(void) {\n    return helper();\n}\n";
        std::fs::write(&main, source).unwrap();

        let mut manager = Manager::new(&root, default_commands());
        let hover = manager.hover(&main, source, 3, 12).unwrap().unwrap();
        assert!(hover.contains("helper"), "{hover}");
        let definition = manager.definition(&main, source, 3, 12).unwrap().unwrap();
        assert!(
            matches!(
                definition.file.file_name().and_then(|name| name.to_str()),
                Some("helper.h" | "helper.c")
            ),
            "{}",
            definition.file.display()
        );
        let references = manager.references(&main, source, 3, 12).unwrap();
        assert!(
            references.iter().any(|location| location.file == main),
            "{references:?}"
        );
        let semantic_tokens = manager.semantic_tokens(&main, source).unwrap();
        assert!(
            semantic_tokens.iter().any(|token| token.kind == "function"),
            "{semantic_tokens:?}"
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
