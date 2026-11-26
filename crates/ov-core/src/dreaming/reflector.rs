use crate::platform::PlatformPaths;
use crate::policy::model::{DreamingPolicy, ReflectorMode};
use reqwest::Client;
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Debug, Clone)]
pub struct ReflectorResult {
    pub content: String,
    pub engine_label: String,
}

#[derive(Debug, Clone)]
pub struct InboxRouteResult {
    pub destination_uri: String,
    pub rationale: String,
    pub engine_label: String,
}

/// Use an optional local CLI agent (`claude -p` / `codex exec`) or an OpenAI-compatible
/// LLM endpoint (e.g. Ollama / vLLM / OpenAI) to semantically merge two overlapping memories
/// and resolve contradictions. Returns `None` if deterministic mode is selected or on fallback.
pub async fn reflect_merge(
    policy: &DreamingPolicy,
    primary_uri: &str,
    primary_raw: &str,
    peer_uri: &str,
    peer_raw: &str,
) -> Option<ReflectorResult> {
    if policy.reflector_mode == ReflectorMode::Deterministic {
        return None;
    }

    let prompt = format!(
        "You are a memory consolidation engine. Merge the following two overlapping Markdown memories into ONE clean, concise, contradiction-free Markdown note.\n\
         Rules:\n\
         - Keep the same language as the inputs.\n\
         - Remove duplicate points; if newer or more specific rules conflict with older vague ones, keep the clearer rule.\n\
         - Output ONLY the merged Markdown body (no YAML frontmatter, no outer ```markdown fences).\n\n\
         [Primary URI: {}]\n{}\n\n\
         [Peer URI: {}]\n{}\n",
        primary_uri,
        truncate_chars(primary_raw, 3000),
        peer_uri,
        truncate_chars(peer_raw, 3000)
    );

    let (raw_out, label) = invoke_reflector(policy, &prompt).await?;
    let cleaned = strip_markdown_fence(&raw_out).trim().to_string();
    if cleaned.len() < 10 {
        return None;
    }
    Some(ReflectorResult {
        content: cleaned,
        engine_label: label,
    })
}

/// Ask the optional reflector to classify an Inbox note into the best matching project URI.
pub async fn reflect_inbox_route(
    policy: &DreamingPolicy,
    note_uri: &str,
    note_raw: &str,
    available_projects: &[String],
    fallback_dest: &str,
) -> Option<InboxRouteResult> {
    if policy.reflector_mode == ReflectorMode::Deterministic || available_projects.is_empty() {
        return None;
    }

    let file_name = note_uri.rsplit('/').next().unwrap_or("note.md");
    let projects: Vec<_> = available_projects
        .iter()
        .map(|uri| directory_uri(uri))
        .collect();
    let project_list = projects.join("\n- ");
    let fallback_dir = directory_uri(fallback_dest.rsplit_once('/')?.0);
    let prompt = format!(
        "You are a memory router. Given an Inbox note and the list of existing project directories, choose the single best project directory URI.\n\
         Respond with ONLY a valid JSON object: {{\"destination_dir\": \"viking://...\", \"rationale\": \"short reason\"}}\n\n\
         [Note URI: {}]\n{}\n\n\
         [Existing Project Directories]\n- {}\n- (fallback: {})\n",
        note_uri,
        truncate_chars(note_raw, 2000),
        project_list,
        fallback_dir
    );

    let (raw_out, label) = invoke_reflector(policy, &prompt).await?;
    let parsed = extract_json_object(&raw_out)?;
    let dir = parsed.get("destination_dir")?.as_str()?;
    let normalized_dir = directory_uri(dir);
    if !dir.trim().starts_with("viking://") || !projects.contains(&normalized_dir) {
        return None;
    }
    let rationale = parsed
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("由 Dreaming 语义反思器自动匹配项目目录")
        .to_string();
    Some(InboxRouteResult {
        destination_uri: format!("{}{}", normalized_dir, file_name),
        rationale,
        engine_label: label,
    })
}

fn directory_uri(uri: &str) -> String {
    format!("{}/", uri.trim().trim_end_matches('/'))
}

