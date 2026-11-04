//! AGT-12 v1: `web_search` and `fetch_url`, the read-only web tools chat
//! offers when Tools is on. Search uses DuckDuckGo's HTML endpoint (no key,
//! no account); requests go straight from the device.
//!
//! The network is injected as a `Fetcher` so the parsers and the tool
//! contract are tested offline against a saved results page.
use serde::Serialize;

use super::ToolRegistry;

/// GETs a URL and returns the body as text, or a human-readable error.
pub type Fetcher = Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// How many results `web_search` hands the model.
pub const MAX_RESULTS: usize = 8;
/// `fetch_url` caps the page text it returns (bytes, on a char boundary).
pub const MAX_PAGE_BYTES: usize = 20 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Registers `web_search` and `fetch_url` on `registry`, using `fetch` for
/// the network.
pub fn register_web_tools(registry: &mut ToolRegistry, fetch: Fetcher) {
    let fetch = std::sync::Arc::new(fetch);
    let f = fetch.clone();
    registry.register("web_search", move |args| web_search(args, &**f));
    registry.register("fetch_url", move |args| fetch_url(args, &**fetch));
}

fn arg(arguments_json: &str, key: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(arguments_json).map_err(|e| format!("bad arguments: {e}"))?;
    v.get(key)
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing \"{key}\""))
}

/// `{"query": "..."}` → a JSON array of up to `MAX_RESULTS` results.
pub fn web_search(
    arguments_json: &str,
    fetch: &dyn Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    let query = arg(arguments_json, "query")?;
    let url = format!("https://html.duckduckgo.com/html/?q={}", url_encode(&query));
    let html = fetch(&url)?;
    let results = parse_ddg_html(&html);
    if results.is_empty() {
        return Err("no results".into());
    }
    serde_json::to_string(&results).map_err(|e| e.to_string())
}

/// `{"url": "https://..."}` → the page's readable text, capped.
pub fn fetch_url(
    arguments_json: &str,
    fetch: &dyn Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    let url = arg(arguments_json, "url")?;
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) URLs can be read".into());
    }
    Ok(readable_text(&fetch(&url)?, MAX_PAGE_BYTES))
}

/// Results from DuckDuckGo's HTML page: each `result__a` link (title + URL)
/// and the `result__snippet` that follows it. Ads are skipped.
/// ponytail: string scanning, not an HTML parser; DuckDuckGo's markup is
/// stable and a fixture test catches a change.
pub fn parse_ddg_html(html: &str) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for block in html.split("class=\"result ").skip(1) {
        // `class="result result--ad …"`: the block starts with the ad marker.
        if block.starts_with("result--ad") {
            continue;
        }
        let Some(a) = block.find("class=\"result__a\"") else {
            continue;
        };
        let tag = &block[a..];
        let Some(url) = attr(tag, "href").map(|u| resolve_ddg_url(&u)) else {
            continue;
        };
        let title = inner_text(tag);
        let snippet = block
            .find("class=\"result__snippet\"")
            .map(|i| inner_text(&block[i..]))
            .unwrap_or_default();
        if !url.is_empty() && !title.is_empty() && !out.iter().any(|r: &SearchResult| r.url == url)
        {
            out.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
        if out.len() == MAX_RESULTS {
            break;
        }
    }
    out
}

/// The value of `name="..."` in the tag starting at `tag`.
fn attr(tag: &str, name: &str) -> Option<String> {
    let end = tag.find('>')?;
    let head = &tag[..end];
    let key = format!("{name}=\"");
    let start = head.find(&key)? + key.len();
    let len = head[start..].find('"')?;
    Some(decode_entities(&head[start..start + len]))
}

/// Text between the end of the tag at `tag` and its closing `</a>`.
fn inner_text(tag: &str) -> String {
    let Some(open) = tag.find('>') else {
        return String::new();
    };
    let rest = &tag[open + 1..];
    let close = rest.find("</a>").unwrap_or(rest.len());
    collapse(&decode_entities(&strip_tags(&rest[..close])))
}

/// Older result pages link through `//duckduckgo.com/l/?uddg=<encoded>`.
fn resolve_ddg_url(href: &str) -> String {
    if let Some(i) = href.find("uddg=") {
        let enc = &href[i + 5..];
        let enc = &enc[..enc.find('&').unwrap_or(enc.len())];
        return url_decode(enc);
    }
    href.to_string()
}

/// A page's readable text: scripts, styles and tags removed, entities
/// decoded, whitespace collapsed, capped at `max` bytes.
pub fn readable_text(html: &str, max: usize) -> String {
    let mut s = html.to_string();
    for tag in ["script", "style", "noscript", "svg", "head"] {
        s = remove_blocks(&s, tag);
    }
    let text = collapse(&decode_entities(&strip_tags(&s)));
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn remove_blocks(s: &str, tag: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(a) = lower[i..].find(&open).map(|p| p + i) {
        out.push_str(&s[i..a]);
        match lower[a..].find(&close) {
            Some(b) => i = a + b + close.len(),
            None => return out,
        }
    }
    out.push_str(&s[i..]);
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDG: &str = include_str!("../../tests/fixtures/ddg-latest-macos.html");

    #[test]
    fn parses_a_saved_duckduckgo_page() {
        let r = parse_ddg_html(DDG);
        assert!(r.len() >= 5, "got {} results", r.len());
        assert!(r.len() <= MAX_RESULTS);
        assert!(r[0].url.starts_with("https://"), "{:?}", r[0]);
        assert!(
            !r[0].title.is_empty() && !r[0].snippet.is_empty(),
            "{:?}",
            r[0]
        );
        assert!(
            !r[0].snippet.contains("<b>"),
            "tags stripped: {}",
            r[0].snippet
        );
        assert!(r.iter().any(|x| x.url.contains("apple.com")));
    }

    #[test]
    fn resolves_redirect_links() {
        assert_eq!(
            resolve_ddg_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%3Fb%3D1&rut=x"),
            "https://example.com/a?b=1"
        );
    }

    #[test]
    fn readable_text_drops_scripts_and_caps() {
        let html = "<html><head><title>x</title></head><body><script>var a=1;</script><p>Hello &amp; welcome</p><style>p{}</style><p>world</p></body></html>";
        assert_eq!(readable_text(html, 1000), "Hello & welcome world");
        let long = format!("<p>{}</p>", "é".repeat(100));
        let capped = readable_text(&long, 11);
        assert!(capped.ends_with('…') && capped.len() <= 11 + '…'.len_utf8());
    }

    #[test]
    fn tools_use_the_injected_fetcher() {
        let mut reg = ToolRegistry::new();
        register_web_tools(
            &mut reg,
            Box::new(|url: &str| {
                if url.contains("duckduckgo") {
                    assert!(url.contains("q=latest+macOS"), "{url}");
                    Ok(DDG.to_string())
                } else {
                    Ok("<p>page text</p>".to_string())
                }
            }),
        );
        let out = reg
            .call("web_search", r#"{"query":"latest macOS"}"#)
            .unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
        assert!(!parsed.is_empty());
        assert_eq!(
            reg.call("fetch_url", r#"{"url":"https://example.com"}"#)
                .unwrap(),
            "page text"
        );
        assert!(reg
            .call("fetch_url", r#"{"url":"file:///etc/passwd"}"#)
            .is_err());
        assert!(reg.call("web_search", r#"{}"#).is_err());
    }
}
