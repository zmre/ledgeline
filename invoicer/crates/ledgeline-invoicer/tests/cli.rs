//! The binary as a user runs it.

use std::process::Command;

fn run(arg: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_ledgeline-invoicer"))
        .arg(arg)
        .output()
        .expect("the ledgeline-invoicer binary runs");
    assert!(output.status.success(), "`{arg}` exits 0: {output:?}");
    String::from_utf8(output.stdout).expect("stdout is UTF-8")
}

#[test]
fn version_and_help_name_the_app() {
    assert_eq!(
        run("--version"),
        format!("ledgeline-invoicer {}\n", env!("CARGO_PKG_VERSION"))
    );
    let help = run("--help");
    assert!(help.starts_with("Ledgeline Invoicer"), "{help}");
    assert!(help.contains("Usage: ledgeline-invoicer"), "{help}");
}
