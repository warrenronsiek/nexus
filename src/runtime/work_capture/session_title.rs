// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary host-session-title: read only bounded exact-session title metadata; transcript messages and provider responses are never retained.
use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Deserialize)]
struct TitleSource {
    session_id: String,
    session_title: Option<String>,
    transcript_path: Option<String>,
}
fn title(value: &str) -> Option<String> {
    (!value.trim().is_empty() && value.len() <= 1024).then(|| value.to_owned())
}
/// Optional read-only enrichment. Failure never changes the lifecycle observation.
pub async fn resolve_session_title(host: &str, input: &[u8]) -> Option<String> {
    let source: TitleSource = serde_json::from_slice(input).ok()?;
    if source.session_title.is_some()
        || source.session_id.is_empty()
        || source.session_id.len() > 512
    {
        return None;
    }
    match host {
        "codex" => {
            let mut command = tokio::process::Command::new("codex");
            command.args(["app-server", "--listen", "stdio://"]);
            codex_title(&mut command, &source.session_id).await
        }
        "claude" => {
            let path = source.transcript_path?;
            tokio::task::spawn_blocking(move || claude_title(Path::new(&path), &source.session_id))
                .await
                .ok()
                .flatten()
        }
        _ => None,
    }
}
fn claude_title(path: &Path, session_id: &str) -> Option<String> {
    let windows = transcript_windows(path).ok()?;
    let mut entries = title_entries(&windows.head, 0, session_id);
    entries.extend(title_entries(
        &windows.tail,
        windows.tail_offset,
        session_id,
    ));
    entries
        .iter()
        .rev()
        .filter(|entry| entry.kind == "custom-title")
        .find_map(|entry| entry.custom_title.as_deref().and_then(title))
        .or_else(|| {
            entries
                .iter()
                .rev()
                .filter(|entry| entry.kind == "ai-title")
                .find_map(|entry| entry.ai_title.as_deref().and_then(title))
        })
}
fn title_entries(bytes: &[u8], offset: u64, session_id: &str) -> Vec<TitleEntry> {
    let text = String::from_utf8_lossy(bytes);
    let retained = if offset > 0 {
        text.split_once('\n').map_or("", |(_, tail)| tail)
    } else {
        text.as_ref()
    };
    retained
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|entry: &TitleEntry| entry.session_id == session_id)
        .collect()
}
#[derive(Deserialize)]
struct TitleEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
    #[serde(rename = "aiTitle")]
    ai_title: Option<String>,
}
struct TitleWindows {
    head: Vec<u8>,
    tail: Vec<u8>,
    tail_offset: u64,
}
fn transcript_windows(path: &Path) -> std::io::Result<TitleWindows> {
    let (mut file, length) = open_transcript(path)?;
    let offset = length.saturating_sub(256 * 1024);
    Ok(TitleWindows {
        head: read_window(&mut file, 0, 64 * 1024)?,
        tail: read_window(&mut file, offset, 256 * 1024)?,
        tail_offset: offset,
    })
}
fn open_transcript(path: &Path) -> std::io::Result<(std::fs::File, u64)> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::other("not a regular transcript"));
    }
    Ok((file, metadata.len()))
}
fn read_window(file: &mut std::fs::File, offset: u64, limit: u64) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}
async fn codex_title(command: &mut tokio::process::Command, session_id: &str) -> Option<String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        read_codex(&mut child, session_id),
    )
    .await
    .ok()
    .flatten();
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}
async fn read_codex(child: &mut tokio::process::Child, session_id: &str) -> Option<String> {
    let mut stdin = child.stdin.take()?;
    let mut reader = BufReader::new(child.stdout.take()?.take(64 * 1024));
    initialize_codex(&mut stdin, &mut reader).await?;
    let request = serde_json::json!({"id":2,"method":"thread/read","params":{"threadId":session_id,"includeTurns":false}});
    stdin
        .write_all(format!("{{\"method\":\"initialized\"}}\n{request}\n").as_bytes())
        .await
        .ok()?;
    response(&mut reader, 2)
        .await
        .and_then(|result| codex_name(&result, session_id))
}
async fn initialize_codex<R: tokio::io::AsyncBufRead + Unpin>(
    stdin: &mut tokio::process::ChildStdin,
    reader: &mut R,
) -> Option<()> {
    stdin.write_all(b"{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"nexus-title-reader\",\"version\":\"1\"}}}\n").await.ok()?;
    response(reader, 1).await.map(|_| ())
}
fn codex_name(result: &serde_json::Value, session_id: &str) -> Option<String> {
    let thread = result.get("thread")?;
    if thread.get("id")?.as_str()? != session_id {
        return None;
    }
    thread
        .get("name")
        .and_then(|value| value.as_str())
        .and_then(title)
}
async fn response<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    id: u64,
) -> Option<serde_json::Value> {
    for _ in 0..32 {
        let mut line = String::new();
        if reader.read_line(&mut line).await.ok()? == 0 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(&line).ok()?;
        if value.get("id").and_then(|id| id.as_u64()) == Some(id) {
            return value.get("result").cloned();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_enrichment_respects_the_durable_label_boundary_without_truncating() {
        assert_eq!(title(&"a".repeat(1024)), Some("a".repeat(1024)));
        assert_eq!(title(&"a".repeat(1025)), None);
    }

    #[test]
    fn a_custom_name_in_the_head_remains_the_host_title_after_a_large_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let transcript = temp.path().join("s.jsonl");
        let custom =
            "{\"type\":\"custom-title\",\"sessionId\":\"s\",\"customTitle\":\"Named by human\"}\n";
        let padding =
            serde_json::json!({"type":"assistant","sessionId":"s","message":"x".repeat(300_000)});
        std::fs::write(&transcript, format!("{custom}{padding}\n{{\"type\":\"ai-title\",\"sessionId\":\"s\",\"aiTitle\":\"Generated later\"}}\n")).unwrap();
        assert_eq!(
            claude_title(&transcript, "s").as_deref(),
            Some("Named by human")
        );
    }

    #[test]
    fn claude_titles_follow_exact_session_metadata_and_latest_rename() {
        let temp = tempfile::tempdir().unwrap();
        let transcript = temp.path().join("session.jsonl");
        std::fs::write(&transcript, concat!(
            "{\"type\":\"custom-title\",\"sessionId\":\"s\",\"customTitle\":\"Initial name\"}\n",
            "{\"type\":\"user\",\"sessionId\":\"s\",\"message\":{\"content\":\"private prompt\"}}\n",
            "{\"type\":\"custom-title\",\"sessionId\":\"other\",\"customTitle\":\"Wrong session\"}\n",
            "{\"type\":\"custom-title\",\"sessionId\":\"s\",\"customTitle\":\"Host display name\"}\n"
        )).unwrap();
        assert_eq!(
            claude_title(&transcript, "s").as_deref(),
            Some("Host display name")
        );
        assert_eq!(claude_title(&transcript, "absent"), None);
    }

    #[test]
    fn claude_generated_display_title_is_used_until_a_custom_name_exists() {
        let temp = tempfile::tempdir().unwrap();
        let transcript = temp.path().join("s.jsonl");
        std::fs::write(
            &transcript,
            "{\"type\":\"ai-title\",\"sessionId\":\"s\",\"aiTitle\":\"AI display name\"}\n",
        )
        .unwrap();
        assert_eq!(
            claude_title(&transcript, "s").as_deref(),
            Some("AI display name")
        );
    }

    #[tokio::test]
    async fn codex_title_uses_metadata_read_without_resuming_or_loading_turns() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("codex");
        std::fs::write(&executable, "#!/usr/bin/env python3\nimport sys,json\nfor line in sys.stdin:\n r=json.loads(line)\n if r.get('method')=='initialize': print(json.dumps({'id':1,'result':{}}),flush=True)\n if r.get('method')=='thread/read':\n  assert r['params']=={'threadId':'session-1','includeTurns':False}\n  print(json.dumps({'id':2,'result':{'thread':{'id':'session-1','name':'Exact Codex name'}}}),flush=True)\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = tokio::process::Command::new(&executable);
        assert_eq!(
            codex_title(&mut command, "session-1").await.as_deref(),
            Some("Exact Codex name")
        );
    }
}
