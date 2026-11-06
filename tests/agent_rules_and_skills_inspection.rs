use std::borrow::Cow;
use std::fs;
use std::marker::PhantomData;
use std::path::Path;

fn read_text(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))
}

fn assert_contains_all(content: &str, path_label: &str, required: &[&str]) {
    for &needle in required {
        assert!(
            content.contains(needle),
            "Expected `{}` to contain `{}`",
            path_label,
            needle
        );
    }
}

#[test]
fn test_repository_agents_and_gemini_rules_exist_and_synchronized() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let agents_path = manifest_dir.join("AGENTS.md");
    let gemini_path = manifest_dir.join("GEMINI.md");

    let agents_content =
        read_text(&agents_path).expect("Repository AGENTS.md must exist and be readable");
    let gemini_content =
        read_text(&gemini_path).expect("Repository GEMINI.md must exist and be readable");

    assert!(!agents_content.trim().is_empty());
    assert!(!gemini_content.trim().is_empty());

    let (_, agents_body) = agents_content
        .split_once("## Plan-writing rule")
        .expect("AGENTS.md must contain ## Plan-writing rule");
    let (_, gemini_body) = gemini_content
        .split_once("## Plan-writing rule")
        .expect("GEMINI.md must contain ## Plan-writing rule");
    assert_eq!(
        agents_body.trim(),
        gemini_body.trim(),
        "Repository AGENTS.md and GEMINI.md rule bodies must stay synchronized"
    );

    let required_tokens = [
        // unwrap() safety & replacement table
        "unwrap()",
        "panic",
        "unwrap_or(val)",
        "unwrap_or_else(fn)",
        "unwrap_or_default()",
        "expect(\"msg\")",
        // Lifetimes ('a) & std::borrow::Cow
        "Lifetimes (`'a`)",
        "Cow<'a, B>",
        "Clone-on-Write",
        "LogEntry<'a>",
        "Cow::Borrowed",
        "Cow::Owned",
        "to_mut()",
        "#[serde(borrow)]",
        // Anti-patterns
        "&String",
        "&Vec",
        "let _ =",
        ".enumerate()",
        ".join(",
        "unsafe",
        "cargo clippy",
        // Advanced AI Rust Code Generation Rules
        "Parse, Don't Validate",
        "EmailAddress",
        "Typestate Pattern",
        "Connection<State>",
        "thiserror",
        "anyhow",
        "with_capacity",
        "MutexGuard",
        "unwrap_used = \"deny\"",
    ];

    assert_contains_all(&agents_content, "repo AGENTS.md", &required_tokens);
    assert_contains_all(&gemini_content, "repo GEMINI.md", &required_tokens);
}

