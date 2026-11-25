//! The oracle for an SPA shell's Content-Security-Policy, shared by both apps'
//! real-shell checks: Ledgeline's `crates/ledgeline-server/tests/spa_shell_csp.rs`
//! and the invoicer's `spa_shell_csp` unit test
//! (`invoicer/crates/ledgeline-invoicer/src/app/spa_shell_csp_tests.rs`).
//! Each includes this file with `#[path]`; it is not a test target of its own
//! (cargo only builds `tests/*.rs` and `tests/*/main.rs`).
//!
//! The policy is computed at request time from the served bytes
//! (`ledgeline_shell::security::shell_csp`), and every other CSP test feeds it
//! hand-written HTML or a `build.rs` placeholder page, which carries no script
//! at all. So nothing else checks the policy against a real SvelteKit shell:
//! an upgrade that emitted an import map, an inline event handler, or a script
//! tag the production scanner mis-parses would break the app to a blank window
//! with every test green.
//!
//! **An independent parser.** The checks re-derive the script inventory with a
//! small attribute-aware tag scanner of their own (quoted values,
//! case-insensitive names, raw-text script bodies) rather than calling the
//! production one. Reusing it would only prove it agrees with itself.

// Each includer uses a different subset (Ledgeline's file also tests the
// scanner directly).
#![allow(dead_code)]

use std::collections::BTreeMap;

/// Present in every SvelteKit shell: the bootstrap script assigns
/// `__sveltekit_<hash> = { base: … }`. A `build.rs` placeholder has no script.
pub const SVELTEKIT_MARKER: &str = "__sveltekit";

/// One start tag: its lower-cased name and attributes (name lower-cased, value
/// unquoted; `None` for a bare attribute such as `async`).
#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub attrs: BTreeMap<String, Option<String>>,
}

impl Tag {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .get(name)
            .map(|value| value.as_deref().unwrap_or(""))
    }
}

/// A `<script>` element: its start tag and its exact raw-text body.
#[derive(Debug)]
pub struct Script {
    pub tag: Tag,
    pub body: String,
}

/// Every start tag in `html`, plus every `<script>` element with its body.
///
/// Intentionally NOT the production scanner's algorithm: this one honours
/// quoted attribute values (a `>` inside quotes does not end the tag), matches
/// names case-insensitively, treats `src` as an attribute NAME rather than a
/// substring (`data-src=` is not `src=`), and skips comments.
pub fn scan(html: &str) -> (Vec<Tag>, Vec<Script>) {
    let bytes = html.as_bytes();
    let mut tags = Vec::new();
    let mut scripts = Vec::new();
    let mut at = 0;
    while let Some(offset) = html[at..].find('<') {
        let open = at + offset;
        if html[open..].starts_with("<!--") {
            let end = html[open..]
                .find("-->")
                .map_or(html.len(), |e| open + e + 3);
            at = end;
            continue;
        }
        if !bytes.get(open + 1).is_some_and(u8::is_ascii_alphabetic) {
            at = open + 1;
            continue;
        }
        let (tag, after) = parse_start_tag(html, open + 1);
        at = after;
        // Raw-text elements: their content is not markup, so skip to the close.
        if tag.name == "script" || tag.name == "style" {
            let close_tag = format!("</{}", tag.name);
            let close = find_ci(&html[at..], &close_tag).map_or(html.len(), |offset| at + offset);
            if tag.name == "script" {
                scripts.push(Script {
                    tag: tag.clone(),
                    body: html[at..close].to_string(),
                });
            }
            at = close;
        }
        tags.push(tag);
    }
    (tags, scripts)
}

/// Parse a start tag whose name begins at `start`; returns it and the index
/// just past its closing `>`.
fn parse_start_tag(html: &str, start: usize) -> (Tag, usize) {
    let bytes = html.as_bytes();
    let is_name_end = |b: u8| b.is_ascii_whitespace() || b == b'/' || b == b'>';
    let mut i = start;
    while i < bytes.len() && !is_name_end(bytes[i]) {
        i += 1;
    }
    let name = html[start..i].to_ascii_lowercase();
    let mut attrs = BTreeMap::new();
    loop {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'>' {
            i += 1;
            break;
        }
        let name_start = i;
        while i < bytes.len() && !is_name_end(bytes[i]) && bytes[i] != b'=' {
            i += 1;
        }
        let attr_name = html[name_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            match bytes.get(i) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let value_start = i + 1;
                    let value_end = html[value_start..]
                        .find(quote as char)
                        .map_or(html.len(), |offset| value_start + offset);
                    i = (value_end + 1).min(html.len());
                    Some(html[value_start..value_end].to_string())
                }
                _ => {
                    let value_start = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    Some(html[value_start..i].to_string())
                }
            }
        } else {
            None
        };
        attrs.insert(attr_name, value);
    }
    (Tag { name, attrs }, i)
}

