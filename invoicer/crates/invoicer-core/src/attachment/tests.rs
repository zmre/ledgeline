use super::*;
use crate::notes::SiteConfig;
use ledgeline_core::vault::MemVault;
use proptest::prelude::*;

#[test]
fn entries_parse_into_their_three_kinds() {
    assert_eq!(
        Attachment::parse(" [[Customer X MSA 2026]] "),
        Ok(Attachment::Note {
            link: "[[Customer X MSA 2026]]".into()
        })
    );
    assert_eq!(
        Attachment::parse("/customers/contracts/SOW 3.pdf"),
        Ok(Attachment::File {
            path: "/customers/contracts/SOW 3.pdf".into()
        })
    );
    assert_eq!(
        Attachment::parse("contracts/SOW 3.pdf"),
        Ok(Attachment::File {
            path: "contracts/SOW 3.pdf".into()
        })
    );
    assert_eq!(
        Attachment::parse("~/Documents/Customer X/MSA.pdf"),
        Ok(Attachment::Outside(OutsidePath::Home(
            "Documents/Customer X/MSA.pdf".into()
        )))
    );
    assert_eq!(
        Attachment::parse("file:///Volumes/Share/Customer%20X%20%23%201/MSA.pdf"),
        Ok(Attachment::Outside(OutsidePath::Absolute(PathBuf::from(
            "/Volumes/Share/Customer X # 1/MSA.pdf"
        ))))
    );
    assert_eq!(
        Attachment::parse("FILE://localhost/tmp/MSA.pdf"),
        Ok(Attachment::Outside(OutsidePath::Absolute(PathBuf::from(
            "/tmp/MSA.pdf"
        ))))
    );
}

#[test]
fn entries_that_dont_read_say_why() {
    assert_eq!(Attachment::parse("  "), Err(AttachmentError::Empty));
    assert!(matches!(
        Attachment::parse("a\nb"),
        Err(AttachmentError::NotOneLine(_))
    ));
    assert!(matches!(
        Attachment::parse("file://server/share/MSA.pdf"),
        Err(AttachmentError::BadFileUrl(_))
    ));
    assert!(matches!(
        Attachment::parse("~/../../etc/passwd"),
        Err(AttachmentError::ClimbsOut(_))
    ));
    assert!(matches!(
        Attachment::parse("~/"),
        Err(AttachmentError::ClimbsOut(_))
    ));
    // Read from a note, a bad one is kept (as a books path) so `check`
    // reports it; an empty one is nothing.
    assert_eq!(
        Attachment::read("file://server/x.pdf"),
        Some(Attachment::File {
            path: "file://server/x.pdf".into()
        })
    );
    assert_eq!(Attachment::read(""), None);
}

#[test]
fn an_outside_file_is_written_from_home_or_as_a_url() {
    let home = Path::new("/Users/pat");
    let under = Attachment::outside(Path::new("/Users/pat/Documents/MSA 2026.pdf"), Some(home));
    assert_eq!(under.entry(), "~/Documents/MSA 2026.pdf");
    assert_eq!(under.name(), "MSA 2026.pdf");
    let elsewhere = Attachment::outside(Path::new("/Volumes/Share/Ünïcode #1?.pdf"), Some(home));
    assert_eq!(
        elsewhere.entry(),
        "file:///Volumes/Share/%C3%9Cn%C3%AFcode%20%231%3F.pdf"
    );
    assert_eq!(elsewhere.name(), "Ünïcode #1?.pdf");
    // A hidden folder is fine outside the books.
    assert_eq!(
        Attachment::outside(Path::new("/Users/pat/.private/PO.pdf"), Some(home)).entry(),
        "~/.private/PO.pdf"
    );
    // No home folder known: a URL.
    assert!(
        Attachment::outside(Path::new("/Users/pat/MSA.pdf"), None)
            .entry()
            .starts_with("file:///")
    );
    // A sibling of the home folder isn't under it.
    assert!(
        Attachment::outside(Path::new("/Users/patrick/MSA.pdf"), Some(home))
            .entry()
            .starts_with("file:///")
    );
}

#[test]
fn names_never_show_syntax() {
    let name = |s: &str| Attachment::parse(s).expect("reads").name();
    assert_eq!(name("[[MSA|the MSA]]"), "MSA");
    assert_eq!(name("/c/SOW 3.pdf"), "SOW 3.pdf");
    assert_eq!(name("~/Docs/PO 7.pdf"), "PO 7.pdf");
    assert_eq!(name("file:///tmp/PO%207.pdf"), "PO 7.pdf");
}

#[test]
fn outside_files_resolve_against_the_home_folder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    std::fs::create_dir_all(home.join("Docs")).expect("dir");
    std::fs::write(home.join("Docs/MSA.pdf"), b"%PDF-").expect("writes");
    let vault = MemVault::new().with_file("customers/X.md", "---\ntype: organization\n---\n");
    let site = Site::open(&vault, SiteConfig::default());
    let from = VaultPath::new("customers/X.md").expect("path");
    let there = Attachment::parse("~/Docs/MSA.pdf").expect("reads");
    assert_eq!(
        there.resolve(&from, &site, &vault, Some(home)),
        Some(AttachmentTarget::Outside(home.join("Docs/MSA.pdf")))
    );
    assert_eq!(there.resolve(&from, &site, &vault, None), None, "no home");
    let missing = Attachment::parse("~/Docs/gone.pdf").expect("reads");
    assert_eq!(missing.resolve(&from, &site, &vault, Some(home)), None);
    let url = Attachment::outside(&home.join("Docs/MSA.pdf"), None);
    let view = url.view(&from, &site, &vault, Some(home));
    assert_eq!(view.name, "MSA.pdf");
    assert!(view.found.is_some());
    let json = serde_json::to_value(&view).expect("json");
    assert_eq!(json["kind"], "outside");
    assert_eq!(json["path"], url.entry());
    assert_eq!(
        json["location"],
        home.join("Docs/MSA.pdf").display().to_string()
    );
}

/// One path segment: letters, spaces, punctuation URLs encode, non-ASCII.
fn segment() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 ._#%?&=+,;@!$'()\\[\\]{}~ÀÉÎõüßñ中文😀-]{1,12}"
        .prop_filter("a real name", |s| s != "." && s != ".." && s.trim() == s)
}

proptest! {
    #[test]
    fn outside_entries_round_trip(parts in proptest::collection::vec(segment(), 1..5), under_home: bool) {
        let home = Path::new("/Users/someone");
        let base = if under_home { home.to_path_buf() } else { PathBuf::from("/Volumes/Share") };
        let file = parts.iter().fold(base, |p, s| p.join(s));
        let entry = Attachment::outside(&file, Some(home));
        let written = entry.entry();
        prop_assert!(!written.contains('\n'));
        let back = Attachment::parse(&written).expect("its own entry reads");
        prop_assert_eq!(&back, &entry);
        let Attachment::Outside(outside) = back else { panic!("outside") };
        prop_assert_eq!(outside.expand(Some(home)), Some(file));
        prop_assert_eq!(entry.name(), parts.last().cloned().unwrap_or_default());
    }

    #[test]
    fn books_entries_round_trip(parts in proptest::collection::vec(segment(), 1..4)) {
        let written = format!("/{}", parts.join("/"));
        prop_assume!(!written.starts_with("/~") && !parts[0].starts_with("[["));
        let entry = Attachment::parse(&written).expect("reads");
        prop_assert_eq!(entry.entry(), written.trim().to_string());
    }
}
