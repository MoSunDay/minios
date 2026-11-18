//! Coverage matrix self-tests (40 §3.1 gate semantics).

use super::*;
use std::collections::BTreeMap;

fn spec(id: &str, milestone: &str, requirement: &str) -> CaseSpec {
    CaseSpec {
        case_id: id.to_string(),
        source: "new:t".to_string(),
        milestone: milestone.to_string(),
        config: vec!["CORE-UP-GRUB-R1".to_string()],
        mode: "qemu".to_string(),
        runner_test_name: format!("run-m1r-user::{id}"),
        requirement: requirement.to_string(),
        adr: String::new(),
    }
}

#[test]
fn parse_manifest_roundtrip_and_rejections() {
    let toml = "[[cases]]\ncase_id = \"a\"  # comment\nsource = \"new:a\"\n\
                milestone = \"M1R\"\nconfig = [\"CORE-UP-GRUB-R1\"]\nmode = \"qemu\"\n\
                runner_test_name = \"run-m1r-user::a\"\nrequirement = \"required\"\nadr = \"\"\n";
    let v = parse_manifest(toml).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].case_id, "a");
    assert!(parse_manifest("[[cases]]\ncase_id = \"a\"\nnope = \"x\"\n").is_err());
    assert!(parse_manifest("[[cases]]\ncase_id = \"a\"\nmode = \"tcp\"\n").is_err());
    assert!(parse_manifest("[[cases]]\ncase_id = \"a\"\nmilestone = \"M99\"\n").is_err());
    assert!(parse_manifest("case_id = \"a\"\n").is_err());
    let dup = format!("{toml}[[cases]]\n{toml}");
    assert!(parse_manifest(&dup).is_err());
}

#[test]
fn gate_passes_full_active_required_coverage() {
    let specs = [spec("a", "M1R", "required"), spec("b", "M2", "required")];
    let mut results = BTreeMap::new();
    results.insert("a".to_string(), "PASS".to_string());
    let (derived, errs) = derive(&specs, "M1R", "r", "CORE-UP-GRUB-R1", &results);
    assert!(errs.is_empty());
    assert_eq!(derived[0].activation, "active");
    assert_eq!(derived[1].activation, "not-active");
    assert!(derived[1].evidence.is_none());
    assert!(gate(&derived, &errs).is_ok());
}

#[test]
fn gate_fails_on_missing_fail_empty_and_spurious() {
    // active required with no result at all
    let specs = [spec("a", "M1R", "required")];
    let (d, e) = derive(&specs, "M1R", "r", "C", &BTreeMap::new());
    assert!(gate(&d, &e).is_err());
    // failed result
    let mut r = BTreeMap::new();
    r.insert("a".to_string(), "FAIL".to_string());
    let (d, e) = derive(&specs, "M1R", "r", "C", &r);
    assert!(gate(&d, &e).is_err());
    // empty active-required set (only future cases)
    let fut = [spec("f", "M2", "required")];
    let (d, e) = derive(&fut, "M1R", "r", "C", &BTreeMap::new());
    assert!(gate(&d, &e).is_err());
    // result for a not-active case must not fake evidence
    let mut r = BTreeMap::new();
    r.insert("f".to_string(), "PASS".to_string());
    let (_, e) = derive(&fut, "M1R", "r", "C", &r);
    assert!(!e.is_empty());
    // result matching no manifest case
    let mut r = BTreeMap::new();
    r.insert("a".to_string(), "PASS".to_string());
    r.insert("zz".to_string(), "PASS".to_string());
    let (d, e) = derive(&specs, "M1R", "r", "C", &r);
    assert!(e.iter().any(|x| x.contains("no manifest case")));
    assert!(gate(&d, &e).is_err());
}

#[test]
fn render_json_shape() {
    let specs = [spec("a", "M1R", "required")];
    let mut r = BTreeMap::new();
    r.insert("a".to_string(), "PASS".to_string());
    let (d, _) = derive(&specs, "M1R", "rundir", "CORE-UP-GRUB-R1", &r);
    let j = render_json("rundir", "CORE-UP-GRUB-R1", "M1R", &d);
    assert!(j.contains("\"run\": \"rundir\""));
    assert!(j.contains("\"activation\": \"active\""));
    assert!(j.contains("\"path\": \"results/rundir/CORE-UP-GRUB-R1.json\""));
    assert!(j.contains("\"result\": \"PASS\""));
}
