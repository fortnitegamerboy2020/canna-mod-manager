//! Bounded, deterministic navigation aids for the staff review workspace.
//!
//! This reads report values only. It neither changes scanner findings or review
//! decisions nor accesses archive paths. All source outline matches are hints.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const MAX_FILES: usize = 800;
const MAX_FINDINGS: usize = 2_001;
const MAX_OBSERVATIONS: usize = 2_000;
const MAX_SYMBOLS: usize = 64;
const MAX_TOTAL_SYMBOLS: usize = 4_096;
const MAX_SOURCE_BYTES: usize = 128 * 1024;
const MAX_TOTAL_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_SUGGESTIONS: usize = 20;
const MAX_LOCATIONS: usize = 12;
const OUTLINE_NOTE: &str = "Heuristic outline of retained source only; names may be missed or misidentified. A callback name does not prove it runs.";
const NOTE: &str = "Advisory review navigation, not additional scanner findings or an approval. Static analysis and reconstructed source cannot prove safety. Review decisions remain bound to this archive hash.";

fn array<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value[field].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn label(value: &Value, limit: usize) -> String {
    value.as_str().unwrap_or("").chars().take(limit).collect()
}
fn path(value: &Value) -> Option<String> {
    let name = value.as_str()?;
    (!name.is_empty() && name.len() <= 2_048 && !name.chars().any(char::is_control))
        .then(|| name.to_owned())
}
fn line(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .filter(|number| (1..=10_000_000).contains(number))
}
fn location(value: &Value) -> Option<Value> {
    let file = path(&value["file"])?;
    Some(json!({"file":file,"line":line(&value["line"]),"finding_id":label(&value["id"],256)}))
}
fn locations(values: &[&Value]) -> Vec<Value> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for finding in values {
        let candidates = std::iter::once(*finding).chain(array(finding, "locations").iter());
        for item in candidates.take(256) {
            if let Some(mut loc) = location(item) {
                // Grouped coverage locations belong to the parent finding.
                loc["finding_id"] = json!(label(&finding["id"], 256));
                if seen.insert((loc["file"].to_string(), loc["line"].to_string())) {
                    result.push(loc);
                }
                if result.len() == MAX_LOCATIONS {
                    return result;
                }
            }
        }
    }
    result
}
fn suggestion(
    id: &str,
    priority: &str,
    title: &str,
    why: &str,
    checks: &[&str],
    findings: &[&Value],
) -> Value {
    json!({"id":id,"category":id,"priority":priority,"title":title,"why":why,
        "checks":checks,"locations":locations(findings),"finding_ids":findings.iter().take(32).map(|f|label(&f["id"],256)).filter(|id|!id.is_empty()).collect::<Vec<_>>(),"advisory":true})
}

#[derive(Default)]
struct File {
    name: String,
    kind: String,
    size: Option<u64>,
    preview: bool,
    lines: usize,
    findings: usize,
    unresolved: usize,
    observations: usize,
    origin: Option<String>,
    origins: Vec<String>,
    language: String,
    decompiler: String,
    symbols: Vec<Value>,
    entry_points: Vec<Value>,
    outline_limited: bool,
}
fn add_file<'a>(
    files: &'a mut BTreeMap<String, File>,
    name: String,
    limited: &mut bool,
) -> Option<&'a mut File> {
    if !files.contains_key(&name) && files.len() >= MAX_FILES {
        *limited = true;
        return None;
    }
    Some(files.entry(name.clone()).or_insert_with(|| File {
        name,
        ..File::default()
    }))
}

