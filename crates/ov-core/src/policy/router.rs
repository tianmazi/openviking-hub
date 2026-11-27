use crate::fs_util::sha256_hex;
use crate::platform::{silent_command, PlatformPaths};
use crate::policy::model::{AgentId, PolicyConfig, ResolvedRoute, RouteMatchSource};
use globset::{Glob, GlobSetBuilder};
use std::fs;
use std::path::Path;

pub const USER_MEMORIES_URI: &str = "viking://~/memories/";

/// Resolve the target OpenViking resource and memory URIs for a given working directory.
/// Precedence:
/// 1. Project-local `RESOURCE_POLICY.md` (`resource_uri: viking://...`)
/// 2. Explicit `routing.workspace_mappings`
/// 3. Git remote repository auto-detection (`projects/<repo>/`)
/// 4. Fallback to `routing.inbox_uri`
pub fn resolve_route(policy: &PolicyConfig, cwd: &Path) -> ResolvedRoute {
    resolve_route_for_agent(policy, cwd, None)
}

/// Resolve route with optional per-agent private namespace isolation (`viking://agent/{id}/memories/`).
pub fn resolve_route_for_agent(
    policy: &PolicyConfig,
    cwd: &Path,
    agent: Option<AgentId>,
) -> ResolvedRoute {
    let agent_private = if policy.routing.isolate_agent_namespaces {
        agent.map(|a| format!("viking://agent/{}/memories/", a.as_str()))
    } else {
        None
    };

    // 1. Check project RESOURCE_POLICY.md
    if policy.routing.respect_project_policy_file {
        if let Some((uri, slug)) = read_project_policy_uri(cwd) {
            return ResolvedRoute {
                target_resource_uri: with_trailing_slash(&uri),
                target_memory_uri: USER_MEMORIES_URI.to_string(),
                agent_private_memory_uri: agent_private,
                matched_source: RouteMatchSource::ProjectPolicyFile,
                is_fallback_inbox: false,
                project_slug: slug,
            };
        }
    }

    // 2. Manual workspace mappings
    let paths = PlatformPaths::detect();
    let canonical_cwd = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    for mapping in &policy.routing.workspace_mappings {
        let expanded = paths
            .as_ref()
            .map(|p| p.expand_tilde(&mapping.local_path))
            .unwrap_or_else(|| mapping.local_path.clone());
        let canonical_map = fs::canonicalize(&expanded).unwrap_or(expanded);
        if canonical_cwd.starts_with(&canonical_map) {
            let slug = canonical_map
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string());
            return ResolvedRoute {
                target_resource_uri: with_trailing_slash(&mapping.resource_uri),
                target_memory_uri: USER_MEMORIES_URI.to_string(),
                agent_private_memory_uri: agent_private,
                matched_source: RouteMatchSource::ManualMapping,
                is_fallback_inbox: false,
                project_slug: slug,
            };
        }
    }

    // 3. Auto-detect Git repository remote
    if policy.routing.auto_map_git_repo {
        if let Some(repo_slug) = detect_git_repo_slug(cwd) {
            let prefix = policy.global.default_visibility.resources_prefix();
            return ResolvedRoute {
                target_resource_uri: format!("{}/projects/{}/", prefix, repo_slug),
                target_memory_uri: USER_MEMORIES_URI.to_string(),
                agent_private_memory_uri: agent_private,
                matched_source: RouteMatchSource::GitRemoteAuto,
                is_fallback_inbox: false,
                project_slug: Some(repo_slug),
            };
        }
    }

    // 4. Fallback to inbox
    ResolvedRoute {
        target_resource_uri: with_trailing_slash(&policy.routing.inbox_uri),
        target_memory_uri: USER_MEMORIES_URI.to_string(),
        agent_private_memory_uri: agent_private,
        matched_source: RouteMatchSource::InboxFallback,
        is_fallback_inbox: true,
        project_slug: None,
    }
}

