//! Module documentation routing, markdown/block rendering, and document URL linking.

use crate::config::ConfigStore;
use crate::gateway::modules::ModuleDescriptor;
use crate::http::write_error;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::Path;

fn extract_query_param(path: &str, key: &str) -> Option<String> {
    let (_, query) = path.split_once('?')?;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key && !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// Extracts a module ID from paths like `/modules/{id}/doc` or `/modules/{id}/docs`.
pub fn extract_module_id_from_doc_path(path: &str) -> Option<String> {
    if let Some(id) = extract_query_param(path, "id") {
        return Some(id);
    }
    let clean = path.split('?').next().unwrap_or(path).trim_matches('/');
    let parts: Vec<&str> = clean.split('/').collect();
    if parts.len() >= 3 && parts[0] == "modules" {
        let sub = parts[2];
        if sub == "doc" || sub == "docs" || sub == "documentation" || sub == "doc.html" {
            return Some(parts[1].to_string());
        }
    }
    None
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Simple safe converter for markdown subsets (headers, lists, code blocks, bold, links).
fn render_markdown_to_safe_html(markdown: &str) -> String {
    let mut html = String::with_capacity(markdown.len() * 2);
    let mut in_code_block = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_code_block {
                html.push_str("</code></pre>\n");
                in_code_block = false;
            } else {
                html.push_str("<pre class=\"code-block\"><code>");
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            html.push_str(&escape_html(line));
            html.push('\n');
            continue;
        }

        if let Some(rest) = line.strip_prefix("### ") {
            html.push_str("<h3>");
            html.push_str(&escape_html(rest.trim()));
            html.push_str("</h3>\n");
        } else if let Some(rest) = line.strip_prefix("## ") {
            html.push_str("<h2>");
            html.push_str(&escape_html(rest.trim()));
            html.push_str("</h2>\n");
        } else if let Some(rest) = line.strip_prefix("# ") {
            html.push_str("<h1>");
            html.push_str(&escape_html(rest.trim()));
            html.push_str("</h1>\n");
        } else if let Some(rest) = line.strip_prefix("- ") {
            html.push_str("<li>");
            html.push_str(&render_inline_markup(rest.trim()));
            html.push_str("</li>\n");
        } else if trimmed.is_empty() {
            html.push_str("<div class=\"spacer\"></div>\n");
        } else {
            html.push_str("<p>");
            html.push_str(&render_inline_markup(line));
            html.push_str("</p>\n");
        }
    }

    if in_code_block {
        html.push_str("</code></pre>\n");
    }
    html
}

fn render_inline_markup(text: &str) -> String {
    let escaped = escape_html(text);
    // Replace `code` with <code>code</code>
    let mut out = String::new();
    let parts = escaped.split('`');
    let mut is_code = false;
    for part in parts {
        if is_code {
            out.push_str("<code>");
            out.push_str(part);
            out.push_str("</code>");
        } else {
            out.push_str(part);
        }
        is_code = !is_code;
    }
    out
}

fn read_local_doc_file(module_id: &str, relative_path: &str) -> Option<String> {
    if relative_path.contains("..") {
        return None;
    }
    let state_dir = crate::gateway::installer::get_state_dir();
    let candidates = [
        state_dir
            .join("modules")
            .join(module_id)
            .join(relative_path),
        Path::new("addons").join(module_id).join(relative_path),
        Path::new(relative_path).to_path_buf(),
    ];
    for cand in &candidates {
        if cand.is_file() {
            if let Ok(content) = std::fs::read_to_string(cand) {
                return Some(content);
            }
        }
    }
    None
}

/// Renders HTML document viewer page for a module.
pub fn handle_module_doc_page(
    client: &mut TcpStream,
    request_path: &str,
    store: &ConfigStore,
) -> io::Result<()> {
    let Some(raw_id) = extract_module_id_from_doc_path(request_path) else {
        return write_error(
            client,
            400,
            "Bad Request",
            "Missing module identifier in path",
        );
    };

    let reg = store.module_registry();
    let guard = match reg.read() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };

    let resolved_id = guard.resolve_module_id(&raw_id).unwrap_or(raw_id);
    let Some(desc) = guard.get_module(&resolved_id).cloned() else {
        return write_error(
            client,
            404,
            "Not Found",
            &format!("Module '{resolved_id}' not found"),
        );
    };
    drop(guard);

    // If an external doc URL is set and no custom doc_content block was provided, redirect directly
    let has_doc_content = desc
        .doc_content
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if !has_doc_content {
        if let Some(ref url) = desc.doc_url {
            let trimmed = url.trim();
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                let redirect = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {trimmed}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                client.write_all(redirect.as_bytes())?;
                return client.flush();
            }
        }
    }

    let rendered_html = generate_doc_page_html(&desc);
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-cache, no-store, must-revalidate\r\nAccess-Control-Allow-Origin: *\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        rendered_html.len(),
        rendered_html
    )?;
    client.flush()
}