/// A small ASCII token outline, intentionally not an AST or a call graph.
/// Strings and comments are skipped; byte/line/token limits bound old reports.
fn outline(text: &str, suffix: &str, remaining: usize) -> (Vec<Value>, Vec<Value>, bool) {
    #[derive(Clone)]
    struct Token {
        text: String,
        line: usize,
    }
    let bytes = text.as_bytes();
    let mut tokens = Vec::<Token>::new();
    let (mut index, mut number, mut limited) = (0, 1, false);
    let hash_comments = matches!(suffix, "py" | "sh" | "ps1" | "toml" | "yaml" | "yml");
    while index < bytes.len() {
        if tokens.len() >= 16_384 || number > 8_192 {
            limited = true;
            break;
        }
        let current = bytes[index];
        if current == b'\n' {
            number += 1;
            index += 1;
            continue;
        }
        if current.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let slash_line = !hash_comments && bytes.get(index..index + 2) == Some(b"//");
        let lua_line = suffix == "lua" && bytes.get(index..index + 2) == Some(b"--");
        if slash_line || lua_line || (hash_comments && current == b'#') {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes.get(index..index + 2) == Some(b"/*") {
            index += 2;
            while index < bytes.len() {
                if bytes.get(index..index + 2) == Some(b"*/") {
                    index += 2;
                    break;
                }
                if bytes[index] == b'\n' {
                    number += 1;
                }
                index += 1;
            }
            continue;
        }
        if matches!(current, b'\'' | b'"' | b'`') {
            // Rust lifetimes are tokens, not unterminated character literals.
            if suffix == "rs"
                && current == b'\''
                && bytes.get(index + 1).is_some_and(u8::is_ascii_alphabetic)
            {
                let mut end = index + 2;
                while bytes
                    .get(end)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                {
                    end += 1;
                }
                if bytes.get(end) != Some(&b'\'') {
                    index = end;
                    continue;
                }
            }
            let quote = current;
            let triple = bytes
                .get(index..index + 3)
                .is_some_and(|part| part.iter().all(|b| *b == quote));
            let verbatim = suffix == "cs" && index > 0 && bytes[index - 1] == b'@';
            index += if triple { 3 } else { 1 };
            while index < bytes.len() {
                if bytes[index] == b'\n' {
                    number += 1;
                }
                if triple
                    && bytes
                        .get(index..index + 3)
                        .is_some_and(|part| part.iter().all(|b| *b == quote))
                {
                    index += 3;
                    break;
                }
                if !triple && bytes[index] == quote {
                    if verbatim && bytes.get(index + 1) == Some(&quote) {
                        index += 2;
                        continue;
                    }
                    index += 1;
                    break;
                }
                if bytes[index] == b'\\' && !verbatim {
                    index = (index + 2).min(bytes.len());
                } else {
                    index += 1;
                }
            }
            continue;
        }
        let start = index;
        if current.is_ascii_alphabetic() || current == b'_' {
            index += 1;
            while bytes
                .get(index)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                index += 1;
            }
            if index - start <= 120 {
                tokens.push(Token {
                    text: text[start..index].to_owned(),
                    line: number,
                });
            }
        } else {
            if current.is_ascii() {
                tokens.push(Token {
                    text: char::from(current).to_string(),
                    line: number,
                });
            }
            index += 1;
        }
    }
    let mut symbols = Vec::new();
    let mut entry_points = Vec::new();
    let mut seen = BTreeSet::new();
    for (position, token) in tokens.iter().enumerate() {
        let previous = position
            .checked_sub(1)
            .and_then(|i| tokens.get(i))
            .map(|v| v.text.as_str())
            .unwrap_or("");
        let next = tokens
            .get(position + 1)
            .map(|v| v.text.as_str())
            .unwrap_or("");
        let named = token
            .text
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_');
        if !named {
            continue;
        }
        let mut kind = match previous {
            "class" => "class",
            "interface" => "interface",
            "struct" => "struct",
            "enum" => "enum",
            "trait" => "trait",
            "fn" | "def" | "function" => "function",
            _ => "",
        };
        if kind.is_empty()
            && matches!(
                suffix,
                "cs" | "java" | "cpp" | "c" | "h" | "hpp" | "js" | "ts"
            )
            && next == "("
            && !matches!(
                token.text.as_str(),
                "if" | "for"
                    | "while"
                    | "switch"
                    | "catch"
                    | "using"
                    | "lock"
                    | "sizeof"
                    | "typeof"
                    | "nameof"
            )
            && !matches!(previous, "." | "new" | "return" | "=" | "," | "(")
        {
            let mut depth = 0usize;
            for (end, item) in tokens.iter().enumerate().skip(position + 1).take(192) {
                if item.text == "(" {
                    depth += 1;
                }
                if item.text == ")" {
                    depth = depth.saturating_sub(1);
                }
                if depth == 0 {
                    let after = tokens.get(end + 1).map(|v| v.text.as_str()).unwrap_or("");
                    if matches!(after, "{" | "=" | ":") {
                        kind = "method";
                    }
                    break;
                }
            }
        }
        if kind.is_empty() || !seen.insert((token.text.clone(), token.line)) {
            continue;
        }
        if symbols.len() >= MAX_SYMBOLS.min(remaining) {
            limited = true;
            break;
        }
        let symbol = json!({"name":token.text,"kind":kind,"line":token.line,"heuristic":true});
        symbols.push(symbol.clone());
        if matches!(kind, "method" | "function")
            && matches!(
                token.text.as_str(),
                "Awake"
                    | "Start"
                    | "OnEnable"
                    | "OnDisable"
                    | "OnDestroy"
                    | "Main"
                    | "Initialize"
                    | "Load"
                    | "OnLoad"
                    | "onLoad"
                    | "onEnable"
                    | "onDisable"
                    | "Update"
                    | "FixedUpdate"
                    | "LateUpdate"
                    | "init"
                    | "main"
            )
        {
            entry_points.push(symbol);
        }
    }
    (symbols, entry_points, limited)
}

fn decompilations(report: &Value) -> Vec<Value> {
    array(report, "decompilations").iter().take(32).map(|d| {
        let scope = if d["scope"] == "project" {"project"} else {"binary"};
        let mut item = json!({"input":path(&d["input"]),"scope":scope,"language":label(&d["language"],40),"tool":label(&d["tool"],80),"status":label(&d["status"],40),"diagnostic":label(&d["diagnostic"],800),"exit_code":d["exit_code"].as_i64(),"inputs":array(d,"inputs").iter().take(64).filter_map(path).collect::<Vec<_>>(),"generated_files":array(d,"generated_files").iter().take(100).filter_map(path).collect::<Vec<_>>(),"limitations":array(d,"limitations").iter().take(12).map(|v|label(v,300)).collect::<Vec<_>>(),"mapping_note":label(&d["mapping_note"],600),"overview_limited":array(d,"inputs").len()>64 || array(d,"generated_files").len()>100 || array(d,"limitations").len()>12});
        for key in ["duration_ms","generated_count","generated_bytes","preview_count","preview_omitted_count","scanned_count","scan_omitted_count"] { item[key] = json!(d[key].as_u64()); }
        item
    }).collect()
}