#[test]
fn test_global_rules_and_skills_exist_and_contain_required_sections() {
    // 1. Global AGENTS.md and GEMINI.md
    let global_agents = read_text(Path::new("/home/ubuntu/AGENTS.md"))
        .expect("Global /home/ubuntu/AGENTS.md must exist");
    let global_gemini = read_text(Path::new("/home/ubuntu/GEMINI.md"))
        .expect("Global /home/ubuntu/GEMINI.md must exist");

    let global_required = [
        "unwrap()",
        "unwrap_or(val)",
        "unwrap_or_else(fn)",
        "unwrap_or_default()",
        "Lifetimes (`'a`)",
        "Cow<'a, B>",
        "Cow::Borrowed",
        "Cow::Owned",
        "to_mut()",
        "#[serde(borrow)]",
        "&String",
        "&Vec<T>",
        "let _ =",
        ".enumerate()",
        "cargo clippy",
        "Parse, Don't Validate",
        "EmailAddress",
        "Typestate Pattern",
        "thiserror",
        "anyhow",
        "with_capacity",
        "MutexGuard",
    ];
    assert_contains_all(&global_agents, "/home/ubuntu/AGENTS.md", &global_required);
    assert_contains_all(&global_gemini, "/home/ubuntu/GEMINI.md", &global_required);

    // 2. Clean-architecture plugin global rule
    let plugin_rule_path =
        Path::new("/home/ubuntu/.gemini/config/plugins/clean-architecture/rules/AGENTS.md");
    let plugin_rule = read_text(plugin_rule_path).expect("Plugin AGENTS.md must exist");
    assert_contains_all(
        &plugin_rule,
        "plugin clean-architecture AGENTS.md",
        &[
            "No Production `unwrap()`",
            "Cow<'a, B>",
            "#[serde(borrow)]",
            "Parse, Don't Validate",
            "Typestate Pattern",
            "thiserror",
            "anyhow",
            "with_capacity",
            "MutexGuard",
            "cargo clippy",
        ],
    );

    // 3. All 3 rust-safety-and-antipatterns skills + references
    let skill_dirs = [
        "/home/ubuntu/.agents/skills/rust-safety-and-antipatterns",
        "/home/ubuntu/.gemini/config/skills/rust-safety-and-antipatterns",
        "/home/ubuntu/.gemini/skills/rust-safety-and-antipatterns",
    ];
    for dir in &skill_dirs {
        let skill_md_path = Path::new(dir).join("SKILL.md");
        let ref_md_path = Path::new(dir).join("references/lifetimes_and_cow.md");

        let skill_md = read_text(&skill_md_path).expect("SKILL.md must exist");
        let ref_md = read_text(&ref_md_path).expect("lifetimes_and_cow.md must exist");

        assert_contains_all(
            &skill_md,
            &skill_md_path.display().to_string(),
            &[
                "name: rust-safety-and-antipatterns",
                "unwrap_or(val)",
                "unwrap_or_else(fn)",
                "unwrap_or_default()",
                "PhanTichLog<'a>",
                "Cow::Borrowed",
                "Cow::Owned",
                "to_mut()",
                "Parse, Don't Validate",
                "EmailAddress",
                "Typestate Pattern",
                "Connection<Disconnected>",
                "Connection<Connected>",
                "thiserror",
                "anyhow",
                "with_capacity",
                "MutexGuard",
            ],
        );

        assert_contains_all(
            &ref_md,
            &ref_md_path.display().to_string(),
            &[
                "PhanTichLog<'a>",
                "Cow::Borrowed",
                "Cow::Owned",
                "to_mut()",
                "#[serde(borrow)]",
                "Decision Matrix",
            ],
        );
    }

    // 4. Both clean-architecture rust_clean_architecture.md references
    let clean_arch_refs = [
        "/home/ubuntu/.gemini/config/plugins/clean-architecture/skills/clean-architecture/references/rust_clean_architecture.md",
        "/home/ubuntu/.gemini/skills/clean-architecture/references/rust_clean_architecture.md",
    ];
    for ref_path in &clean_arch_refs {
        let content =
            read_text(Path::new(ref_path)).expect("rust_clean_architecture.md must exist");
        assert_contains_all(
            &content,
            ref_path,
            &[
                "unwrap()",
                "unwrap_or(val)",
                "unwrap_or_else(fn)",
                "unwrap_or_default()",
                "PhanTichLog<'a>",
                "Cow::Borrowed",
                "Cow::Owned",
                "to_mut()",
                "Parse, Don't Validate",
                "UserId",
                "EmailAddress",
                "Connection<Disconnected>",
                "Connection<Connected>",
                "thiserror",
                "anyhow",
                "Vec::with_capacity",
                "MutexGuard",
            ],
        );
    }
}

// ============================================================================
// Executable verification of the Rust patterns documented in the rules/skills
// ============================================================================

#[derive(Debug, PartialEq, Eq)]
pub struct LogEntry<'a> {
    pub level: &'a str,
    pub message: &'a str,
}

impl<'a> LogEntry<'a> {
    pub fn parse(line: &'a str) -> Option<Self> {
        let (level, message) = line.split_once(": ")?;
        Some(Self { level, message })
    }
}

pub fn sanitize_identifier(input: &str) -> Cow<'_, str> {
    if !input.contains(' ') {
        return Cow::Borrowed(input);
    }
    Cow::Owned(input.replace(' ', "_"))
}

pub fn ensure_https(url: &str) -> Cow<'_, str> {
    let mut cow = Cow::Borrowed(url);
    if !cow.starts_with("https://") {
        cow.to_mut().insert_str(0, "https://");
    }
    cow
}

pub fn safe_fallback_or(input: Option<i32>, default_val: i32) -> i32 {
    input.unwrap_or(default_val)
}

pub fn safe_fallback_or_else<F: FnOnce() -> i32>(input: Option<i32>, fallback_fn: F) -> i32 {
    input.unwrap_or_else(fallback_fn)
}

pub fn safe_fallback_default(input: Option<i32>) -> i32 {
    input.unwrap_or_default()
}

pub fn sum_slice(values: &[i32]) -> i32 {
    values.iter().sum()
}

pub fn format_indexed_items(items: &[&str]) -> String {
    let mut parts = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        parts.push(format!("{idx}:{item}"));
    }
    parts.join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    InvalidEmail,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EmailAddress(String);

