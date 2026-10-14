// @feature runtime
// @spec docs/features/runtime.md
// @boundary dynamic-json

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, ChildStdout};

pub struct StdioMcpConnection {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioMcpConnection {
    pub fn new(input: ChildStdin, output: ChildStdout) -> Self {
        Self {
            input,
            output: BufReader::new(output),
            next_id: 1,
        }
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        let response = self.read_response();
        assert_eq!(response["id"], id);
        response
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":arguments}))["result"]
            ["structuredContent"]
            .clone()
    }

    pub fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        id
    }

    pub fn read_response(&mut self) -> Value {
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
}
