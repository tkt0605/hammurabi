//! # `.hb` ブロック構文 — `{ config, define | goal+settings, ... }`
//!
//! 行ベース構文と同一ファイル内で併用可能。最外殻の `{ ... }` を検出して別パスで解釈する。
//!
//! ## 想定する形（EBNF 風）
//! ```text
//! document   ::= '{' field ( ',' field )* '}'
//! field      ::= 'config' ':' config_obj
//!              | 'define' ':' define_obj
//!              | 'goal' ':' <識別子>
//!              | 'settings' ':' settings_array
//! define_obj ::= '{' ( 'context' ':' context_value )? 'goal' ':' id 'settings' ':' settings_array '...' '}'
//! context_value ::= string | braced_text | bracketed_text | dash_list
//! bracketed_text ::= '[' ... ']'   -- 並列箇条書き向け（行頭 `-` 可）
//! dash_list ::= ( 空 & 次行から `-` 続き ) | ( 同一行 `context: - 項目` & 任意で続く `-` 行 )
//! config_obj ::= '{' config_pair ( ','? config_pair )* '}' ','?
//! config_pair ::= 'agent' | 'model' | 'lang' | 'api_key' ':' <値（行末まで）>
//! settings_array ::= '[' settings_body ']'
//! settings_body ::= (require|ensure|invariant|forbid 行)*   -- 1 goal 分
//!                 | '{' settings_body '}'                  -- 同上を {} で囲む
//! ```
//!
//! 複数 goal は **`define: { goal, settings }` を繰り返す**か、トップレベルで `goal` の直後に
//! `settings` を置き、それを繰り返す（文書順で複数 goal が得られる）。
//!
//! カンマは `config` 内・トップレベルフィールドの区切りとして **省略可能**。
//!
//! コメントは行頭の `#`（行全体）および `//`（以降）。ブロック内の `key:` を持たない行は注釈として **無視** します。

use crate::codegen::TargetLang;
use crate::config::AgentKind;
use crate::lang::goal::ContractualGoal;

use super::{ItemKind, ParseError, ParsedGoal, ParsedItem, Span, ErrorSeverity};
use super::{parse_forbidden, parse_predicate};

/// ブロック内 `config` で指定されたメタ（ファイル全体へマージする）
#[derive(Debug, Default, Clone)]
pub(crate) struct BraceFileMeta {
    pub agent:   Option<AgentKind>,
    pub model:   Option<String>,
    pub lang:    Option<TargetLang>,
    pub api_key: Option<String>,
}

/// `//` より後ろ、および行頭 `#` 以降はブレイス走査から除外（コメント内の `{` `}` をブロックにしない）。
fn brace_scan_line_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        let ws_len = line.len() - trimmed.len();
        return &line[..ws_len];
    }
    if let Some(i) = line.find("//") {
        return &line[..i];
    }
    line
}

/// テキスト内の最外殻 `{` … `}` のバイト範囲（両端含む）。ネストは depth で追う。
pub(crate) fn find_outer_brace_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let mut out = Vec::new();
    let mut line_off = 0usize;

    for raw_line in text.lines() {
        let prefix = brace_scan_line_prefix(raw_line);
        for (bi, c) in prefix.char_indices() {
            let g = line_off + bi;
            match c {
                '{' => {
                    if depth == 0 {
                        start = Some(g);
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = start {
                            out.push((s, g));
                        }
                        start = None;
                    } else if depth < 0 {
                        depth = 0;
                    }
                }
                _ => {}
            }
        }
        line_off += raw_line.len();
        if line_off < text.len() && text.as_bytes()[line_off] == b'\n' {
            line_off += 1;
        }
    }
    out
}