/// JSON endpoint returning documentation metadata for agents and dashboard APIs.
pub fn handle_get_module_doc(
    client: &mut TcpStream,
    request_path: &str,
    store: &ConfigStore,
) -> io::Result<()> {
    let Some(raw_id) = extract_module_id_from_doc_path(request_path) else {
        return write_error(
            client,
            400,
            "Bad Request",
            "Missing module identifier in path",
        );
    };

    let reg = store.module_registry();
    let guard = match reg.read() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };

    let resolved_id = guard.resolve_module_id(&raw_id).unwrap_or(raw_id);
    let Some(desc) = guard.get_module(&resolved_id).cloned() else {
        return write_error(
            client,
            404,
            "Not Found",
            &format!("Module '{resolved_id}' not found"),
        );
    };
    drop(guard);

    let doc_url = desc
        .doc_url
        .clone()
        .unwrap_or_else(|| format!("/modules/{}/doc", desc.id));
    let resp = serde_json::json!({
        "ok": true,
        "status": "ok",
        "id": desc.id,
        "name": desc.name,
        "namespace": desc.namespace,
        "kind": desc.kind,
        "version": desc.version,
        "doc_url": doc_url,
        "doc_content": desc.doc_content,
        "capabilities": desc.capabilities,
        "source": desc.source,
    });
    let bytes = serde_json::to_vec(&resp).unwrap_or_default();
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )?;
    client.write_all(&bytes)?;
    client.flush()
}