fn with_trailing_slash(uri: &str) -> String {
    let uri = uri.trim();
    if uri.is_empty() || uri == "viking://" {
        return uri.to_string();
    }
    format!("{}/", uri.trim_end_matches('/'))
}

/// Check whether a URI is owned by another agent when namespace isolation is enabled.
pub fn is_foreign_agent_private_uri(policy: &PolicyConfig, caller: AgentId, uri: &str) -> bool {
    if !policy.routing.isolate_agent_namespaces {
        return false;
    }
    let Some(path) = uri.trim().strip_prefix("viking://") else {
        return false;
    };
    let mut segments = Vec::new();
    let path = path.split(['?', '#']).next().unwrap_or_default();
    for segment in path.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    let owner = match segments.as_slice() {
        ["agent", owner, ..] => owner,
        ["memories" | "resources", "agents", owner, ..]
        | ["~" | "user", "memories" | "resources", "agents", owner, ..]
        | ["user", _, "memories" | "resources", "agents", owner, ..] => owner,
        _ => return false,
    };
    AgentId::parse_str(owner).is_some_and(|owner| owner != caller)
}

/// Check if a file path or URI matches any user-protected rule glob (e.g., `CLAUDE.md`, `AGENTS.md`).
pub fn is_protected_rule_path(policy: &PolicyConfig, candidate: &str) -> bool {
    let mut builder = GlobSetBuilder::new();
    for pattern in &policy.routing.protected_rule_globs {
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
        }
    }
    let Ok(set) = builder.build() else {
        return false;
    };
    let normalized = candidate
        .trim_start_matches("viking://")
        .trim_start_matches("~/")
        .replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    set.is_match(normalized)
        || normalized.match_indices('/').any(|(index, _)| set.is_match(&normalized[index + 1..]))
}

fn read_project_policy_uri(cwd: &Path) -> Option<(String, Option<String>)> {
    let policy_file = cwd.join("RESOURCE_POLICY.md");
    let text = fs::read_to_string(policy_file).ok()?;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("resource_uri:") {
            let uri = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if uri.starts_with("viking://") {
                let slug = uri
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .map(|s| s.to_string());
                return Some((uri, slug));
            }
        }
    }
    None
}