/// ブロック範囲をスペース化（改行は維持）し、行パーサが中身を誤解釈しないようにする。
pub(crate) fn mask_brace_ranges(text: &str, ranges: &[(usize, usize)]) -> String {
    if ranges.is_empty() {
        return text.to_owned();
    }
    let mut sorted: Vec<_> = ranges.to_vec();
    sorted.sort_by_key(|r| r.0);
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for &(s, e) in &sorted {
        if s > last {
            out.push_str(&text[last..s]);
        }
        for c in text[s..=e].chars() {
            if c == '\n' {
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
        last = e + 1;
    }
    out.push_str(&text[last..]);
    out
}

fn line_of_byte(text: &str, byte_idx: usize) -> u32 {
    text[..byte_idx.min(text.len())].bytes().filter(|&x| x == b'\n').count() as u32
}

/// 最外殻ブロックをパースしてメタと **1 つ以上の** `ParsedGoal` を返す。
pub(crate) fn parse_brace_block(
    full_text: &str,
    range:     (usize, usize),
) -> Result<(BraceFileMeta, Vec<ParsedGoal>), Vec<ParseError>> {
    let (s, e) = range;
    if e <= s + 1 {
        return Err(vec![err(
            line_of_byte(full_text, s),
            "`{` に対応する `}` がありません",
        )]);
    }
    let slice = &full_text[s..=e];
    let inner = &slice[1..slice.len() - 1];
    let base = line_of_byte(full_text, s);
    parse_brace_inner(inner, base)
}

fn err(line: u32, msg: impl Into<String>) -> ParseError {
    ParseError {
        span:     Span::whole_line(line, 1),
        message:  msg.into(),
        severity: ErrorSeverity::Error,
    }
}

fn trim_comma(s: &str) -> &str {
    s.trim().trim_end_matches(',').trim()
}

fn strip_line_comment(line: &str) -> &str {
    let ts = line.trim_start();
    if ts.starts_with('#') {
        return "";
    }
    if let Some(i) = line.find("//") {
        line[..i].trim_end()
    } else {
        line
    }
}

/// フィールド行として解釈するための区切り（ASCII / 全角コロン）。
fn line_has_field_colon(line: &str) -> bool {
    line.contains(':') || line.contains('：')
}

fn split_key_colon(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    line.split_once(':')
        .or_else(|| line.split_once('：'))
        .map(|(k, v)| (k.trim(), v.trim()))
}

/// `key:` 行の `rest` が `{` で始まるとき、対応する外側の `}` まで `{` `}` の深さで走査し、
/// 内側の本文を行（元の行番号付き）に分割して返す。`settings:` 内の `{` などネストに対応。
fn read_balanced_brace_inner_lines(
    lines: &[(u32, String)],
    mut i: usize,
    rest:  &str,
    ln:    u32,
    ctx:   &str,
) -> Result<(Vec<(u32, String)>, usize), Vec<ParseError>> {
    let rest = rest.trim();
    let Some(after_open) = rest.strip_prefix('{') else {
        return Err(vec![err(
            ln,
            format!("`{ctx}:` の直後は `{{` で始めてください（例: `{ctx}: {{`）"),
        )]);
    };

    let mut depth = 1u32;
    let mut inner: Vec<(u32, String)> = Vec::new();
    let mut buf = String::new();
    let mut cur_ln = ln;
    let mut segment: &str = after_open;

    loop {
        for ch in segment.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    buf.push(ch);
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let t = buf.trim();
                        if !t.is_empty() {
                            inner.push((cur_ln, t.to_owned()));
                        }
                        return Ok((inner, i + 1));
                    }
                    buf.push(ch);
                }
                _ => buf.push(ch),
            }
        }

        let t = buf.trim();
        if !t.is_empty() {
            inner.push((cur_ln, t.to_owned()));
        }
        buf.clear();

        i += 1;
        if i >= lines.len() {
            return Err(vec![err(
                ln,
                format!("`{ctx}: {{` に対応する閉じ `}}` がありません"),
            )]);
        }
        cur_ln = lines[i].0;
        segment = &lines[i].1;
    }
}

/// `rest` が `[` で始まるとき、対応する `]` まで `[` `]` の深さで走査する（`context:` 用）。
fn read_balanced_square_inner_lines(
    lines: &[(u32, String)],
    mut i: usize,
    rest:  &str,
    ln:    u32,
    ctx:   &str,
) -> Result<(Vec<(u32, String)>, usize), Vec<ParseError>> {
    let rest = rest.trim();
    let Some(after_open) = rest.strip_prefix('[') else {
        return Err(vec![err(
            ln,
            format!("`{ctx}:` の直後が `[` ではありません（内部エラー）"),
        )]);
    };

    let mut depth = 1u32;
    let mut inner: Vec<(u32, String)> = Vec::new();
    let mut buf = String::new();
    let mut cur_ln = ln;
    let mut segment: &str = after_open;

    loop {
        for ch in segment.chars() {
            match ch {
                '[' => {
                    depth += 1;
                    buf.push(ch);
                }
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        let t = buf.trim();
                        if !t.is_empty() {
                            inner.push((cur_ln, t.to_owned()));
                        }
                        return Ok((inner, i + 1));
                    }
                    buf.push(ch);
                }
                _ => buf.push(ch),
            }
        }

        let t = buf.trim();
        if !t.is_empty() {
            inner.push((cur_ln, t.to_owned()));
        }
        buf.clear();

        i += 1;
        if i >= lines.len() {
            return Err(vec![err(
                ln,
                format!("`{ctx}: [` に対応する閉じ `]` がありません"),
            )]);
        }
        cur_ln = lines[i].0;
        segment = &lines[i].1;
    }
}

