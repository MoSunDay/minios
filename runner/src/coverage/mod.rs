//! Coverage matrix derivation + gate (plans/linux26 40 §3.1, 04 §4).
//!
//! The input manifest (`tests/user/coverage.toml`) carries ONLY static
//! fields; this module never writes it back. Per round it derives
//! `activation` (milestone vs current stage) and `evidence` (round result
//! id/path/result), renders the derived results file as JSON, and applies
//! the gate. Pure logic + unit tests; process I/O lives in main.rs.

/// Static manifest entry (40 §3.1 schema). `config` may list several frozen
/// config ids; a derived result file always covers exactly one config.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseSpec {
    pub case_id: String,
    pub source: String,
    pub milestone: String,
    pub config: Vec<String>,
    pub mode: String,
    pub runner_test_name: String,
    pub requirement: String,
    pub adr: String,
}

/// One derived evidence record (40 §3.1: `{id,path,result}`, never faked
/// for `not-active` cases).
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence {
    pub id: String,
    pub path: String,
    pub result: String,
}

/// Manifest case plus this round's derivation.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedCase {
    pub spec: CaseSpec,
    pub activation: &'static str,
    pub evidence: Option<Evidence>,
}

const MODES: [&str; 3] = ["host", "qemu", "external"];
const REQUIREMENTS: [&str; 3] = ["required", "optional", "out-of-scope"];