fn detect_git_repo_slug(cwd: &Path) -> Option<String> {
    let out = silent_command("git")
        .args(["-C", &cwd.to_string_lossy(), "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let remote = String::from_utf8_lossy(&out.stdout).trim().to_string();
    project_identity(&remote)
}

/// Normalize common Git HTTPS/SSH remotes without retaining credentials.
pub fn project_identity(remote: &str) -> Option<String> {
    let raw = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let (host, path) = if let Some((_, rest)) = raw.split_once("://") {
        rest.split_once('/')?
    } else {
        raw.split_once(':')?
    };
    let host = host.rsplit('@').next()?.to_ascii_lowercase();
    if host.is_empty() || path.is_empty() {
        return None;
    }
    let canonical = format!("{host}/{}", path.trim_matches('/'));
    let slug = parse_repo_slug(raw)?;
    Some(format!("{slug}__{}", &sha256_hex(&canonical)[..12]))
}

/// The mapped resource root defines a shared project peer, independent of the agent.
pub fn project_peer(route: &ResolvedRoute) -> Option<String> {
    if route.is_fallback_inbox || route.target_resource_uri.is_empty() {
        return None;
    }
    Some(format!("project-{}", &sha256_hex(route.target_resource_uri.trim_end_matches('/'))[..24]))
}

pub fn parse_repo_slug(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let last = trimmed.rsplit(['/', ':']).next()?;
    let clean: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if clean.is_empty() {
        None
    } else {
        Some(clean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn stable_project_identity() {
        let a = project_identity("git@github.com:team/payment.git").unwrap();
        assert_eq!(Some(a.clone()), project_identity("https://github.com/team/payment"));
        assert_eq!(Some(a.clone()), project_identity("ssh://git@github.com/team/payment.git"));
        assert_ne!(Some(a.clone()), project_identity("https://github.com/other/payment"));
        assert_ne!(Some(a), project_identity("https://gitlab.example/team/payment"));
    }

    #[test]
    fn test_foreign_agent_private_uri() {
        let mut policy = PolicyConfig::default();
        for uri in [
            "viking://agent/claude-code/memories/note.md",
            "viking://~/memories/agents/claude-code/",
            "viking://~/resources/agents/claude-code",
            "viking://user/alice/memories/agents/claude-code/note.md",
            "viking://~/resources/projects/../agents//claude-code/note.md",
        ] {
            assert!(is_foreign_agent_private_uri(&policy, AgentId::Codex, uri));
            assert!(!is_foreign_agent_private_uri(&policy, AgentId::ClaudeCode, uri));
        }
        for uri in [
            "viking://agent/skills/llm-wiki",
            "viking://~/memories/agents/unknown/note.md",
            "viking://resources/projects/demo/agents/claude-code/note.md",
            "viking://~/memories/agents/codex/note.md",
        ] {
            assert!(!is_foreign_agent_private_uri(&policy, AgentId::Codex, uri));
        }
        policy.routing.isolate_agent_namespaces = false;
        assert!(!is_foreign_agent_private_uri(
            &policy,
            AgentId::Codex,
            "viking://agent/claude-code/memories/"
        ));
    }

    #[test]
    fn test_parse_repo_slug() {
        assert_eq!(
            parse_repo_slug("git@github.com:org/OpenViking-Desktop.git").as_deref(),
            Some("openviking-desktop")
        );
        assert_eq!(
            parse_repo_slug("https://github.com/volcengine/OpenViking/").as_deref(),
            Some("openviking")
        );
        assert_eq!(
            parse_repo_slug("https://github.com/volcengine/OpenViking").as_deref(),
            Some("openviking")
        );
    }

    #[test]
    fn test_protected_rule_globs() {
        let policy = PolicyConfig::default();
        assert!(is_protected_rule_path(&policy, "CLAUDE.md"));
        assert!(is_protected_rule_path(
            &policy,
            "viking://resources/projects/demo/CLAUDE.md"
        ));
        assert!(is_protected_rule_path(
            &policy,
            "viking://resources/projects/demo/CLAUDE.md/"
        ));
        assert!(is_protected_rule_path(&policy, "viking://resources/projects/foo/CLAUDE.md"));
        assert!(is_protected_rule_path(&policy, "AGENTS.md"));
        assert!(is_protected_rule_path(
            &policy,
            "viking://resources/projects/demo/.agents/rules/foo.md"
        ));
        assert!(!is_protected_rule_path(
            &policy,
            "viking://resources/projects/demo/other.agents/rules/foo.md"
        ));
        assert!(!is_protected_rule_path(
            &policy,
            "viking://user/memories/preferences/git.md"
        ));
        assert!(!is_protected_rule_path(&policy, "viking://~/memories/preferences/style.md"));
    }

    #[test]
    fn test_project_policy_file_precedence() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("RESOURCE_POLICY.md"),
            "---\nresource_uri: viking://resources/projects/custom-app\n---\n",
        )
        .unwrap();
        let policy = PolicyConfig::default();
        let route = resolve_route_for_agent(&policy, dir.path(), Some(AgentId::Codex));
        assert_eq!(route.matched_source, RouteMatchSource::ProjectPolicyFile);
        assert_eq!(route.target_resource_uri, "viking://resources/projects/custom-app/");
        assert_eq!(
            route.agent_private_memory_uri.as_deref(),
            Some("viking://agent/codex/memories/")
        );
        assert_eq!(route.target_memory_uri, USER_MEMORIES_URI);
        let other = resolve_route_for_agent(&policy, dir.path(), Some(AgentId::ClaudeCode));
        assert_eq!(project_peer(&route), project_peer(&other));
        assert!(!route.is_fallback_inbox);
    }
}
