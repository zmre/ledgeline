//! The engine's display formatter renders every amount exactly as the web UI
//! does (plan 26 B2).
//!
//! `fixtures/display-format/cases.json` is generated from the SPA's
//! `web/src/lib/domain/money.ts` and asserted on BOTH sides — here and in
//! `web/src/lib/domain/displayFormat.golden.test.ts` — so the two formatters
//! cannot drift: a row only one of them reproduces fails one of the suites.

mod common;

use common::fixtures_dir;
use ledgeline_core::Dec;
use ledgeline_core::display::{display_places, format_amount, format_dec};
use ledgeline_core::model::{AmountStyle, CommoditySide, DigitGroups};
use serde_json::Value;
use std::sync::Arc;

/// A golden style (the SPA's `AmountStyle` shape) as the engine's.
fn style_of(raw: &Value) -> AmountStyle {
    let mark = raw["decimalPoint"].as_str().expect("decimalPoint");
    let single = |text: &str| {
        let mut chars = text.chars();
        let c = chars.next().expect("a one-character mark");
        assert!(chars.next().is_none(), "multi-character mark {text:?}");
        c
    };
    AmountStyle {
        side: if raw["side"] == "R" {
            CommoditySide::Right
        } else {
            CommoditySide::Left
        },
        spaced: raw["spaced"] == true,
        decimal_mark: Some(single(mark)),
        digit_groups: raw["digitGroups"].as_array().map(|pair| {
            Arc::new(DigitGroups {
                mark: single(pair[0].as_str().expect("group mark")),
                sizes: pair[1]
                    .as_array()
                    .expect("group sizes")
                    .iter()
                    .map(|size| u8::try_from(size.as_u64().expect("size")).expect("small size"))
                    .collect(),
            })
        }),
        precision: u32::try_from(raw["precision"].as_u64().expect("precision"))
            .expect("precision fits"),
    }
}

fn as_u32(value: &Value) -> u32 {
    u32::try_from(value.as_u64().expect("an integer")).expect("fits u32")
}

#[test]
fn every_golden_row_formats_exactly_as_the_web_ui_does() {
    let path = fixtures_dir().join("display-format").join("cases.json");
    let text = std::fs::read_to_string(&path).expect("display-format/cases.json readable");
    let golden: Value = serde_json::from_str(&text).expect("display-format/cases.json is JSON");
    let styles: Vec<(String, AmountStyle)> = golden["styles"]
        .as_array()
        .expect("styles")
        .iter()
        .map(|entry| {
            (
                entry["commodity"].as_str().expect("commodity").to_string(),
                style_of(&entry["style"]),
            )
        })
        .collect();
    let cases = golden["cases"].as_array().expect("cases");
    assert!(cases.len() >= 2_000, "the display golden lost its rows");

    let failures: Vec<String> = cases
        .iter()
        .filter_map(|row| {
            let mantissa: i128 = row[0]
                .as_str()
                .expect("mantissa")
                .parse()
                .expect("integer mantissa");
            let qty = Dec::new(mantissa, as_u32(&row[1]));
            let (commodity, style) = &styles[as_u32(&row[2]) as usize];
            let max = as_u32(&row[3]);
            let got = (
                display_places(qty, style.precision, max),
                format_dec(qty, style, max),
                format_amount(commodity, qty, style, max),
            );
            let want = (
                as_u32(&row[4]),
                row[5].as_str().expect("formatDec").to_string(),
                row[6].as_str().expect("formatAmount").to_string(),
            );
            (got != want).then(|| format!("{row}: got {got:?}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} rows differ from the TypeScript formatter; first few:\n{}",
        failures.len(),
        cases.len(),
        failures
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