fn is_define_body_field_line(line: &str) -> bool {
    let t = strip_line_comment(line).trim();
    let Some((k, _)) = split_key_colon(t) else {
        return false;
    };
    matches!(
        k.to_lowercase().as_str(),
        "goal" | "settings" | "context"
    )
}

/// `context:` の値が「`-` で始まる行の並び」のとき。`rest` は同一行の `:` 右（空可）。
fn parse_context_dash_list(
    lines: &[(u32, String)],
    idx:   usize,
    rest:  &str,
) -> Result<(String, usize), Vec<ParseError>> {
    let mut bullets: Vec<String> = Vec::new();
    let rest = rest.trim();
    let mut i = idx + 1;

    if !rest.is_empty() {
        if !rest.starts_with('-') {
            return Err(vec![err(
                lines[idx].0,
                "`context:` の箇条書きは `- 項目` で始めるか、`[` / `{` / `\"` 形式を使ってください",
            )]);
        }
        bullets.push(rest.to_owned());
    }

    while i < lines.len() {
        let (line_ln, ref line) = lines[i];
        let t = strip_line_comment(line).trim();
        if t.is_empty() {
            i += 1;
            continue;
        }
        if is_define_body_field_line(line) {
            break;
        }
        if t.starts_with('-') {
            bullets.push(t.to_owned());
            i += 1;
        } else {
            return Err(vec![err(
                line_ln,
                "context の `-` リスト中に、`goal:` / `settings:` 以外の行が入っています。\
                 並列箇条書きは各行を `-` で始めるか、`context: [ ... ]` を使ってください",
            )]);
        }
    }

    Ok((bullets.join("\n"), i))
}

/// `rest` = `context:` の右側全体（1 行分）。`"..."` / `{` / `[` / `-` リスト。
fn parse_double_quoted(rest: &str, ln: u32) -> Result<(String, &str), Vec<ParseError>> {
    let s = rest.trim();
    let Some(body) = s.strip_prefix('"') else {
        return Err(vec![err(ln, "内部エラー: `context:` の値が引用符で始まっていません")]);
    };
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                let tail = chars.as_str();
                return Ok((out, tail));
            }
            '\\' => {
                let Some(esc) = chars.next() else {
                    return Err(vec![err(
                        ln,
                        "`context:` の文字列で `\\` の後に文字がありません",
                    )]);
                };
                match esc {
                    '"' => out.push('"'),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    '\\' => out.push('\\'),
                    c => out.push(c),
                }
            }
            c => out.push(c),
        }
    }
    Err(vec![err(
        ln,
        "`context:` の文字列が閉じ引用符 `\"` で終わっていません",
    )])
}