fn declared_dependencies(file: &Value, text: &str) -> Vec<Value> {
    let Some(name) = path(&file["name"]) else {
        return Vec::new();
    };
    let lower = name.to_ascii_lowercase();
    let mut result = Vec::new();
    if lower.ends_with("/manifest.json")
        || lower.ends_with("/package.json")
        || lower == "manifest.json"
        || lower == "package.json"
    {
        if let Ok(metadata) = serde_json::from_str::<Value>(text) {
            let deps = &metadata["dependencies"];
            if let Some(values) = deps.as_array() {
                for value in values.iter().take(60) {
                    if value.is_string() {
                        result.push(json!({"name":label(value,240),"file":name}));
                    }
                }
            } else if let Some(values) = deps.as_object() {
                for (key, value) in values.iter().take(60) {
                    let version = label(value, 80);
                    result.push(json!({"name":format!("{} {version}",key.chars().take(160).collect::<String>()),"file":name}));
                }
            }
        }
    } else if lower.ends_with(".csproj") {
        for text_line in text.lines().take(8_192) {
            if !text_line.contains("<Reference ") && !text_line.contains("<PackageReference ") {
                continue;
            }
            if let Some(rest) = text_line.split_once("Include=\"").map(|(_, rest)| rest)
                && let Some((dependency, _)) = rest.split_once('"')
            {
                result.push(
                    json!({"name":dependency.chars().take(240).collect::<String>(),"file":name}),
                );
            }
            if result.len() >= 60 {
                break;
            }
        }
    }
    result
}

fn component_summary(report: &Value, files: &BTreeMap<String, File>) -> Vec<Value> {
    let mut counts = BTreeMap::<String, BTreeMap<String, (usize, usize)>>::new();
    for finding in array(report, "findings").iter().take(MAX_FINDINGS) {
        let Some(name) = path(&finding["file"]) else {
            continue;
        };
        let origin = files
            .get(&name)
            .and_then(|file| file.origin.clone())
            .unwrap_or(name);
        let rule = label(&finding["rule"], 80);
        let count = counts.entry(origin).or_default().entry(rule).or_default();
        count.0 += 1;
        count.1 += usize::from(finding["accepted"] != true);
    }
    array(report, "decompilations").iter().take(32).filter_map(|decomp| {
        let input = path(&decomp["input"])?;
        let rules = counts.get(&input).cloned().unwrap_or_default();
        let placement = if input.contains("/patchers/") { "Patcher directory" }
            else if input.contains("/plugins/") { "Plugin directory" } else { "Archive component" };
        let metadata = array(report,"binary_metadata").iter().take(16).find(|item|item["input"]==input);
        let metadata = metadata.map(|item|json!({"status":label(&item["status"],40),
            "payload_name_hints":array(item,"payload_name_hints").iter().take(64).map(|name|label(name,256)).filter(|name|!name.is_empty()).collect::<Vec<_>>(),
            "heap_limit_reached":item["heap_limit_reached"]==true,"name_limit_reached":item["name_limit_reached"]==true,
            "note":"Untrusted CLR string-heap names only, not verified resource declarations or use. Patch-like names can explain what to inspect; they do not clear packing or entropy findings."}));
        Some(json!({"input":input,"placement_hint":placement,"status":label(&decomp["status"],40),"metadata":metadata,
            "finding_count":rules.values().map(|(count,_)|count).sum::<usize>(),
            "unresolved_count":rules.values().map(|(_,count)|count).sum::<usize>(),
            "rules":rules.into_iter().map(|(rule,(count,unresolved))|json!({"rule":rule,"count":count,"unresolved_count":unresolved})).collect::<Vec<_>>(),
            "note":"Direct file locations only; grouped cross-file limitations remain in Findings. Placement and component names do not verify upstream identity, trust or caller reachability."}))
    }).collect()
}