fn generate_doc_page_html(desc: &ModuleDescriptor) -> String {
    let icon = escape_html(desc.icon.as_deref().unwrap_or("◫"));
    let name = escape_html(&desc.name);
    let id = escape_html(&desc.id);
    let ns = escape_html(&desc.namespace);
    let ver = desc.version.as_deref().unwrap_or("1.0.0");
    let doc_url_val = desc
        .doc_url
        .clone()
        .unwrap_or_else(|| format!("/modules/{id}/doc"));

    let ext_link_btn = if doc_url_val.starts_with("http://") || doc_url_val.starts_with("https://")
    {
        format!(
            "<a href=\"{}\" class=\"nav-btn ext-btn\" target=\"_blank\" rel=\"noopener noreferrer\">Official External Docs ↗</a>",
            escape_html(&doc_url_val)
        )
    } else {
        String::new()
    };

    let content_body = if let Some(ref text) = desc.doc_content {
        render_markdown_to_safe_html(text)
    } else if let Some(local_text) = read_local_doc_file(&desc.id, "README.md") {
        render_markdown_to_safe_html(&local_text)
    } else {
        format!(
            "<div class=\"empty-docs\">
              <h3>No Document Block Configured</h3>
              <p>Module <code>{id}</code> has no inline document block yet.</p>
              <p>You can add a document block or link an external documentation URL in the module configuration.</p>
              <div class=\"btns\" style=\"margin-top:16px;\">
                <a href=\"/modules/{id}/config\" class=\"nav-btn primary-btn\">Open Module Configuration ↗</a>
              </div>
            </div>"
        )
    };

    let caps_html = desc
        .capabilities
        .iter()
        .map(|c| format!("<span class=\"pill cap\">{}</span>", escape_html(c)))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{name} Documentation — Toolcase Gateway</title>
<style>
  *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    background: #0d0f14; color: #e2e8f0; min-height: 100vh; padding: 32px 20px; line-height: 1.6;
  }}
  .container {{ max-width: 860px; margin: 0 auto; }}
  .header {{
    display: flex; justify-content: space-between; align-items: flex-start;
    margin-bottom: 24px; gap: 16px; flex-wrap: wrap; padding-bottom: 20px;
    border-bottom: 1px solid #1e2433;
  }}
  h1 {{ font-size: 1.6rem; font-weight: 700; color: #f8fafc; display: flex; align-items: center; gap: 10px; }}
  h2 {{ font-size: 1.25rem; font-weight: 600; color: #f1f5f9; margin: 20px 0 10px 0; }}
  h3 {{ font-size: 1.05rem; font-weight: 600; color: #cbd5e1; margin: 16px 0 8px 0; }}
  p {{ margin-bottom: 12px; color: #cbd5e1; font-size: 0.95rem; }}
  li {{ margin-left: 20px; margin-bottom: 6px; color: #cbd5e1; font-size: 0.92rem; }}
  .spacer {{ height: 12px; }}
  .meta-bar {{ display: flex; gap: 10px; align-items: center; flex-wrap: wrap; margin-top: 8px; }}
  .pill {{ padding: 3px 10px; border-radius: 999px; font-size: 0.78rem; font-weight: 600; background: #1e293b; color: #94a3b8; border: 1px solid #334155; }}
  .pill.cap {{ background: #0c4a6e; color: #7dd3fc; border-color: #0284c7; }}
  .pill.route {{ background: #132e27; color: #6ee7b7; border-color: #059669; font-family: monospace; }}
  .nav-actions {{ display: flex; gap: 10px; flex-wrap: wrap; }}
  .nav-btn {{
    display: inline-flex; align-items: center; gap: 6px; padding: 8px 14px; border-radius: 8px;
    background: #1e2433; border: 1px solid #2e364f; color: #cbd5e1; text-decoration: none;
    font-size: 0.85rem; font-weight: 600; transition: all 0.15s ease;
  }}
  .nav-btn:hover {{ background: #262f42; color: #fff; }}
  .primary-btn {{ background: #0284c7; border-color: #0369a1; color: #fff; }}
  .primary-btn:hover {{ background: #0369a1; }}
  .ext-btn {{ background: #1e3a8a; border-color: #1d4ed8; color: #bfdbfe; }}
  .ext-btn:hover {{ background: #1d4ed8; color: #fff; }}
  .card {{
    background: #151922; border: 1px solid #232a3b; border-radius: 12px; padding: 28px;
    margin-bottom: 24px; box-shadow: 0 4px 12px rgba(0,0,0,0.3);
  }}
  code {{
    background: #090d14; padding: 2px 6px; border-radius: 4px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.88em; color: #38bdf8; border: 1px solid #1e2638;
  }}
  .code-block {{
    background: #090d14; border: 1px solid #222b3e; border-radius: 8px; padding: 14px 16px;
    margin: 14px 0; overflow-x: auto;
  }}
  .code-block code {{ background: transparent; padding: 0; border: none; color: #e2e8f0; font-size: 0.88rem; }}
  .empty-docs {{ padding: 24px; background: #12161f; border-radius: 8px; border: 1px dashed #2e364f; text-align: center; }}
  .footer {{ text-align: center; font-size: 0.8rem; color: #64748b; margin-top: 32px; }}
  .footer a {{ color: #94a3b8; text-decoration: none; }}
  .footer a:hover {{ color: #38bdf8; }}
</style>
</head>
<body>
<div class="container">
  <div class="header">
    <div>
      <h1><span>{icon}</span> {name}</h1>
      <div class="meta-bar">
        <span class="pill">v{ver}</span>
        <span class="pill route">{ns}</span>
        {caps_html}
      </div>
    </div>
    <div class="nav-actions">
      {ext_link_btn}
      <a href="/modules/{id}/config" class="nav-btn">Configure ↗</a>
      <a href="/dashboard" class="nav-btn">Dashboard</a>
    </div>
  </div>

  <div class="card">
    {content_body}
  </div>

  <div class="footer">
    Toolcase Gateway Module Documentation · <a href="/modules/{id}/config">Module Settings</a> · <a href="/dashboard">Overview</a>
  </div>
</div>
</body>
</html>"#
    )
}
