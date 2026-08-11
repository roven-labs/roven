//! Minimal MCP stdio client used for trusted, read-only tool discovery.

use std::{
    collections::HashSet,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct McpTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
}

#[derive(Debug)]
pub(crate) struct McpClient {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    tools: Vec<McpTool>,
}

impl McpClient {
    pub(crate) fn connect(workspace: &Path) -> io::Result<Self> {
        let mut child = Command::new(codegraph_program())
            .args(["serve", "--mcp"])
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "CodeGraph MCP stdin unavailable")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "CodeGraph MCP stdout unavailable",
            )
        })?;
        let mut client = Self {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            tools: Vec::new(),
        };

        client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {
                    "name": "roven",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )?;
        client.notify("notifications/initialized", json!({}))?;

        let tools = client.request("tools/list", json!({}))?;
        client.tools = parse_tools(&tools)?;

        Ok(client)
    }

    pub(crate) fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    pub(crate) fn call(&mut self, name: &str, arguments: Value) -> io::Result<Value> {
        self.request(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
        )
    }

    fn notify(&mut self, method: &str, params: Value) -> io::Result<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        write_message(&mut self.stdin, &message)
    }

    fn request(&mut self, method: &str, params: Value) -> io::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        write_message(&mut self.stdin, &message)?;

        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line)?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "CodeGraph MCP closed stdout",
                ));
            }
            let response: Value = serde_json::from_str(line.trim()).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid CodeGraph MCP response: {error}"),
                )
            })?;
            if response.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(io::Error::other(format!("MCP server error: {error}")));
            }
            return response.get("result").cloned().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "CodeGraph MCP response did not contain a result",
                )
            });
        }
    }
}

fn codegraph_program() -> PathBuf {
    let override_path = std::env::var_os("ROVEN_CODEGRAPH_PATH").map(PathBuf::from);
    #[cfg(windows)]
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let local_app_data = None;

    resolve_codegraph_program(local_app_data.as_deref(), override_path.as_deref())
}

fn resolve_codegraph_program(
    local_app_data: Option<&Path>,
    override_path: Option<&Path>,
) -> PathBuf {
    override_path
        .filter(|path| path.is_file())
        .map(Path::to_path_buf)
        .or_else(|| {
            local_app_data
                .map(|path| {
                    path.join("codegraph")
                        .join("current")
                        .join("bin")
                        .join("codegraph.cmd")
                })
                .filter(|path| path.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("codegraph"))
}

impl Drop for McpClient {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &self._child.id().to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(windows))]
        let _ = self._child.kill();
        let _ = self._child.wait();
    }
}

fn write_message(stdin: &mut ChildStdin, message: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *stdin, message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    stdin.write_all(b"\n")?;
    stdin.flush()
}

fn parse_tool(tool: &Value) -> io::Result<McpTool> {
    let name = tool
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MCP tool has no name"))?;
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let input_schema = tool
        .get("inputSchema")
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MCP tool has no inputSchema"))?;
    Ok(McpTool {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema,
    })
}

fn parse_tools(value: &Value) -> io::Result<Vec<McpTool>> {
    let tools: Vec<McpTool> = value
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MCP tools/list had no tools"))?
        .iter()
        .map(parse_tool)
        .collect::<io::Result<_>>()?;
    let mut names = HashSet::new();
    if tools.iter().any(|tool| !names.insert(&tool.name)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "MCP tools/list contained duplicate tool names",
        ));
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{parse_tools, resolve_codegraph_program};
    use serde_json::json;

    #[test]
    fn parse_tools_preserves_every_discovered_tool() {
        let tools = parse_tools(&json!({
            "tools": [
                {
                    "name": "codegraph_explore",
                    "description": "the exact description",
                    "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}
                },
                {
                    "name": "codegraph_node",
                    "description": "another exact description",
                    "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}}
                }
            ]
        }))
        .expect("tool should parse");

        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "codegraph_explore");
        assert_eq!(tools[0].description, "the exact description");
        assert_eq!(tools[1].name, "codegraph_node");
        assert_eq!(
            tools[1].input_schema["properties"]["name"]["type"],
            "string"
        );
    }

    #[test]
    fn installed_codegraph_advertises_and_answers_the_raw_tool() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"));
        if !workspace.join(".codegraph").is_dir()
            || std::process::Command::new("codegraph")
                .arg("--version")
                .status()
                .is_err()
        {
            return;
        }

        let mut client = super::McpClient::connect(workspace).expect("CodeGraph MCP should start");
        assert!(
            client
                .tools()
                .iter()
                .any(|tool| tool.name == "codegraph_explore")
        );
        let result = client
            .call(
                "codegraph_explore",
                json!({"query": "src/tools.rs", "maxFiles": 1}),
            )
            .expect("CodeGraph MCP should answer tools/call");
        assert!(result.get("content").is_some() || result.get("structuredContent").is_some());
    }

    #[test]
    fn resolves_a_valid_override_before_the_appdata_install() {
        let root = test_directory("override");
        let override_program = root.join("custom-codegraph.cmd");
        let appdata_program = root
            .join("appdata")
            .join("codegraph")
            .join("current")
            .join("bin")
            .join("codegraph.cmd");
        write_test_program(&override_program);
        write_test_program(&appdata_program);

        assert_eq!(
            resolve_codegraph_program(Some(&root.join("appdata")), Some(&override_program)),
            override_program
        );

        fs::remove_dir_all(root).expect("test directory should be removable");
    }

    #[test]
    fn resolves_the_standard_appdata_install_before_path_fallback() {
        let root = test_directory("appdata");
        let program = root
            .join("codegraph")
            .join("current")
            .join("bin")
            .join("codegraph.cmd");
        write_test_program(&program);

        assert_eq!(resolve_codegraph_program(Some(&root), None), program);
        assert_eq!(
            resolve_codegraph_program(None, None),
            PathBuf::from("codegraph")
        );

        fs::remove_dir_all(root).expect("test directory should be removable");
    }

    fn test_directory(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("roven-mcp-{name}-{}-{unique}", std::process::id()))
    }

    fn write_test_program(path: &Path) {
        fs::create_dir_all(path.parent().expect("test program should have a parent"))
            .expect("test program directory should be created");
        fs::write(path, "test program").expect("test program should be created");
    }
}
