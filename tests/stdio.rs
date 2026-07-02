//! Smoke test for the stdio transport: spawns the released binary and
//! speaks framed JSON-RPC over its pipes. This is the only test of
//! `main.rs`'s glue — it exists to catch stdout pollution and framing
//! regressions forever.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn frame(payload: &serde_json::Value) -> Vec<u8> {
    let body = payload.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn read_frame(reader: &mut impl BufRead) -> serde_json::Value {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0, "unexpected EOF");
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            content_length = Some(value.parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0u8; content_length.expect("Content-Length header")];
    reader.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).expect("stdout carried a non-protocol byte")
}

fn wait_for_exit(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "server exited with {status}");
            return;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("server did not exit after shutdown/exit");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_binary_speaks_lsp_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_csv-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn csv-lsp");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": { "general": { "positionEncodings": ["utf-8"] } } },
    });
    stdin.write_all(&frame(&initialize)).unwrap();
    stdin.flush().unwrap();

    let response = read_frame(&mut stdout);
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["serverInfo"]["name"], "csv-lsp");
    assert_eq!(
        response["result"]["capabilities"]["positionEncoding"],
        "utf-8"
    );

    let initialized =
        serde_json::json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    stdin.write_all(&frame(&initialized)).unwrap();

    let shutdown = serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    stdin.write_all(&frame(&shutdown)).unwrap();
    stdin.flush().unwrap();
    let response = read_frame(&mut stdout);
    assert_eq!(response["id"], 2);

    let exit = serde_json::json!({ "jsonrpc": "2.0", "method": "exit" });
    stdin.write_all(&frame(&exit)).unwrap();
    stdin.flush().unwrap();

    wait_for_exit(&mut child);
}