fn join_context_inner_lines(inner_lines: Vec<(u32, String)>) -> String {
    inner_lines
        .into_iter()
        .map(|(_, l)| strip_line_comment(&l).trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `context:` の値。`"..."` / `{...}` / `[...]` / `context:` の次行からの `-` リスト（同一行 `context: - x` も可）。
fn parse_context_field(
    lines: &[(u32, String)],
    idx:   usize,
    rest:  &str,
    ln:    u32,
) -> Result<(String, usize), Vec<ParseError>> {
    let t = rest.trim();
    if t.starts_with('[') {
        let (inner_lines, next) = read_balanced_square_inner_lines(lines, idx, rest, ln, "context")?;
        return Ok((join_context_inner_lines(inner_lines), next));
    }
    if t.starts_with('{') {
        let (inner_lines, next) = read_balanced_brace_inner_lines(lines, idx, rest, ln, "context")?;
        return Ok((join_context_inner_lines(inner_lines), next));
    }
    if t.starts_with('"') {
        let (s, tail) = parse_double_quoted(rest, ln)?;
        let tail = strip_line_comment(tail).trim();
        if !tail.is_empty() && !tail.starts_with("//") {
            return Err(vec![err(
                ln,
                format!("`context:` の引用符の後に不要なトークンがあります: `{tail}`"),
            )]);
        }
        return Ok((s, idx + 1));
    }
    if t.is_empty() || t.starts_with('-') {
        return parse_context_dash_list(lines, idx, rest);
    }
    Err(vec![err(
        ln,
        "`context:` は次のいずれかで指定してください: \
         `\"1行\"` / `{ 複数行 }` / `[ 並列・箇条書き ]` / `context:` の次行から `- 項目` を並べる",
    )])
}

fn validate_goal_name(name: &str, ln: u32) -> Result<(), Vec<ParseError>> {
    if name.is_empty() || name == "<Name>" {
        return Err(vec![err(
            ln,
            "`goal:` の後に識別子が必要です（例: `goal: safe_division`）",
        )]);
    }
    if name.contains(|c: char| !(c.is_alphanumeric() || c == '_')) {
        return Err(vec![err(
            ln,
            format!("goal 名に使えない文字があります: `{name}`（英数字と `_` のみ）"),
        )]);
    }
    Ok(())
}

fn assemble_parsed_goal(
    name:           &str,
    name_line:      u32,
    settings_lines: Vec<(u32, String)>,
    context:        Option<String>,
) -> Result<ParsedGoal, Vec<ParseError>> {
    let mut goal = ContractualGoal::new(name);
    let mut items: Vec<ParsedItem> = Vec::new();

    for (ln, sline) in settings_lines {
        let cleaned = strip_line_comment(&sline).trim();
        if cleaned.is_empty() {
            continue;
        }
        let Some((kw, rest)) = split_key_colon(cleaned) else {
            return Err(vec![err(
                ln,
                format!("settings 内: `{cleaned}` は key: value 形式ではありません"),
            )]);
        };
        let rest = trim_comma(rest);
        if rest.is_empty() {
            return Err(vec![err(ln, format!("`{kw}:` の後に値が必要です"))]);
        }
        match kw.to_lowercase().as_str() {
            "require" => match parse_predicate(rest) {
                Ok(pred) => {
                    let display = pred.to_string();
                    items.push(ParsedItem {
                        span:    Span::whole_line(ln, sline.len() as u32),
                        kind:    ItemKind::Precondition,
                        hover:   String::new(),
                        display: display.clone(),
                    });
                    goal = goal.require(pred);
                }
                Err(msg) => return Err(vec![err(ln, msg)]),
            },
            "ensure" => match parse_predicate(rest) {
                Ok(pred) => {
                    let display = pred.to_string();
                    items.push(ParsedItem {
                        span:    Span::whole_line(ln, sline.len() as u32),
                        kind:    ItemKind::Postcondition,
                        hover:   String::new(),
                        display: display.clone(),
                    });
                    goal = goal.ensure(pred);
                }
                Err(msg) => return Err(vec![err(ln, msg)]),
            },
            "invariant" => match parse_predicate(rest) {
                Ok(pred) => {
                    let display = pred.to_string();
                    items.push(ParsedItem {
                        span:    Span::whole_line(ln, sline.len() as u32),
                        kind:    ItemKind::Invariant,
                        hover:   String::new(),
                        display: display.clone(),
                    });
                    goal = goal.invariant(pred);
                }
                Err(msg) => return Err(vec![err(ln, msg)]),
            },
            "forbid" => match parse_forbidden(rest) {
                Ok(fp) => {
                    let display = fp.to_string();
                    items.push(ParsedItem {
                        span:    Span::whole_line(ln, sline.len() as u32),
                        kind:    ItemKind::Forbidden,
                        hover:   String::new(),
                        display: display.clone(),
                    });
                    goal = goal.forbid(fp);
                }
                Err(msg) => return Err(vec![err(ln, msg)]),
            },
            other => {
                return Err(vec![err(
                    ln,
                    format!("settings 内の不明キー `{other}` — require / ensure / invariant / forbid"),
                )]);
            }
        }
    }

    let name_span = Span::new(name_line, 0, name.len() as u32);
    Ok(ParsedGoal {
        goal,
        name_span,
        items,
        context,
    })
}

fn parse_define_body(inner: &[(u32, String)]) -> Result<ParsedGoal, Vec<ParseError>> {
    let mut idx = 0usize;
    let mut goal_name: Option<String> = None;
    let mut goal_ln: u32 = 0;
    let mut settings_lines: Option<Vec<(u32, String)>> = None;
    let mut context: Option<String> = None;

    while idx < inner.len() {
        let (ln, ref line) = inner[idx];
        let cleaned = strip_line_comment(line).trim();
        if cleaned.is_empty() {
            idx += 1;
            continue;
        }
        let Some((key, rest)) = split_key_colon(cleaned) else {
            return Err(vec![err(
                ln,
                format!("define 内: `{cleaned}` は key: value 形式ではありません"),
            )]);
        };
        match key.to_lowercase().as_str() {
            "context" => {
                if context.is_some() {
                    return Err(vec![err(ln, "`context:` は define 内で 1 度だけ指定してください")]);
                }
                let (text, next) = parse_context_field(inner, idx, rest, ln)?;
                context = if text.trim().is_empty() {
                    None
                } else {
                    Some(text)
                };
                idx = next;
            }
            "goal" => {
                let name = trim_comma(rest);
                validate_goal_name(name, ln)?;
                goal_name = Some(name.to_owned());
                goal_ln = ln;
                idx += 1;
            }
            "settings" => {
                let (body, next) = parse_settings_header(inner, idx, rest, ln)?;
                settings_lines = Some(body);
                idx = next;
            }
            other => {
                return Err(vec![err(
                    ln,
                    format!(
                        "define 内の不明フィールド `{other}` — `context` / `goal` / `settings`"
                    ),
                )]);
            }
        }
    }

    let name = goal_name.ok_or_else(|| {
        vec![err(
            inner.first().map(|l| l.0).unwrap_or(1),
            "define 内に `goal:` がありません",
        )]
    })?;
    let settings = settings_lines.ok_or_else(|| {
        vec![err(
            inner.first().map(|l| l.0).unwrap_or(1),
            "define 内に `settings:` がありません",
        )]
    })?;

    assemble_parsed_goal(&name, goal_ln, settings, context)
}

/// `inner` = 外側の `{` `}` を除いた本文
fn parse_brace_inner(inner: &str, base_line: u32) -> Result<(BraceFileMeta, Vec<ParsedGoal>), Vec<ParseError>> {
    let raw_lines: Vec<&str> = inner.lines().collect();
    let mut lines: Vec<(u32, String)> = Vec::new();
    for (i, raw) in raw_lines.iter().enumerate() {
        let t = strip_line_comment(raw).trim();
        if t.is_empty() {
            continue;
        }
        lines.push((base_line + i as u32, t.to_owned()));
    }

    let mut idx = 0usize;
    let mut meta = BraceFileMeta::default();
    let mut out: Vec<ParsedGoal> = Vec::new();
    let mut pending_goal: Option<(String, u32)> = None;

    while idx < lines.len() {
        let (ln, ref line) = lines[idx];
        if !line_has_field_colon(line) {
            idx += 1;
            continue;
        }
        let Some((key, rest)) = split_key_colon(line) else {
            return Err(vec![err(ln, format!("ブロック内: コロン付きキーが必要です: `{line}`"))]);
        };
        match key.to_lowercase().as_str() {
            "config" => {
                let (m, next) = parse_config_section(&lines, idx, rest, ln)?;
                meta = merge_meta(meta, m);
                idx = next;
            }
            "define" => {
                let (inner_lines, next) =
                    read_balanced_brace_inner_lines(&lines, idx, rest, ln, "define")?;
                let pg = parse_define_body(&inner_lines)?;
                out.push(pg);
                idx = next;
            }
            "goal" => {
                if pending_goal.is_some() {
                    return Err(vec![err(
                        ln,
                        "連続した `goal:` です。前の goal に `settings:` を付けるか、`define: { ... }` にまとめてください",
                    )]);
                }
                let name = trim_comma(rest);
                validate_goal_name(name, ln)?;
                pending_goal = Some((name.to_owned(), ln));
                idx += 1;
            }
            "settings" => {
                let Some((ref gname, gln)) = pending_goal else {
                    return Err(vec![err(
                        ln,
                        "`settings:` の前にトップレベルの `goal:` が必要です（複数 goal は `define: { ... }` を繰り返してください）",
                    )]);
                };
                let (body, next) = parse_settings_header(&lines, idx, rest, ln)?;
                let pg = assemble_parsed_goal(gname, gln, body, None)?;
                out.push(pg);
                pending_goal = None;
                idx = next;
            }
            other => {
                return Err(vec![err(
                    ln,
                    format!(
                        "不明なフィールド `{other}` — `config`, `define`, `goal`, `settings`"
                    ),
                )]);
            }
        }
    }

    if let Some((ref name, gln)) = pending_goal {
        return Err(vec![err(
            gln,
            format!("`goal:` `{name}` に対応する `settings:` がありません"),
        )]);
    }

    if out.is_empty() {
        return Err(vec![err(
            base_line,
            "ブロックに goal がありません（`define: { ... }` または `goal:` と `settings:` を指定してください）",
        )]);
    }

    Ok((meta, out))
}

fn merge_meta(mut a: BraceFileMeta, b: BraceFileMeta) -> BraceFileMeta {
    if b.agent.is_some() {
        a.agent = b.agent;
    }
    if b.model.is_some() {
        a.model = b.model;
    }
    if b.lang.is_some() {
        a.lang = b.lang;
    }
    if b.api_key.is_some() {
        a.api_key = b.api_key;
    }
    a
}

/// `config:` 行から `config_obj` を読み、`(meta, 次の idx)` を返す。
fn parse_config_section(
    lines: &[(u32, String)],
    idx:   usize,
    rest:  &str,
    ln:    u32,
) -> Result<(BraceFileMeta, usize), Vec<ParseError>> {
    let rest = rest.trim();
    let Some(after_open) = rest.strip_prefix('{') else {
        return Err(vec![err(ln, "`config:` の後は `{` で始まる必要があります（例: `config: {`）")]);
    };
    let mut meta = BraceFileMeta::default();

    // 単行閉じ: `config: { agent: mock, lang: rust }`
    if let Some(rb) = after_open.rfind('}') {
        let inside = after_open[..rb].trim();
        if !inside.is_empty() {
            for part in inside.split(',') {
                let p = part.trim();
                if !p.is_empty() {
                    apply_config_pair(p, ln, &mut meta)?;
                }
            }
        }
        return Ok((meta, idx + 1));
    }

    // 複数行: `config: {` の次行から `}` まで
    let first = after_open.trim();
    if !first.is_empty() {
        for part in first.split(',') {
            let p = part.trim();
            if !p.is_empty() {
                apply_config_pair(p, ln, &mut meta)?;
            }
        }
    }

    let mut i = idx + 1;
    while i < lines.len() {
        let (ln_i, ref line) = lines[i];
        let t = strip_line_comment(line).trim();
        if t.is_empty() {
            i += 1;
            continue;
        }
        if t == "}" || t == "}," {
            return Ok((meta, i + 1));
        }
        if let Some((k, v)) = split_key_colon(t) {
            let v = trim_comma(v);
            match k.to_lowercase().as_str() {
                "agent" => {
                    meta.agent = Some(v.parse::<AgentKind>().map_err(|e| vec![err(ln_i, e)])?);
                }
                "model" => {
                    if !v.is_empty() {
                        meta.model = Some(v.to_owned());
                    }
                }
                "lang" | "language" => {
                    meta.lang = Some(v.parse::<TargetLang>().map_err(|e| vec![err(ln_i, e)])?);
                }
                "api_key" | "apikey" | "api-key" => {
                    if v.starts_with('$') {
                        let var = v.trim_start_matches('$');
                        if let Ok(s) = std::env::var(var) {
                            if !s.is_empty() {
                                meta.api_key = Some(s);
                            }
                        }
                    } else if !v.is_empty() {
                        meta.api_key = Some(v.to_owned());
                    }
                }
                _ => {
                    return Err(vec![err(
                        ln_i,
                        format!("config 内の不明キー `{k}` — agent / model / lang / api_key"),
                    )]);
                }
            }
        } else if !line_has_field_colon(t) {
            i += 1;
            continue;
        } else {
            return Err(vec![err(ln_i, format!("config 内: `{t}` は key: value 形式ではありません"))]);
        }
        i += 1;
    }
    Err(vec![err(ln, "`config: {` に対応する `}` がありません")])
}

fn apply_config_pair(pair_line: &str, ln: u32, meta: &mut BraceFileMeta) -> Result<(), Vec<ParseError>> {
    let Some((k, v)) = split_key_colon(pair_line) else {
        return Ok(());
    };
    let v = trim_comma(v);
    match k.to_lowercase().as_str() {
        "agent" => meta.agent = Some(v.parse::<AgentKind>().map_err(|e| vec![err(ln, e)])?),
        "model" => {
            if !v.is_empty() {
                meta.model = Some(v.to_owned());
            }
        }
        "lang" | "language" => {
            meta.lang = Some(v.parse::<TargetLang>().map_err(|e| vec![err(ln, e)])?);
        }
        "api_key" | "apikey" | "api-key" => {
            if v.starts_with('$') {
                let var = v.trim_start_matches('$');
                if let Ok(s) = std::env::var(var) {
                    if !s.is_empty() {
                        meta.api_key = Some(s);
                    }
                }
            } else if !v.is_empty() {
                meta.api_key = Some(v.to_owned());
            }
        }
        _ => {}
    }
    Ok(())
}

/// `settings: [` から `]` までを読み、内部の `(行番号, 行)` を返す。
fn parse_settings_header(
    lines: &[(u32, String)],
    idx:   usize,
    rest:  &str,
    ln:    u32,
) -> Result<(Vec<(u32, String)>, usize), Vec<ParseError>> {
    let mut tail = rest.trim();
    if !tail.starts_with('[') {
        return Err(vec![err(ln, "`settings:` の後に `[` が必要です（例: `settings: [`）")]);
    }
    tail = tail[1..].trim();
    let mut body: Vec<(u32, String)> = Vec::new();
    let mut i = idx + 1;

    // 同一行に `]`
    if let Some(pos) = tail.find(']') {
        let before = tail[..pos].trim();
        if before.starts_with('{') {
            return Err(vec![err(
                ln,
                "同一行の `settings: [ {` は未対応です。`[` の次行から `require:` を書いてください。",
            )]);
        }
        if !before.is_empty() {
            return Err(vec![err(ln, "同一行の settings は `settings: [` のみにしてください")]);
        }
        return Ok((body, i));
    }

    // `settings: [` の直後が `{` のみ（次行から `require:` … `}`）
    if tail == "{" || tail == "{," {
        let (inner, next_i) = read_braced_setting_lines(lines, idx + 1, ln)?;
        body.extend(inner);
        i = next_i;
        if i >= lines.len() {
            return Err(vec![err(ln, "`settings` の `]` がありません")]);
        }
        let (ln_close, ref line_close) = lines[i];
        let tc = line_close.trim();
        if tc != "]" && tc != "]," {
            return Err(vec![err(
                ln_close,
                format!("`settings` 内 `{{` ブロックの後は `]` です（得られた行: `{tc}`）"),
            )]);
        }
        return Ok((body, i + 1));
    }

    if !tail.is_empty() {
        // `settings: [ require: ...` 同一行
        if let Some((k, _)) = split_key_colon(tail) {
            let kl = k.to_lowercase();
            if matches!(kl.as_str(), "require" | "ensure" | "invariant" | "forbid") {
                body.push((ln, tail.to_owned()));
            }
        }
    }

    while i < lines.len() {
        let (ln_i, ref line) = lines[i];
        let cleaned = strip_line_comment(line).trim();
        if cleaned.is_empty() {
            i += 1;
            continue;
        }
        if cleaned == "]" || cleaned == "]," {
            return Ok((body, i + 1));
        }
        if cleaned == "{" || cleaned == "{," {
            let (inner, next_i) = read_braced_setting_lines(lines, i + 1, ln_i)?;
            body.extend(inner);
            i = next_i;
            continue;
        }
        body.push((ln_i, line.clone()));
        i += 1;
    }
    Err(vec![err(ln, "`settings: [` に対応する `]` がありません")])
}

/// `{` の次行から `}` までを読み、`(行, 内容)` のリストと `}` の次のインデックスを返す。
fn read_braced_setting_lines(
    lines: &[(u32, String)],
    mut i: usize,
    ln_brace: u32,
) -> Result<(Vec<(u32, String)>, usize), Vec<ParseError>> {
    let mut body = Vec::new();
    while i < lines.len() {
        let (ln_i, ref line) = lines[i];
        let cleaned = strip_line_comment(line).trim();
        if cleaned.is_empty() {
            i += 1;
            continue;
        }
        if cleaned == "}" || cleaned == "}," {
            return Ok((body, i + 1));
        }
        body.push((ln_i, line.clone()));
        i += 1;
    }
    Err(vec![err(ln_brace, "`settings` 内の `{` に対応する `}` がありません")])
}

/// `ParseResult` のファイルレベルフィールドへマージ（後勝ち）
pub(crate) fn merge_brace_meta_into_result(
    result: &mut super::ParseResult,
    meta:   BraceFileMeta,
) {
    if let Some(a) = meta.agent {
        result.agent = Some(a);
    }
    if let Some(m) = meta.model {
        result.model = Some(m);
    }
    if let Some(l) = meta.lang {
        result.lang = l;
    }
    if let Some(k) = meta.api_key {
        result.api_key = Some(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_outer_brace_ranges_ignore_braces_inside_line_comments() {
        let src = "// 最外殻 { config, define... } 注釈\n// { goal, settings }\n{\n  config: { agent: mock }\n  define: {\n    goal: g\n    settings: [\n      ensure: n_ok\n    ]\n  }\n}\n";
        let r = find_outer_brace_ranges(src);
        assert_eq!(r.len(), 1, "expected only real block, got {:?}", r);
    }

    #[test]
    fn outer_block_skips_lines_without_field_colon() {
        let src = r#"
{
  注記 最外殻には config と define だけ書く
  config: { agent: mock }
  define: {
    goal: g
    settings: [
      ensure: n_ok
    ]
  }
}
"#;
        let ranges = find_outer_brace_ranges(src);
        let (_m, pgs) = parse_brace_block(src, ranges[0]).unwrap();
        assert_eq!(pgs.len(), 1);
        assert_eq!(pgs[0].goal.name, "g");
    }

    #[test]
    fn define_and_settings_skip_hash_comments() {
        let src = r#"
{
  define: {
    context: [ - x ]
    # 設計・査定の意図
    goal: g
    settings: [
      # require は後で足す
      ensure: n_ok
    ]
  }
}
"#;
        let ranges = find_outer_brace_ranges(src);
        assert!(parse_brace_block(src, ranges[0]).is_ok());
    }

    #[test]
    fn brace_block_roundtrip_goal() {
        let src = r#"
{
  config: {
    agent: mock
    lang: rust
  },
  goal: from_brace,
  settings: [
    require: InRange(n, 0, 10)
    ensure: n_ok
    forbid: UnprovenUnwrap
  ]
}
"#;
        let ranges = find_outer_brace_ranges(src);
        assert_eq!(ranges.len(), 1);
        let (meta, pgs) = parse_brace_block(src, ranges[0]).unwrap();
        assert_eq!(pgs.len(), 1);
        let pg = &pgs[0];
        assert_eq!(meta.agent, Some(AgentKind::Mock));
        assert_eq!(meta.lang, Some(TargetLang::Rust));
        assert_eq!(pg.goal.name, "from_brace");
        assert_eq!(pg.goal.preconditions.len(), 1);
        assert_eq!(pg.goal.postconditions.len(), 1);
    }

    #[test]
    fn two_define_blocks_yield_two_goals() {
        let src = r#"
{
  config: { agent: mock }
  define: {
    goal: first_def
    settings: [
      require: InRange(a, 0, 1)
    ]
  }
  define: {
    goal: second_def
    settings: [
      ensure: n_ok
    ]
  }
}
"#;
        let ranges = find_outer_brace_ranges(src);
        let (_meta, pgs) = parse_brace_block(src, ranges[0]).unwrap();
        assert_eq!(pgs.len(), 2);
        assert_eq!(pgs[0].goal.name, "first_def");
        assert_eq!(pgs[1].goal.name, "second_def");
    }

    #[test]
    fn define_context_quoted_string() {
        let src = r#"
{
  define: {
    context: "査定用の設計意図"
    goal: g
    settings: [
      ensure: n_ok
    ]
  }
}
"#;
        let ranges = find_outer_brace_ranges(src);
        let (_meta, pgs) = parse_brace_block(src, ranges[0]).unwrap();
        assert_eq!(pgs[0].context.as_deref(), Some("査定用の設計意図"));
    }

    #[test]
    fn define_context_braced_multiline() {
        let src = r#"
{
  define: {
    context: {
      行1の意図
      行2の補足
    }
    goal: g
    settings: [
      ensure: n_ok
    ]
  }
}
"#;
        let ranges = find_outer_brace_ranges(src);
        let (_meta, pgs) = parse_brace_block(src, ranges[0]).unwrap();
        assert!(pgs[0].context.as_ref().unwrap().contains("行1"));
        assert!(pgs[0].context.as_ref().unwrap().contains("行2"));
    }

    #[test]
    fn define_context_bracket_with_dash_items() {
        let src = r#"
{
  define: {
    context: [
      - 並列の意図A
      - 並列の意図B
    ]
    goal: g
    settings: [
      ensure: n_ok
    ]
  }
}
"#;
        let ranges = find_outer_brace_ranges(src);
        let (_meta, pgs) = parse_brace_block(src, ranges[0]).unwrap();
        let c = pgs[0].context.as_ref().unwrap();
        assert!(c.contains("- 並列の意図A"));
        assert!(c.contains("- 並列の意図B"));
    }

    #[test]
    fn define_context_dash_list_only() {
        let src = r#"
{
  define: {
    context:
      - 行1
      - 行2
    goal: g
    settings: [
      ensure: n_ok
    ]
  }
}
"#;
        let ranges = find_outer_brace_ranges(src);
        let (_meta, pgs) = parse_brace_block(src, ranges[0]).unwrap();
        let c = pgs[0].context.as_ref().unwrap();
        assert!(c.contains("- 行1"));
        assert!(c.contains("- 行2"));
    }

    #[test]
    fn define_context_same_line_dash_then_continued() {
        let src = r#"
{
  define: {
    context: - 先頭
      - 続き
    goal: g
    settings: [
      ensure: n_ok
    ]
  }
}
"#;
        let ranges = find_outer_brace_ranges(src);
        let (_meta, pgs) = parse_brace_block(src, ranges[0]).unwrap();
        let c = pgs[0].context.as_ref().unwrap();
        assert!(c.starts_with("- 先頭"));
        assert!(c.contains("- 続き"));
    }

    #[test]
    fn define_duplicate_context_errors() {
        let src = r#"
{
  define: {
    context: "a"
    context: "b"
    goal: g
    settings: [
      ensure: n_ok
    ]
  }
}
"#;
        let ranges = find_outer_brace_ranges(src);
        assert!(parse_brace_block(src, ranges[0]).is_err());
    }
}
