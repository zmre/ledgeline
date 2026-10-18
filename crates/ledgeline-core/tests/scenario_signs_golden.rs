//! The engine's scenario sign and section rules reproduce the TypeScript they
//! replaced (plan 26 B6).
//!
//! `fixtures/projections-signs/scenario-signs.json` was CAPTURED from the SPA's
//! `projections/scenarioModel.ts` — `signedQuantity` of `$1234.56` and of
//! `$-1234.56`, and `resolvedSection` for each role × period — over every
//! account each fixture journal's `/api/journal` classified, plus names the
//! engine never saw (the ones a user is still typing), before those rules were
//! deleted. A failure here is the port drifting; the fixture is not to be
//! regenerated from Rust.

mod common;

use common::fixtures_dir;
use ledgeline_core::Dec;
use ledgeline_core::parse_journal;
use ledgeline_core::projections::{LineRole, line_section};
use ledgeline_core::reports::{account_decls, declared_types, signed_entry};
use serde_json::{Value, json};

fn dec_json(value: Dec) -> Value {
    json!({"m": value.mantissa.to_string(), "p": value.places})
}

#[test]
fn scenario_signs_and_sections_match_the_captured_typescript() {
    let path = fixtures_dir().join("projections-signs/scenario-signs.json");
    let golden: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("readable")).expect("JSON");
    let mut accounts = 0;
    let mut sections = 0;
    for entry in golden["journals"].as_array().expect("journals") {
        let rel = entry["journal"].as_str().expect("journal");
        let path = fixtures_dir().join(rel);
        let text = std::fs::read_to_string(&path).expect("fixture readable");
        let journal = parse_journal(&text, &path.to_string_lossy()).expect("fixture parses");
        let declared = declared_types(&account_decls(&journal));
        for case in entry["accounts"].as_array().expect("accounts") {
            let account = case["account"].as_str().expect("account");
            let magnitude = Dec::new(123_456, 2);
            assert_eq!(
                dec_json(signed_entry(account, magnitude, &declared).expect("signs")),
                case["signed"],
                "{rel}: {account} signed"
            );
            assert_eq!(
                dec_json(signed_entry(account, Dec::new(-123_456, 2), &declared).expect("signs")),
                case["signedFromNegative"],
                "{rel}: {account} signed from a negative entry"
            );
            for want in case["sections"].as_array().expect("sections") {
                let role = LineRole::parse(want["role"].as_str().expect("role")).expect("role");
                let raw = want["raw"].as_str().expect("raw");
                let section = line_section(role, raw, account, &declared);
                assert_eq!(
                    serde_json::to_value(section).expect("serializes"),
                    want["section"],
                    "{rel}: {account} {role:?} {raw:?}"
                );
                sections += 1;
            }
            accounts += 1;
        }
    }
    assert!(accounts >= 150, "too few accounts compared: {accounts}");
    assert!(sections >= 1500, "too few sections compared: {sections}");
}
