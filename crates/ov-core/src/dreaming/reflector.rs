use crate::platform::PlatformPaths;
use crate::policy::model::{DreamingPolicy, KnowledgeRelation, ReflectorMode};
use reqwest::Client;
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Debug, Clone)]
pub struct ReflectorResult {
    pub relation: KnowledgeRelation,
    pub classified: bool,
    pub content: String,
    pub rationale: Option<String>,
    pub confidence: Option<f32>,
    pub engine_label: String,
}

#[derive(Debug, Clone)]
pub struct InboxRouteResult {
    pub destination_uri: String,
    pub rationale: String,
    pub engine_label: String,
}

#[derive(Debug, Clone)]
pub struct TranscriptVerifyResult {
    pub corrected_content: String,
    pub rationale: String,
    pub confidence: f32,
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
        "You are a memory evolution engine. Compare the two Markdown memories below and classify their relationship into ONE of 5 types:\n\
         - DUPLICATES: same fact repeated; merge cleanly and remove redundancy.\n\
         - UPDATES: newer/more specific fact supersedes an older one; keep the updated fact and note what was superseded.\n\
         - EXTENDS: complementary facts; combine both without discarding existing valid rules.\n\
         - CONFLICTS: direct contradiction that requires human review; keep BOTH positions clearly labeled under a conflict note.\n\
         - DERIVES: higher-level inferred pattern from both notes.\n\
         Also discard ephemeral one-off states (temporary counters, single-run error line numbers) and distill reusable procedural steps & rules.\n\
         Respond with ONLY a valid JSON object:\n\
         {{\"relation\": \"DUPLICATES|UPDATES|EXTENDS|CONFLICTS|DERIVES\", \"merged_content\": \"Markdown body without YAML frontmatter\", \"rationale\": \"short explanation in input language\", \"confidence\": 0.90}}\n\n\
         [Primary URI: {}]\n{}\n\n\
         [Peer URI: {}]\n{}\n",
        primary_uri,
        truncate_chars(primary_raw, 3000),
        peer_uri,
        truncate_chars(peer_raw, 3000)
    );

    let (raw_out, label) = invoke_reflector(policy, &prompt).await?;
    parse_reflect_merge_output(&raw_out, label)
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

/// Cross-check a recently written/updated memory note against committed local session transcripts.
/// Returns `Some(TranscriptVerifyResult)` only when the reflector detects a factual contradiction.
pub async fn reflect_transcript_verify(
    policy: &DreamingPolicy,
    note_uri: &str,
    note_raw: &str,
    transcript_excerpt: &str,
) -> Option<TranscriptVerifyResult> {
    if policy.reflector_mode == ReflectorMode::Deterministic || transcript_excerpt.trim().is_empty() {
        return None;
    }

    let prompt = format!(
        "You are a memory fact-checker. Compare the memory note below against the ground-truth recent session transcripts.\n\
         Only flag a contradiction if the note states a concrete fact, path, command, or preference that directly conflicts with the transcripts.\n\
         Respond with ONLY a valid JSON object:\n\
         {{\"contradicted\": true|false, \"corrected_content\": \"Corrected Markdown body without YAML frontmatter (empty if not contradicted)\", \"rationale\": \"short explanation citing the transcript\", \"confidence\": 0.85}}\n\n\
         [Memory Note URI: {}]\n{}\n\n\
         [Recent Committed Transcripts (Ground Truth)]\n{}\n",
        note_uri,
        truncate_chars(note_raw, 2500),
        truncate_chars(transcript_excerpt, 3500)
    );

    let (raw_out, label) = invoke_reflector(policy, &prompt).await?;
    let parsed = extract_json_object(&raw_out)?;
    if !parsed.get("contradicted").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let corrected = parsed
        .get("corrected_content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let rationale = parsed
        .get("rationale")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("对照本地原始会话记录发现笔记事实偏差，建议修正。")
        .to_string();
    let confidence = parsed
        .get("confidence")
        .and_then(Value::as_f64)
        .map(|v| (v as f32).clamp(0.0, 1.0))
        .unwrap_or(0.8);
    Some(TranscriptVerifyResult {
        corrected_content: corrected,
        rationale,
        confidence,
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


pub(crate) fn parse_reflect_merge_output(raw_out: &str, engine_label: String) -> Option<ReflectorResult> {
    if let Some(obj) = extract_json_object(raw_out) {
        let relation = obj
            .get("relation")
            .and_then(Value::as_str)
            .and_then(KnowledgeRelation::parse_str);
        if let Some(body) = obj
            .get("merged_content")
            .or_else(|| obj.get("content"))
            .and_then(Value::as_str)
        {
            let cleaned = strip_markdown_fence(body).trim().to_string();
            if cleaned.len() >= 10 {
                let rationale = obj
                    .get("rationale")
                    .and_then(Value::as_str)
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty());
                let confidence = obj
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .map(|c| (c as f32).clamp(0.0, 1.0));
                return Some(ReflectorResult {
                    relation: relation.unwrap_or(KnowledgeRelation::Duplicates),
                    classified: relation.is_some(),
                    content: cleaned,
                    rationale,
                    confidence,
                    engine_label,
                });
            }
        }
    }

    // Graceful fallback when an LLM or mock returns plain Markdown instead of JSON
    let cleaned = strip_markdown_fence(raw_out).trim().to_string();
    if cleaned.len() < 10 {
        return None;
    }
    Some(ReflectorResult {
        relation: KnowledgeRelation::Duplicates,
        classified: false,
        content: cleaned,
        rationale: None,
        confidence: None,
        engine_label,
    })
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

    #[test]
    fn test_parse_five_knowledge_relations_and_fallback() {
        let conflict_json = r#"{"relation":"CONFLICTS","merged_content":"- Side A: use Bearer\n- Side B: use Basic","rationale":"Auth headers contradict","confidence":0.88}"#;
        let parsed = parse_reflect_merge_output(conflict_json, "llm:test".into()).unwrap();
        assert!(parsed.classified);
        assert_eq!(parsed.relation, KnowledgeRelation::Conflicts);
        assert_eq!(parsed.rationale.as_deref(), Some("Auth headers contradict"));
        assert_eq!(parsed.relation.default_risk(0.95), crate::policy::model::RiskLevel::High);

        let updates_json = r#"{"relation":"UPDATES","merged_content":"Java 21 is required","rationale":"Upgraded from 17"}"#;
        let upd = parse_reflect_merge_output(updates_json, "llm:test".into()).unwrap();
        assert_eq!(upd.relation, KnowledgeRelation::Updates);
        assert_eq!(upd.relation.default_risk(0.95), crate::policy::model::RiskLevel::Medium);

        let fallback = parse_reflect_merge_output("```markdown\n# Plain merged body\n```", "llm:test".into()).unwrap();
        assert!(!fallback.classified);
        assert_eq!(fallback.relation, KnowledgeRelation::Duplicates);
        assert_eq!(fallback.content, "# Plain merged body");
        for input in [
            r##"{"merged_content":"# Missing relation"}"##,
            r##"{"relation":"OTHER","merged_content":"# Invalid relation"}"##,
        ] {
            assert!(!parse_reflect_merge_output(input, "llm:test".into()).unwrap().classified);
        }
    }
}