impl EmailAddress {
    pub fn parse(raw: impl Into<String>) -> Result<Self, DomainError> {
        let s = raw.into();
        if s.contains('@') && s.contains('.') {
            Ok(Self(s))
        } else {
            Err(DomainError::InvalidEmail)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct Disconnected;
pub struct Connected;

pub struct Connection<State> {
    endpoint: String,
    _state: PhantomData<State>,
}

impl Connection<Disconnected> {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            _state: PhantomData,
        }
    }

    pub fn connect(self) -> Result<Connection<Connected>, DomainError> {
        Ok(Connection {
            endpoint: self.endpoint,
            _state: PhantomData,
        })
    }
}

impl Connection<Connected> {
    pub fn send(&self, payload: &[u8]) -> Result<usize, DomainError> {
        Ok(self.endpoint.len() + payload.len())
    }
}

#[test]
fn test_executable_rust_safety_and_zero_clone_patterns() {
    // 1. Safe Option/Result handling without unwrap()
    assert_eq!(safe_fallback_or(None, 42), 42);
    assert_eq!(safe_fallback_or(Some(7), 42), 7);
    assert_eq!(safe_fallback_or_else(None, || 21 * 2), 42);
    assert_eq!(safe_fallback_default(None), 0);

    // 2. Zero-copy lifetime parsing: verify pointers lie inside original buffer!
    let raw_log = String::from("INFO: gateway started on port 8080");
    let raw_start = raw_log.as_ptr() as usize;
    let raw_end = raw_start + raw_log.len();

    let parsed = LogEntry::parse(&raw_log).expect("valid log entry");
    assert_eq!(parsed.level, "INFO");
    assert_eq!(parsed.message, "gateway started on port 8080");

    let level_ptr = parsed.level.as_ptr() as usize;
    let msg_ptr = parsed.message.as_ptr() as usize;
    assert!(
        level_ptr >= raw_start && level_ptr < raw_end,
        "level slice must borrow directly from input buffer without heap allocation"
    );
    assert!(
        msg_ptr >= raw_start && msg_ptr < raw_end,
        "message slice must borrow directly from input buffer without heap allocation"
    );

    // 3. Cow<'a, str> conditional transformation
    let clean_id = sanitize_identifier("rust_clean_arch");
    assert!(
        matches!(clean_id, Cow::Borrowed(_)),
        "Expected Cow::Borrowed when no spaces are present"
    );
    assert_eq!(clean_id, "rust_clean_arch");

    let dirty_id = sanitize_identifier("rust clean arch");
    assert!(
        matches!(dirty_id, Cow::Owned(_)),
        "Expected Cow::Owned when spaces are replaced"
    );
    assert_eq!(dirty_id, "rust_clean_arch");

    // 4. Cow::to_mut() lazy mutation
    let already_https = ensure_https("https://example.com");
    assert!(
        matches!(already_https, Cow::Borrowed(_)),
        "Expected Cow::Borrowed when url already starts with https://"
    );
    assert_eq!(already_https, "https://example.com");

    let bare_domain = ensure_https("example.com");
    assert!(
        matches!(bare_domain, Cow::Owned(_)),
        "Expected Cow::Owned when https:// prefix is inserted via to_mut()"
    );
    assert_eq!(bare_domain, "https://example.com");

    // 5. Idiomatic slice parameters, .enumerate(), and .join()
    let arr = [10, 20, 30];
    let vec_items = vec![10, 20, 30];
    assert_eq!(sum_slice(&arr), 60);
    assert_eq!(sum_slice(&vec_items), 60);
    assert_eq!(
        format_indexed_items(&["alpha", "beta", "gamma"]),
        "0:alpha 1:beta 2:gamma"
    );

    // 6. Type-Driven Design: Newtype ("Parse, Don't Validate") & Typestate Pattern
    assert_eq!(
        EmailAddress::parse("invalid-email"),
        Err(DomainError::InvalidEmail)
    );
    let valid_email = EmailAddress::parse("agent@example.com").expect("valid email");
    assert_eq!(valid_email.as_str(), "agent@example.com");

    let disconnected = Connection::<Disconnected>::new("127.0.0.1:8080");
    let connected = disconnected.connect().expect("connect succeeds");
    let bytes_recorded = connected.send(b"ping").expect("send succeeds");
    assert_eq!(bytes_recorded, "127.0.0.1:8080".len() + 4);
}