/// Milestone ordering for activation: M0R < M1R < M2 < … < M13.
pub fn milestone_rank(m: &str) -> Option<u8> {
    match m {
        "M0R" => Some(0),
        "M1R" => Some(1),
        "M2" => Some(2),
        "M3" => Some(3),
        "M4" => Some(4),
        "M5" => Some(5),
        "M6" => Some(6),
        "M7" => Some(7),
        "M8" => Some(8),
        "M9" => Some(9),
        "M10" => Some(10),
        "M11" => Some(11),
        "M12" => Some(12),
        "M13" => Some(13),
        _ => None,
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Parse the TOML-subset manifest: `[[cases]]` tables whose fields are all
/// scalar strings or one string array. Anything else is a hard error — the
/// manifest is frozen machine input, not best-effort config.
pub fn parse_manifest(text: &str) -> Result<Vec<CaseSpec>, String> {
    let mut out: Vec<CaseSpec> = Vec::new();
    let mut have = false; // inside a [[cases]] table
    let mut seen = std::collections::BTreeSet::new();
    for (no, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[cases]]" {
            out.push(CaseSpec {
                case_id: String::new(),
                source: String::new(),
                milestone: String::new(),
                config: Vec::new(),
                mode: String::new(),
                runner_test_name: String::new(),
                requirement: String::new(),
                adr: String::new(),
            });
            have = true;
            continue;
        }
        let Some(eq) = line.find('=') else {
            return Err(format!("line {}: not `key = value`: {line:?}'", no + 1));
        };
        let key = line[..eq].trim();
        let val = line[eq + 1..].trim();
        if !have {
            return Err(format!("line {}: field outside [[cases]]: {key}", no + 1));
        }
        let slot = out.last_mut().unwrap();
        let dup = |k: &str| format!("duplicate field {k}");
        match key {
            "case_id" => {
                if !slot.case_id.is_empty() {
                    return Err(dup(key));
                }
                slot.case_id = parse_str(val, no)?;
                if !seen.insert(slot.case_id.clone()) {
                    return Err(format!("duplicate case_id {}", slot.case_id));
                }
            }
            "source" | "milestone" | "mode" | "runner_test_name" | "adr" => {
                let field = match key {
                    "source" => &mut slot.source,
                    "milestone" => &mut slot.milestone,
                    "mode" => &mut slot.mode,
                    "runner_test_name" => &mut slot.runner_test_name,
                    _ => &mut slot.adr,
                };
                if !field.is_empty() {
                    return Err(dup(key));
                }
                *field = parse_str(val, no)?;
            }
            "config" => slot.config = parse_str_array(val, no)?,
            "requirement" => {
                if !slot.requirement.is_empty() {
                    return Err(dup(key));
                }
                slot.requirement = parse_str(val, no)?;
            }
            other => return Err(format!("line {}: unknown field {other}", no + 1)),
        }
    }
    for (i, c) in out.iter().enumerate() {
        let missing: Vec<&str> = [
            c.case_id.is_empty().then_some("case_id"),
            c.source.is_empty().then_some("source"),
            c.milestone.is_empty().then_some("milestone"),
            c.config.is_empty().then_some("config"),
            c.mode.is_empty().then_some("mode"),
            c.runner_test_name.is_empty().then_some("runner_test_name"),
            c.requirement.is_empty().then_some("requirement"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !missing.is_empty() {
            return Err(format!(
                "case #{}: missing fields {}",
                i + 1,
                missing.join(",")
            ));
        }
        if !MODES.contains(&c.mode.as_str()) {
            return Err(format!("{}: bad mode {:?}", c.case_id, c.mode));
        }
        if !REQUIREMENTS.contains(&c.requirement.as_str()) {
            return Err(format!(
                "{}: bad requirement {:?}",
                c.case_id, c.requirement
            ));
        }
        if milestone_rank(&c.milestone).is_none() {
            return Err(format!("{}: bad milestone {:?}", c.case_id, c.milestone));
        }
        if c.requirement == "out-of-scope" && c.adr.is_empty() {
            return Err(format!("{}: out-of-scope needs adr", c.case_id));
        }
    }
    Ok(out)
}

fn parse_str(val: &str, no: usize) -> Result<String, String> {
    if val.len() >= 2 && val.starts_with('"') && val.ends_with('"') {
        Ok(val[1..val.len() - 1].to_string())
    } else {
        Err(format!("line {}: expected \"string\": {val:?}", no + 1))
    }
}

fn parse_str_array(val: &str, no: usize) -> Result<Vec<String>, String> {
    let inner = val
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or_else(|| format!("line {}: expected [..]: {val:?}", no + 1))?;
    let mut items = Vec::new();
    for part in inner.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        items.push(parse_str(p, no)?);
    }
    if items.is_empty() {
        return Err(format!("line {}: empty config array", no + 1));
    }
    Ok(items)
}

/// Derive one round: activation by milestone vs `now`; evidence joined from
/// the round's per-case results. Returns derived cases and spurious-result
/// errors (results for unknown or not-active cases).
pub fn derive(
    specs: &[CaseSpec],
    now: &str,
    run_id: &str,
    config_id: &str,
    results: &std::collections::BTreeMap<String, String>,
) -> (Vec<DerivedCase>, Vec<String>) {
    let now_rank = milestone_rank(now).expect("current milestone validated by caller");
    let mut errs = Vec::new();
    let mut used = std::collections::BTreeSet::new();
    let derived = specs
        .iter()
        .map(|spec| {
            let active = milestone_rank(&spec.milestone).unwrap() <= now_rank;
            let evidence = if active {
                used.insert(spec.case_id.clone());
                results.get(&spec.case_id).map(|r| Evidence {
                    id: format!("{run_id}/{config_id}/{}", spec.case_id),
                    path: format!("results/{run_id}/{config_id}.json"),
                    result: r.clone(),
                })
            } else {
                if results.contains_key(&spec.case_id) {
                    used.insert(spec.case_id.clone());
                    errs.push(format!(
                        "{}: not-active case has a round result (must not fake evidence)",
                        spec.case_id
                    ));
                }
                None
            };
            DerivedCase {
                spec: spec.clone(),
                activation: if active { "active" } else { "not-active" },
                evidence,
            }
        })
        .collect();
    for id in results.keys() {
        if !used.contains(id) {
            errs.push(format!("{id}: round result matches no manifest case"));
        }
    }
    (derived, errs)
}

/// Gate (40 §3.1): fail on empty active-required set, any active required
/// case without PASS evidence, or any derivation error.
pub fn gate(cases: &[DerivedCase], derive_errs: &[String]) -> Result<(), Vec<String>> {
    let mut errs = derive_errs.to_vec();
    let active_required: Vec<&DerivedCase> = cases
        .iter()
        .filter(|c| c.activation == "active" && c.spec.requirement == "required")
        .collect();
    if active_required.is_empty() {
        errs.push("gate: no active required cases for this milestone".to_string());
    }
    for c in &active_required {
        match &c.evidence {
            Some(e) if e.result == "PASS" => {}
            Some(e) => errs.push(format!(
                "gate: {} active required result {}",
                c.spec.case_id, e.result
            )),
            None => errs.push(format!(
                "gate: {} active required has no result (skip/missing)",
                c.spec.case_id
            )),
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn q(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

/// Render the derived results file (hand-rolled JSON, zero dependencies).
pub fn render_json(run_id: &str, config_id: &str, now: &str, cases: &[DerivedCase]) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"run\": {},\n", q(run_id)));
    out.push_str(&format!("  \"config_id\": {},\n", q(config_id)));
    out.push_str(&format!("  \"milestone\": {},\n", q(now)));
    out.push_str("  \"cases\": [\n");
    for (i, c) in cases.iter().enumerate() {
        let ev = match &c.evidence {
            Some(e) => format!(
                "{{ \"id\": {}, \"path\": {}, \"result\": {} }}",
                q(&e.id),
                q(&e.path),
                q(&e.result)
            ),
            None => "null".to_string(),
        };
        out.push_str(&format!(
            "    {{ \"case_id\": {}, \"source\": {}, \"milestone\": {}, \
             \"activation\": {}, \"config\": [{}], \"mode\": {}, \
             \"runner_test_name\": {}, \"requirement\": {}, \"adr\": {}, \
             \"evidence\": {} }}{}",
            q(&c.spec.case_id),
            q(&c.spec.source),
            q(&c.spec.milestone),
            q(c.activation),
            c.spec
                .config
                .iter()
                .map(|v| q(v))
                .collect::<Vec<_>>()
                .join(", "),
            q(&c.spec.mode),
            q(&c.spec.runner_test_name),
            q(&c.spec.requirement),
            q(&c.spec.adr),
            ev,
            if i + 1 == cases.len() { "" } else { "," }
        ));
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

#[cfg(test)]
mod tests;