/// Byte offset of the first ASCII-case-insensitive match of `needle`.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// How many times `<script` opens, counted the dumbest possible way.
pub fn raw_script_open_count(html: &str) -> usize {
    html.to_ascii_lowercase().matches("<script").count()
}

/// The source list of one CSP directive, e.g. `script-src`.
pub fn directive<'a>(csp: &'a str, name: &str) -> Vec<&'a str> {
    csp.split(';')
        .map(str::trim)
        .find_map(|part| {
            let mut words = part.split_ascii_whitespace();
            (words.next() == Some(name)).then(|| words.collect())
        })
        .unwrap_or_default()
}

/// Assert that `csp` admits exactly the inline scripts `html` (a served SPA
/// shell) carries, and nothing a browser would refuse.
///
/// * `require_real`: the shell must be a SvelteKit build (the Nix checks set
///   `LEDGELINE_REQUIRE_REAL_SPA`); otherwise a placeholder only prints a note.
/// * `sha256_source`: an inline script's CSP source, quotes included
///   (`'sha256-…'`).
/// * `how_to_build`: what to run for a real shell, for the note.
pub fn assert_shell_csp(
    csp: &str,
    html: &str,
    require_real: bool,
    sha256_source: impl Fn(&str) -> String,
    how_to_build: &str,
) {
    let is_real = html.contains(SVELTEKIT_MARKER);
    if require_real {
        assert!(
            is_real,
            "LEDGELINE_REQUIRE_REAL_SPA is set but the embedded shell is not a SvelteKit build \
             (no `{SVELTEKIT_MARKER}`), so this run proves nothing about the real UI. \
             Served body:\n{html}"
        );
    } else if !is_real {
        eprintln!(
            "NOTE: the embedded SPA is build.rs's placeholder, so this checks the placeholder \
             shell only. {how_to_build}"
        );
    }

    let (tags, scripts) = scan(html);
    let inline = scripts
        .iter()
        .filter(|script| script.tag.attr("src").is_none())
        .collect::<Vec<_>>();
    let external = scripts.len() - inline.len();

    // Our scanner saw every `<script` there is — no tag hid from it.
    assert_eq!(
        raw_script_open_count(html),
        scripts.len(),
        "the number of `<script` openings disagrees with the parsed script elements"
    );

    let script_src = directive(csp, "script-src");
    assert!(
        script_src.contains(&"'self'"),
        "script-src must admit same-origin bundles: {csp}"
    );
    assert!(
        !script_src.contains(&"'unsafe-inline'"),
        "script-src must never fall back to 'unsafe-inline': {csp}"
    );
    let hashed = script_src
        .iter()
        .filter(|source| source.starts_with("'sha256-"))
        .copied()
        .collect::<Vec<_>>();

    // Every `<script` is either covered by a hash or loads from `src`.
    assert_eq!(
        hashed.len() + external,
        raw_script_open_count(html),
        "CSP hashes ({}) + external scripts ({external}) must account for every <script>: {csp}",
        hashed.len()
    );
    // ...and each hash is the right one.
    for script in &inline {
        let expected = sha256_source(&script.body);
        assert!(
            hashed.contains(&expected.as_str()),
            "the CSP lacks {expected} for an inline script it serves, so the browser will \
             refuse it:\n{}\nCSP: {csp}",
            script.body
        );
    }

    // Import maps are governed by script-src too, and SvelteKit does not emit
    // one today; if it starts to, this is where to find out.
    assert!(
        scripts.iter().all(|script| !script
            .tag
            .attr("type")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("importmap"))),
        "the shell carries an import map; hash coverage for it is untested"
    );

    // Inline event handlers (`onload=`, `onclick=`, …) are NOT covered by a
    // script hash without 'unsafe-hashes', so any one of them is a silent break.
    let handlers = tags
        .iter()
        .flat_map(|tag| {
            tag.attrs
                .keys()
                .filter(|name| name.len() > 2 && name.starts_with("on"))
                .map(move |name| format!("<{} {name}=…>", tag.name))
        })
        .collect::<Vec<_>>();
    assert!(
        handlers.is_empty(),
        "inline event handlers are refused by this CSP: {handlers:?}"
    );

    if is_real {
        assert!(
            !inline.is_empty(),
            "a SvelteKit shell boots from an inline script; finding none means this scan is broken"
        );
    }
}