async fn invoke_reflector(policy: &DreamingPolicy, prompt: &str) -> Option<(String, String)> {
    match policy.reflector_mode {
        ReflectorMode::Deterministic => None,
        ReflectorMode::LocalCli => invoke_local_cli(&policy.reflector_cli_agent, prompt).await,
        ReflectorMode::OpenaiCompatible => invoke_openai_compatible(policy, prompt).await,
    }
}

async fn invoke_local_cli(preferred: &str, prompt: &str) -> Option<(String, String)> {
    let paths = PlatformPaths::detect()?;
    let candidates: Vec<&str> = match preferred {
        "claude" | "claude-code" => vec!["claude"],
        "codex" => vec!["codex"],
        _ => vec!["claude", "codex"],
    };

    for bin_name in candidates {
        let bin_path = paths.resolve_binary(bin_name);
        #[cfg(windows)]
        let bin_path = bin_path.or_else(|| {
            if bin_name != "codex" {
                return None;
            }
            let path = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
                .join("OpenAI/Codex/bin/codex.exe");
            path.is_file().then_some(path)
        });
        let Some(bin_path) = bin_path else {
            continue;
        };
        let mut cmd = Command::new(bin_path);
        cmd.env("OPENVIKING_DISABLE_HOOKS", "1")
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        if bin_name == "claude" {
            cmd.args([
                "-p",
                "--output-format",
                "text",
                "--tools",
                "",
                "--max-turns",
                "1",
            ]);
        } else {
            cmd.args([
                "exec",
                "-s",
                "read-only",
                "--ephemeral",
                "--skip-git-repo-check",
                "-c",
                "features.hooks=false",
                "-",
            ]);
        }
        let Ok(mut child) = cmd.spawn() else {
            continue;
        };
        let Some(mut stdin) = child.stdin.take() else {
            continue;
        };
        let write_prompt = async move {
            let result = stdin.write_all(prompt.as_bytes()).await;
            drop(stdin);
            result
        };
        let output = async { tokio::try_join!(write_prompt, child.wait_with_output()) };
        // The timed future owns the child, so dropping it on timeout kills the process.
        let Ok(Ok(((), out))) = tokio::time::timeout(Duration::from_secs(25), output).await else {
            continue;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        if out.status.success() && !text.trim().is_empty() {
            return Some((text.trim().to_owned(), format!("local_cli:{bin_name}")));
        }
    }
    None
}

async fn invoke_openai_compatible(policy: &DreamingPolicy, prompt: &str) -> Option<(String, String)> {
    let base = policy.reflector_endpoint.trim().trim_end_matches('/');
    if base.is_empty() || policy.reflector_model.trim().is_empty() {
        return None;
    }
    let url = if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{}/chat/completions", base)
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .ok()?;
    let mut req = client.post(&url).json(&json!({
        "model": policy.reflector_model,
        "temperature": 0.2,
        "messages": [
            { "role": "system", "content": "You are a precise memory consolidation assistant." },
            { "role": "user", "content": prompt }
        ]
    }));

    if !policy.reflector_api_key.trim().is_empty() {
        req = req.bearer_auth(policy.reflector_api_key.trim());
    }

    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let val: Value = resp.json().await.ok()?;
    let text = val
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()?
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some((text, format!("llm:{}", policy.reflector_model)))
    }
}

fn strip_markdown_fence(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(rest) = trimmed
        .strip_prefix("```markdown\n")
        .or_else(|| trimmed.strip_prefix("```md\n"))
        .or_else(|| trimmed.strip_prefix("```\n"))
    {
        return rest.strip_suffix("```").unwrap_or(rest).trim();
    }
    trimmed
}

fn extract_json_object(s: &str) -> Option<Value> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&s[start..=end]).ok()
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_fence_and_extract_json() {
        let fenced = "```markdown\n# Title\nBody\n```";
        assert_eq!(strip_markdown_fence(fenced), "# Title\nBody");

        let noisy = "Sure! Here is the JSON:\n{\"destination_dir\":\"viking://resources/projects/demo/\",\"rationale\":\"matches demo\"}\n";
        let v = extract_json_object(noisy).unwrap();
        assert_eq!(
            v.get("destination_dir").and_then(|x| x.as_str()),
            Some("viking://resources/projects/demo/")
        );
    }
}