/// Regenerated at staff-only GET time, including for old stored reports.
pub fn overview(report: &Value) -> Value {
    let raw_findings = array(report, "findings");
    let findings: Vec<_> = raw_findings
        .iter()
        .take(MAX_FINDINGS)
        .filter(|f| f.is_object())
        .collect();
    let raw_files = array(report, "files");
    let raw_inventory = array(report, "inventory");
    let raw_observations = array(report, "observations");
    let decomp = decompilations(report);
    let mut files = BTreeMap::<String, File>::new();
    let (mut files_limited, mut text_limited, mut symbols_limited) = (false, false, false);
    let (mut text_bytes, mut symbol_count) = (0, 0);
    let mut dependencies = Vec::<Value>::new();
    let mut rewriting_locations = Vec::<Value>::new();
    // Previews and finding locations take precedence over asset inventory.
    for file in raw_files.iter().take(MAX_FILES) {
        let Some(name) = path(&file["name"]) else {
            files_limited = true;
            continue;
        };
        let Some(item) = add_file(&mut files, name.clone(), &mut files_limited) else {
            continue;
        };
        item.kind = label(&file["kind"], 40);
        item.size = file["byte_size"].as_u64().or_else(|| file["size"].as_u64());
        item.origin = path(&file["origin"]);
        item.origins = array(file, "origins")
            .iter()
            .take(64)
            .filter_map(path)
            .collect();
        item.language = label(&file["language"], 40);
        item.decompiler = label(&file["decompiler"], 80);
        if let Some(source) = file["text"].as_str() {
            item.preview = true;
            // Counting only retained previews never claims full source coverage.
            item.lines =
                source.bytes().filter(|b| *b == b'\n').count() + usize::from(!source.is_empty());
            let allowance = MAX_SOURCE_BYTES.min(MAX_TOTAL_SOURCE_BYTES.saturating_sub(text_bytes));
            let mut end = source.len().min(allowance);
            while !source.is_char_boundary(end) {
                end -= 1;
            }
            let text = &source[..end];
            text_bytes += end;
            let truncated = source.len() > end;
            text_limited |= truncated;
            let suffix = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            // Textual context is explicitly advisory, not a call graph or a trust
            // exception. Documents quoting these names cannot trigger this hint.
            if suffix == "cs"
                && rewriting_locations.len() < MAX_LOCATIONS
                && (text.contains("Mono.Cecil") || text.contains("ModuleDefinition.ReadModule"))
                && (text.contains("File.WriteAllBytes")
                    || text.contains("File.Move")
                    || text.contains(".Write("))
            {
                let number = text
                    .lines()
                    .position(|line| {
                        line.contains("ModuleDefinition.ReadModule")
                            || line.contains("File.WriteAllBytes")
                    })
                    .map(|i| i + 1);
                rewriting_locations.push(json!({"file":name,"line":number}));
            }
            // Outlines for executable source formats only; docs/assets can quote APIs.
            if matches!(
                suffix.as_str(),
                "cs" | "java" | "rs" | "py" | "js" | "ts" | "cpp" | "c" | "h" | "hpp" | "lua"
            ) {
                let (symbols, entries, limited) = outline(
                    text,
                    &suffix,
                    MAX_TOTAL_SYMBOLS.saturating_sub(symbol_count),
                );
                symbol_count += symbols.len();
                item.symbols = symbols;
                item.entry_points = entries;
                item.outline_limited = limited || truncated;
                symbols_limited |= limited;
            }
            if dependencies.len() < 60 {
                dependencies.extend(
                    declared_dependencies(file, text)
                        .into_iter()
                        .take(60 - dependencies.len()),
                );
            }
        }
    }
    for finding in &findings {
        let candidates = std::iter::once(*finding).chain(array(finding, "locations").iter());
        let mut seen_files = BTreeSet::new();
        for item in candidates.take(256) {
            if let Some(name) = path(&item["file"]) {
                if !seen_files.insert(name.clone()) {
                    continue;
                }
                if let Some(file) = add_file(&mut files, name, &mut files_limited) {
                    file.findings += 1;
                    if finding["accepted"] != true {
                        file.unresolved += 1;
                    }
                }
            }
        }
    }
    for item in raw_inventory.iter().take(6_000) {
        let Some(name) = path(&item["name"]) else {
            files_limited = true;
            continue;
        };
        if let Some(file) = add_file(&mut files, name, &mut files_limited) {
            if file.kind.is_empty() {
                file.kind = label(&item["kind"], 40);
            }
            if file.size.is_none() {
                file.size = item["size"].as_u64();
            }
        }
    }
    for item in raw_observations.iter().take(MAX_OBSERVATIONS) {
        if let Some(name) = path(&item["file"])
            && let Some(file) = files.get_mut(&name)
        {
            file.observations += 1;
        }
    }
    let mut groups = BTreeMap::<String, Vec<&Value>>::new();
    for finding in &findings {
        let rule = label(&finding["rule"], 80);
        groups
            .entry(if rule.is_empty() {
                "unspecified".into()
            } else {
                rule
            })
            .or_default()
            .push(finding);
    }
    let reported_complete = report["coverage_complete"].as_bool();
    let coverage_findings = groups.get("coverage").cloned().unwrap_or_default();
    let incomplete_decomp = decomp.iter().any(|d| d["status"] != "complete");
    let incomplete_engine = report["engines"].as_object().is_some_and(|engines| {
        engines.values().take(12).any(|engine| {
            matches!(
                engine["status"].as_str(),
                Some("error" | "failed" | "incomplete" | "unavailable" | "limited")
            )
        })
    });
    let status = label(&report["status"], 40);
    let coverage_state = if !coverage_findings.is_empty()
        || reported_complete == Some(false)
        || incomplete_decomp
        || incomplete_engine
        || status == "failed"
    {
        "incomplete"
    } else if reported_complete == Some(true) && status == "complete" {
        "complete"
    } else {
        "unknown"
    };
    let mut suggestions = Vec::new();
    if status != "complete" {
        suggestions.push(suggestion("analysis-status","high","Finish or retry analysis before a review decision","This report is pending, running or failed. Retained details do not establish that the current archive was fully inspected.",&["Check the worker status and bounded diagnostic output.","Retry failed analysis; verify the displayed archive SHA256 before reviewing."],&[]));
    }
    if coverage_state != "complete" {
        suggestions.push(suggestion("coverage","high","Check what analysis could not inspect",if coverage_state == "incomplete" {"Coverage limits or unavailable reconstruction are recorded. Missing preview content can still affect runtime behavior."} else {"This older or incomplete report has no explicit complete coverage result."},&["Open each coverage location and decompiler limitation, including grouped occurrences.","Compare inventory entries with retained previews; inspect unsupported or omitted code through an appropriate separate review.","A complete static pass still does not establish safe runtime behavior."],&coverage_findings));
    }
    if !rewriting_locations.is_empty() && groups.contains_key("filesystem") {
        let mut hint = suggestion(
            "compatibility-rewriting",
            "normal",
            "Review assembly rewriting, backups and restoration",
            "Retained C# text mentions assembly rewriting alongside flagged file operations. Compatibility repair can explain this behavior, and it can change DLLs the loader will execute. The text match does not establish a reachable call or safe path.",
            &[
                "Trace game, managed, plugin and cache directory values from their callers; verify the final paths and traversal handling.",
                "Inspect before/after hashes and the bounds on embedded or downloaded patch data; a compressed resource alone does not settle packing evidence.",
                "Check backup and restoration index filenames, failures and cleanup. Path.Combine alone does not keep an externally supplied filename inside a directory.",
                "Follow rewritten bytes into writes and loader calls; compare the behavior with the stated compatibility purpose.",
            ],
            groups.get("filesystem").map(Vec::as_slice).unwrap_or(&[]),
        );
        hint["locations"] = json!(rewriting_locations);
        suggestions.push(hint);
    }
    for (rule, title, why, checks) in [
        (
            "signature",
            "Inspect the detected signature",
            "The scanner recorded a signature match; verify the exact engine result and affected archive content.",
            vec![
                "Check the signature name, exact file and engine status.",
                "Use the existing review policy and evidence; these hints do not override quarantine or approval requirements.",
            ],
        ),
        (
            "commands",
            "Trace process and shell execution",
            "Process-launch API matches need call-site context and argument provenance.",
            vec![
                "Trace executable names, arguments and environment values back to their source.",
                "Check whether downloads, user input or decoded strings can change the command.",
            ],
        ),
        (
            "privileges",
            "Check privilege and persistence behavior",
            "These matches can alter privileges, startup or system state.",
            vec![
                "Identify the exact target, requested privilege and lifecycle trigger.",
                "Confirm why the mod needs this behavior and how changes are removed.",
            ],
        ),
        (
            "sensitive-files",
            "Trace sensitive file references",
            "Sensitive path strings can be references or real accesses; follow their uses before deciding.",
            vec![
                "Trace references to browser, credential, key or account stores.",
                "Check whether any read values are written, transmitted or used in commands.",
            ],
        ),
        (
            "filesystem",
            "Follow file paths and operations",
            "File access alone does not establish malicious behavior; runtime targets and downstream uses matter.",
            vec![
                "Trace literal and computed paths, configuration values and working directories to their runtime targets.",
                "Distinguish reads, writes, moves and deletes; check permissions, traversal and filename handling.",
                "Check whether written data is later loaded, executed or transmitted; an extension alone does not decide this.",
            ],
        ),
        (
            "network",
            "Trace requests and received data",
            "Network matches indicate APIs requiring context, not a verified destination or payload.",
            vec![
                "Trace URLs, hosts, methods, headers and payloads through callers and configuration.",
                "A bundled HTTP client wrapper is not proof that the mod calls it. Find reachable callers and their arguments.",
                "Check downloads, certificate handling and whether received data reaches file writes or dynamic loading.",
            ],
        ),
        (
            "identity",
            "Check collection and use of account or device data",
            "Identity API matches require context about what data is collected and where it goes.",
            vec![
                "Identify fields read and the reason the mod needs them.",
                "Follow collected values into logging, network requests and stored files.",
            ],
        ),
        (
            "native",
            "Trace native and memory calls",
            "Native interfaces can move behavior outside the retained managed source.",
            vec![
                "Identify imported libraries, entry points, arguments and buffers.",
                "Inspect relevant native payloads and check any unresolved reconstruction limits.",
            ],
        ),
        (
            "dynamic",
            "Separate reflection, code loading and decoded data",
            "Reflection and default-value construction can support hooks or serializers. Assembly loading, evaluation and decoded executable data require a different trace; the matched API alone does not distinguish their purpose.",
            vec![
                "Distinguish reflective invocation or value-type construction from assembly loading and evaluation; identify the exact operation.",
                "Trace assembly, script and decoded-data origins before the load or evaluation call.",
                "Check whether network or user-controlled values can supply executable content.",
            ],
        ),
        (
            "packer-signature",
            "Inspect packing evidence and reconstruction",
            "The scanner recorded packing evidence; it does not by itself establish malware.",
            vec![
                "Compare engine results with binary layout evidence and source reconstruction limits.",
                "Identify which behavior remains hidden or unsupported before reviewing.",
            ],
        ),
        (
            "packer-marker",
            "Inspect packing evidence and reconstruction",
            "The scanner recorded packing evidence; it does not by itself establish malware.",
            vec![
                "Compare markers with binary layout evidence and source reconstruction limits.",
                "Identify which behavior remains hidden or unsupported before reviewing.",
            ],
        ),
        (
            "packing-review",
            "Verify the context of packing indicators",
            "Names, strings and entropy can have legitimate explanations; this suggestion adds no packing determination.",
            vec![
                "Inspect the matched section, marker or entropy context and compare actual tool output.",
                "Separate compressed assets or library text from executable reconstruction gaps.",
            ],
        ),
    ] {
        if let Some(values) = groups.get(rule) {
            let priority = if matches!(
                rule,
                "signature" | "commands" | "privileges" | "sensitive-files"
            ) && values.iter().any(|f| f["accepted"] != true)
            {
                "high"
            } else {
                "normal"
            };
            suggestions.push(suggestion(rule, priority, title, why, &checks, values));
        }
    }
    let entry_locations: Vec<_> = files
        .values()
        .flat_map(|file| {
            file.entry_points.iter().map(
                move |entry| json!({"file":file.name,"line":entry["line"],"symbol":entry["name"]}),
            )
        })
        .take(MAX_LOCATIONS)
        .collect();
    if !entry_locations.is_empty() {
        let mut s = suggestion(
            "entry-points",
            "normal",
            "Start with initialization and lifecycle candidates",
            "Heuristic callback names provide starting points; they are not proof of reachability.",
            &[
                "Inspect plugin initialization, lifecycle methods and registered patches or event handlers.",
                "Follow calls into the flagged APIs, including delayed tasks and exception paths.",
                "For multiplayer behavior, inspect synchronization and authority checks; static names do not verify multiplayer compatibility.",
            ],
            &[],
        );
        s["locations"] = json!(entry_locations);
        suggestions.push(s);
    }
    if !dependencies.is_empty() {
        let mut s = suggestion(
            "dependencies",
            "normal",
            "Check declared dependencies and versions",
            "These entries come from untrusted retained package metadata, not dependency resolution or compatibility testing.",
            &[
                "Compare declared names and versions with the archive contents and actual assembly references.",
                "Review dependent code and game version requirements; metadata alone does not establish compatibility.",
            ],
            &[],
        );
        s["locations"] = json!(
            dependencies
                .iter()
                .take(MAX_LOCATIONS)
                .map(|d| json!({"file":d["file"],"line":null}))
                .collect::<Vec<_>>()
        );
        suggestions.push(s);
    }
    let url_refs: Vec<_> = raw_observations
        .iter()
        .take(MAX_OBSERVATIONS)
        .filter(|o| o["rule"] == "url-reference")
        .collect();
    if !url_refs.is_empty() && !groups.contains_key("network") {
        suggestions.push(suggestion("references","normal","Check how referenced URLs are used","URL strings are observations and are not themselves network requests.",&["Follow the reference to callers, configuration and dynamic APIs if relevant.","Preserve the distinction between a literal reference and demonstrated API behavior."],&url_refs));
    }
    if suggestions.is_empty() {
        suggestions.push(suggestion("manual-review","normal","Inspect the retained source and archive structure","No navigation matches were generated. This says nothing about unobserved behavior or safety.",&["Check plugin entry points, package metadata and embedded payloads.","Review intended behavior and dependencies alongside the report's analysis limitations."],&[]));
    }
    suggestions.sort_by_key(|s| if s["priority"] == "high" { 0 } else { 1 });
    let suggestions_limited = suggestions.len() > MAX_SUGGESTIONS;
    suggestions.truncate(MAX_SUGGESTIONS);
    let mut engines = Vec::new();
    if let Some(values) = report["engines"].as_object() {
        for (name, engine) in values.iter().take(12) {
            engines.push(json!({"name":name.chars().take(80).collect::<String>(),"status":label(&engine["status"],40)}));
        }
    }
    let capability_names = [
        ("commands", "Process execution"),
        ("filesystem", "File operations"),
        ("network", "Network APIs"),
        ("identity", "Device/account data"),
        ("sensitive-files", "Sensitive path references"),
        ("privileges", "Privileges/persistence"),
        ("native", "Native/memory APIs"),
        ("dynamic", "Reflection / code loading / decoding"),
    ];
    let capabilities: Vec<_> = capability_names.iter().filter_map(|(rule,label)|groups.get(*rule).map(|values|json!({"rule":rule,"label":label,"count":values.len(),"unresolved_count":values.iter().filter(|f|f["accepted"] != true).count()}))).collect();
    let file_rows: Vec<_> = files.values().map(|file| {
        let status = decomp.iter().find(|d|d["input"] == file.name).map(|d|d["status"].as_str().unwrap_or("unknown"));
        let reason = if file.preview {"Retained source preview"} else if let Some(state) = status {match state {"not-supported"=>"Source reconstruction is not supported for this binary","failed"=>"Source reconstruction failed","unavailable"=>"Decompiler unavailable","limited"|"incomplete"=>"Source reconstruction or preview was limited",_=>"Archive input; inspect linked reconstructed source and any preview limits"}} else {"No source preview retained; inventory metadata only"};
        json!({"name":file.name,"kind":file.kind,"size":file.size,"preview_available":file.preview,"preview_reason":reason,"lines":file.lines,"finding_count":file.findings,"unresolved_count":file.unresolved,"observation_count":file.observations,"origin":file.origin,"origins":file.origins,"language":file.language,"decompiler":file.decompiler,"symbols":file.symbols,"entry_points":file.entry_points,"outline_note":OUTLINE_NOTE,"outline_limited":file.outline_limited})
    }).collect();
    json!({"version":"canna-review-guide-1","advisory":true,"note":NOTE,"analysis_status":status,
        "coverage":{"state":coverage_state,"reported_complete":reported_complete},
        "counts":{"preview_files":raw_files.len(),"decompiled_files":raw_files.iter().filter(|f|f["kind"] == "decompiled").count(),"inventory_files":raw_inventory.len(),"findings":raw_findings.len(),"unresolved_findings":raw_findings.iter().filter(|f|f["accepted"] != true).count(),"observations":raw_observations.len(),"decompilations":array(report,"decompilations").len()},
        "engines":engines,"capabilities":capabilities,"components":component_summary(report,&files),"files":file_rows,"decompilations":decomp,"dependencies":dependencies,"suggestions":suggestions,
        "limits":{"truncated_files":files_limited || raw_files.len()>MAX_FILES || raw_inventory.len()>6_000,"truncated_symbols":symbols_limited,"truncated_text":text_limited,"truncated_suggestions":suggestions_limited,"truncated_findings":raw_findings.len()>MAX_FINDINGS,"truncated_observations":raw_observations.len()>MAX_OBSERVATIONS,"truncated_decompilations":array(report,"decompilations").len()>32,"source_bytes_inspected":text_bytes,"max_files":MAX_FILES,"max_symbols_per_file":MAX_SYMBOLS,"max_source_bytes":MAX_TOTAL_SOURCE_BYTES}})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patcher_context_is_advisory_and_keeps_findings_and_coverage() {
        let report = json!({"status":"complete","coverage_complete":false,
            "files":[{"name":"decompiled/archive/patchers/fix.dll/AutoFix.cs","origin":"archive/patchers/fix.dll","kind":"decompiled",
                "text":"using Mono.Cecil;\nvoid Run() { ModuleDefinition.ReadModule(path); File.WriteAllBytes(path, patch); }"}],
            "decompilations":[{"input":"archive/patchers/fix.dll","status":"complete"}],
            "findings":[{"id":"write","rule":"filesystem","file":"decompiled/archive/patchers/fix.dll/AutoFix.cs","line":2},
                {"id":"limits","rule":"coverage","locations":[{"file":"archive/other.dll"}]}]});
        let saved = report.clone();
        let result = overview(&report);
        let hint = result["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "compatibility-rewriting")
            .unwrap();
        assert_eq!(hint["advisory"], true);
        assert_eq!(hint["locations"][0]["line"], 2);
        assert_eq!(hint["finding_ids"], json!(["write"]));
        assert_eq!(result["coverage"]["state"], "incomplete");
        assert_eq!(result["components"][0]["finding_count"], 1);
        assert_eq!(result["components"][0]["rules"][0]["rule"], "filesystem");
        assert_eq!(
            result["components"][0]["placement_hint"],
            "Patcher directory"
        );
        assert_eq!(report, saved);
    }
    #[test]
    fn documentation_mentions_do_not_create_a_rewriting_hint() {
        let result = overview(
            &json!({"status":"complete","files":[{"name":"archive/README.md",
            "text":"using Mono.Cecil; ModuleDefinition.ReadModule(path); File.WriteAllBytes(path, patch);"}],
            "findings":[{"id":"write","rule":"filesystem","file":"archive/README.md"}]}),
        );
        assert!(
            !result["suggestions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == "compatibility-rewriting")
        );
    }
    #[test]
    fn component_summary_is_bounded_and_reflection_is_not_labelled_only_loading() {
        let result = overview(&json!({"status":"complete","coverage_complete":true,
            "decompilations":(0..100).map(|i|json!({"input":format!("archive/{i}.dll"),"status":"complete"})).collect::<Vec<_>>(),
            "binary_metadata":[{"input":"archive/0.dll","status":"complete","payload_name_hints":(0..100).map(|i|json!(format!("curated/{}.bsdf","x".repeat(i*20)))).collect::<Vec<_>>()}],
            "findings":[{"id":"reflection","rule":"dynamic","file":"archive/0.dll","accepted":true}]}));
        assert_eq!(result["components"].as_array().unwrap().len(), 32);
        assert_eq!(result["components"][0]["finding_count"], 1);
        assert_eq!(result["components"][0]["unresolved_count"], 0);
        assert_eq!(
            result["components"][0]["metadata"]["payload_name_hints"]
                .as_array()
                .unwrap()
                .len(),
            64
        );
        assert!(
            result["components"][0]["metadata"]["payload_name_hints"]
                .as_array()
                .unwrap()
                .iter()
                .all(|name| name.as_str().unwrap().len() <= 256)
        );
        assert_eq!(
            result["capabilities"][0]["label"],
            "Reflection / code loading / decoding"
        );
        assert_eq!(result["coverage"]["state"], "complete");
    }
    #[test]
    fn legacy_report_gets_advisory_navigation_without_inventing_coverage_or_decisions() {
        let report = json!({"status":"complete","files":[{"name":"decompiled/Plugin.cs","kind":"decompiled","text":"public class Plugin { void Awake() { File.ReadAllText(\"config.txt\"); } }"}],"findings":[{"id":"file-op","rule":"filesystem","file":"decompiled/Plugin.cs","line":1,"accepted":true}],"observations":[]});
        let original = report.clone();
        let result = overview(&report);
        assert_eq!(report, original);
        assert_eq!(result["advisory"], true);
        assert_eq!(result["coverage"]["state"], "unknown");
        assert_eq!(result["counts"]["unresolved_findings"], 0);
        assert_eq!(result["files"][0]["entry_points"][0]["name"], "Awake");
        assert!(
            array(&result, "suggestions")
                .iter()
                .any(|s| s["id"] == "filesystem")
        );
        assert!(result.get("accepted").is_none());
    }
    #[test]
    fn current_report_links_decompiler_limits_and_grouped_finding_locations() {
        let report = json!({"status":"complete","coverage_complete":true,"files":[{"name":"decompiled/a/Plugin.cs","kind":"decompiled","origin":"archive/a.dll","language":"csharp","decompiler":"ilspy","text":"class Plugin { public void Start() {} }"}],"inventory":[{"name":"archive/a.dll","size":200,"kind":"archive"},{"name":"archive/n.dll","size":80}],"decompilations":[{"input":"archive/a.dll","status":"complete","tool":"ilspy","generated_files":["decompiled/a/Plugin.cs"]},{"input":"archive/n.dll","status":"not-supported","tool":"none","limitations":["Native source not reconstructed"]}],"findings":[{"id":"coverage-group","rule":"coverage","locations":[{"file":"archive/n.dll","line":null},{"file":"decompiled/a/Plugin.cs","line":4}]}]});
        let result = overview(&report);
        assert_eq!(result["coverage"]["state"], "incomplete");
        let gap = array(&result, "suggestions")
            .iter()
            .find(|s| s["id"] == "coverage")
            .unwrap();
        assert_eq!(gap["locations"][0]["file"], "archive/n.dll");
        assert_eq!(gap["locations"][0]["finding_id"], "coverage-group");
        let native = array(&result, "files")
            .iter()
            .find(|f| f["name"] == "archive/n.dll")
            .unwrap();
        assert_eq!(native["preview_available"], false);
        assert!(
            native["preview_reason"]
                .as_str()
                .unwrap()
                .contains("not supported")
        );
        assert_eq!(native["finding_count"], 1);
    }
    #[test]
    fn failed_and_empty_reports_never_gain_a_safe_or_approved_status() {
        for report in [
            json!({}),
            json!({"status":"failed","files":[],"findings":[]}),
            json!({"status":"complete","coverage_complete":true,"files":[],"findings":[]}),
        ] {
            let result = overview(&report);
            assert!(!array(&result, "suggestions").is_empty());
            assert_eq!(result["advisory"], true);
            assert!(result.get("approved").is_none());
            assert!(result.get("safe").is_none());
            if report["status"] != "complete" {
                assert_eq!(result["suggestions"][0]["id"], "analysis-status");
            }
        }
    }
    #[test]
    fn outline_excludes_comments_literals_and_reference_documents() {
        let source = "// class Fake { void Awake(){} }\nclass Real { string s = @\"void Main(){}\";\npublic void Start() { File.ReadAllText(\"class Hidden {}\"); }\n/* void Update() {} */ }";
        let result = overview(
            &json!({"status":"complete","files":[{"name":"archive/Plugin.cs","text":source},{"name":"archive/README.md","text":"class Fake { void Awake() {} }"}]}),
        );
        let code = array(&result, "files")
            .iter()
            .find(|f| f["name"] == "archive/Plugin.cs")
            .unwrap();
        let names: Vec<_> = array(code, "symbols")
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["Real", "Start"]);
        assert_eq!(array(code, "entry_points").len(), 1);
        let docs = array(&result, "files")
            .iter()
            .find(|f| f["name"] == "archive/README.md")
            .unwrap();
        assert!(array(docs, "symbols").is_empty());
    }
    #[test]
    fn hostile_reports_are_bounded_and_paths_are_not_interpreted_as_host_files() {
        let source = "class A { void Awake(){} }\n".repeat(100_000);
        let report = json!({"status":"complete","files":[{"name":"archive/../../../../private/credentials.cs","text":source},{"name":"archive/Bad\nName.cs","text":"void Main(){}"}],"inventory":(0..900).map(|i|json!({"name":format!("archive/{i}.bin")})).collect::<Vec<_>>(),"findings":[{"id":"<img onerror=fetch('/')>","rule":"filesystem","file":"archive/../../../../private/credentials.cs","line":-1}],"review_overview":{"approved":true}});
        let result = overview(&report);
        assert!(array(&result, "files").len() <= MAX_FILES);
        assert_eq!(result["limits"]["truncated_files"], true);
        assert_eq!(result["limits"]["truncated_text"], true);
        assert!(
            result["limits"]["source_bytes_inspected"].as_u64().unwrap()
                <= MAX_TOTAL_SOURCE_BYTES as u64
        );
        assert!(
            array(&result, "files")
                .iter()
                .all(|f| f["symbols"].as_array().unwrap().len() <= MAX_SYMBOLS)
        );
        assert!(
            !array(&result, "files")
                .iter()
                .any(|f| f["name"] == "archive/Bad\nName.cs")
        );
        assert!(result.get("approved").is_none());
    }
    #[test]
    fn dependencies_are_declared_metadata_and_url_observations_stay_separate() {
        let report = json!({"status":"complete","coverage_complete":true,"files":[{"name":"archive/manifest.json","text":"{\"dependencies\":[\"Someone-UnboundLib-3.2.0\"]}"}],"findings":[],"observations":[{"id":"url","rule":"url-reference","file":"archive/Plugin.cs","line":4}]});
        let result = overview(&report);
        assert_eq!(
            result["dependencies"][0]["name"],
            "Someone-UnboundLib-3.2.0"
        );
        assert!(array(&result, "capabilities").is_empty());
        assert!(
            array(&result, "suggestions")
                .iter()
                .any(|s| s["id"] == "references")
        );
        assert!(
            array(&result, "suggestions")
                .iter()
                .any(|s| s["id"] == "dependencies")
        );
    }
    #[test]
    fn grouped_and_individual_occurrences_count_each_file_once_and_prioritize_unresolved() {
        let result = overview(
            &json!({"status":"complete","coverage_complete":true,"findings":[{"id":"command","rule":"commands","file":"archive/a.cs","line":2,"locations":[{"file":"archive/a.cs","line":3},{"file":"archive/b.cs","line":4}]},{"id":"network","rule":"network","file":"archive/a.cs","accepted":true}]}),
        );
        assert_eq!(result["files"][0]["finding_count"], 2);
        assert_eq!(result["files"][0]["unresolved_count"], 1);
        assert_eq!(result["suggestions"][0]["id"], "commands");
        assert_eq!(
            result["suggestions"][0]["locations"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
    #[test]
    fn engine_failures_and_project_reconstruction_mapping_remain_explicit() {
        let result = overview(
            &json!({"status":"complete","coverage_complete":true,"engines":{"clamav":{"status":"error"}},"files":[{"name":"decompiled/java-project/Plugin.java","text":"class Plugin { void onEnable() {} }","kind":"decompiled","origin":"archive/","origins":["archive/Plugin.class","archive/Helper.class"],"byte_size":43}],"decompilations":[{"input":"archive/","scope":"project","inputs":["archive/Plugin.class","archive/Helper.class"],"language":"java","tool":"cfr","status":"complete","generated_files":["decompiled/java-project/Plugin.java"],"mapping_note":"Whole-project outputs do not establish individual class-to-source mappings."}]}),
        );
        assert_eq!(result["coverage"]["state"], "incomplete");
        assert_eq!(result["files"][0]["origin"], "archive/");
        assert_eq!(result["files"][0]["origins"].as_array().unwrap().len(), 2);
        assert_eq!(result["files"][0]["size"], 43);
        assert_eq!(result["decompilations"][0]["scope"], "project");
        assert!(
            result["decompilations"][0]["mapping_note"]
                .as_str()
                .unwrap()
                .contains("do not establish")
        );
        assert_eq!(result["decompilations"][0]["input"], "archive/");
    }
}
