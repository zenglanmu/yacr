//! Unit tests.

use super::*;

fn request(key: &str) -> ResourceRequest {
    ResourceRequest {
        document: DocumentId(1),
        key: ResourceKey::sanitize(key),
        kind: ResourceKind::FontShx,
    }
}

#[test]
fn rejects_absolute_and_traversal_references() {
    assert!(!is_safe_reference("/etc/passwd"));
    assert!(!is_safe_reference("C:\\Windows\\font.ttf"));
    assert!(!is_safe_reference("../../secret.ttf"));
    assert!(!is_safe_reference("http://example.com/x.shx"));
    assert!(is_safe_reference("romans.shx"));
    assert!(is_safe_reference("fonts/simplex.shx"));
}

#[test]
fn sanitize_strips_directories_and_lowercases() {
    assert_eq!(bare_name("fonts/simplex.shx"), "simplex.shx");
    assert_eq!(
        ResourceKey::sanitize("dir/Romans.SHX").as_str(),
        "romans.shx"
    );
}

#[test]
fn chain_follows_priority_order() {
    let mut user = MapResolver::new(ResourceLimits::default());
    user.grant("romans.shx", Arc::from(b"user".to_vec()), "user pack")
        .unwrap();
    let mut bundled = MapResolver::new(ResourceLimits::default());
    bundled
        .grant("romans.shx", Arc::from(b"bundled".to_vec()), "bundled")
        .unwrap();
    let empty = MapResolver::new(ResourceLimits::default());
    let chain = ResolverChain {
        user_pack: Some(&user),
        document_map: Some(&empty),
        bundled: Some(&bundled),
    };
    let data = chain.resolve(&request("romans.shx")).unwrap();
    assert_eq!(&*data.bytes, b"user");
}

#[test]
fn missing_resource_is_reported_not_faked() {
    let empty = MapResolver::new(ResourceLimits::default());
    assert!(empty.resolve(&request("missing.shx")).is_err());
    assert!(PendingResourceResolver.resolve(&request("x.shx")).is_err());
}

#[test]
fn unsafe_grant_is_rejected() {
    let mut r = MapResolver::new(ResourceLimits::default());
    assert!(r
        .grant("../evil.shx", Arc::from(b"x".to_vec()), "x")
        .is_err());
}

const CATALOG: &str = r#"[
        { "file": "simplex.shx", "name": ["simplex"], "type": "shx" },
        { "file": "@extfont2.shx", "name": ["@extfont2"], "type": "shx", "encoding": "shift-jis" },
        { "file": "AMGDT DWE Edits.shx", "name": ["AMGDT DWE Edits"], "type": "shx" },
        { "file": "simsun.woff", "name": ["SimSun", "宋体"], "type": "mesh" }
    ]"#;

#[test]
fn font_catalog_parses_and_indexes_by_name_file_and_stem() {
    let catalog = FontCatalog::from_json(CATALOG).unwrap();
    assert_eq!(catalog.len(), 4);
    // By declared name, case-insensitively and with a directory prefix.
    assert_eq!(catalog.get("Simplex").unwrap().file, "simplex.shx");
    assert_eq!(
        catalog.get("fonts/SIMPLEX.SHX").unwrap().file,
        "simplex.shx"
    );
    // By file name.
    assert_eq!(catalog.get("simsun.woff").unwrap().kind, FontKind::Mesh);
    // By file stem.
    assert_eq!(catalog.get("simsun").unwrap().file, "simsun.woff");
    // Unknown fonts resolve to nothing rather than a fabricated face.
    assert!(catalog.get("no-such-font").is_none());
}

#[test]
fn font_catalog_records_kind_and_encoding() {
    let catalog = FontCatalog::from_json(CATALOG).unwrap();
    let ext = catalog.get("@extfont2").unwrap();
    assert_eq!(ext.kind, FontKind::Shx);
    assert_eq!(ext.encoding.as_deref(), Some("shift-jis"));
    let sun = catalog.get("宋体").unwrap();
    assert_eq!(sun.kind, FontKind::Mesh);
}

#[test]
fn font_urls_encode_spaces_and_default_to_the_mlightcad_catalog() {
    let catalog = FontCatalog::from_json(CATALOG).unwrap();
    let face = catalog.get("AMGDT DWE Edits").unwrap();
    assert_eq!(
        default_font_url(face),
        "https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts/AMGDT%20DWE%20Edits.shx"
    );
    let plain = catalog.get("simplex").unwrap();
    assert_eq!(
        face_url("https://example.com/f/", plain),
        "https://example.com/f/simplex.shx"
    );
}

#[test]
fn malformed_font_catalog_is_rejected() {
    assert!(FontCatalog::from_json("not json").is_err());
    assert!(FontCatalog::from_json("{}").is_err());
    // Missing/blank file entries are skipped, not panicked on.
    let catalog = FontCatalog::from_json(r#"[{"name":["x"]},{"file":""}]"#).unwrap();
    assert!(catalog.is_empty());
}

#[test]
fn font_plan_maps_requests_to_urls_and_dedups_files() {
    let catalog = FontCatalog::from_json(CATALOG).unwrap();
    let requested = vec![
        "SimSun".to_string(),
        "宋体".to_string(),
        "simplex.shx".to_string(),
        "missing.ttf".to_string(),
    ];
    let plan = plan_fonts(&catalog, &requested, DEFAULT_FONT_BASE_URL);
    // SimSun and 宋体 share one file; simplex resolves; missing drops.
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].file, "simsun.woff");
    assert_eq!(plan[0].request, "SimSun");
    assert!(plan[0].url.starts_with(DEFAULT_FONT_BASE_URL));
    assert_eq!(plan[1].file, "simplex.shx");
    assert_eq!(plan[1].kind, FontKind::Shx);
}

#[test]
fn grant_enforces_the_running_total_budget() {
    let limits = ResourceLimits {
        max_bytes: 1024,
        total_bytes: 100,
        ..ResourceLimits::default()
    };
    let mut resolver = MapResolver::new(limits);
    resolver
        .grant("a.shx", Arc::from(vec![0u8; 60]), "pack")
        .unwrap();
    assert_eq!(resolver.used_bytes(), 60);
    // A second grant that would push the total over 100 is rejected with a
    // structured over-budget issue, not silently dropped.
    let issue = resolver
        .grant("b.shx", Arc::from(vec![0u8; 60]), "pack")
        .unwrap_err();
    assert_eq!(issue.code, codes::RESOURCE_OVER_BUDGET);
    assert_eq!(issue.budget, Some(ResourceBudget::TotalBytes));
    assert_eq!(issue.actual, 120);
    assert_eq!(issue.limit, 100);
    // The rejected grant did not consume budget.
    assert_eq!(resolver.used_bytes(), 60);
    // A later grant that fits still succeeds.
    resolver
        .grant("c.shx", Arc::from(vec![0u8; 40]), "pack")
        .unwrap();
    assert_eq!(resolver.used_bytes(), 100);
}

#[test]
fn replacing_a_resource_only_charges_the_resulting_total() {
    let mut resolver = MapResolver::new(ResourceLimits {
        max_bytes: 100,
        total_bytes: 100,
        ..ResourceLimits::default()
    });
    resolver
        .grant("a.shx", Arc::from(vec![1u8; 60]), "original")
        .unwrap();
    resolver
        .grant("b.shx", Arc::from(vec![2u8; 40]), "other")
        .unwrap();
    // Directory and case aliases resolve to the same granted entry.
    resolver
        .grant("fonts/A.SHX", Arc::from(vec![3u8; 60]), "replacement")
        .unwrap();
    assert_eq!(resolver.used_bytes(), 100);
    assert_eq!(resolver.len(), 2);
    let data = resolver.resolve(&request("a.shx")).unwrap();
    assert_eq!(&*data.bytes, &[3u8; 60]);
    assert_eq!(data.license_hint.as_deref(), Some("replacement"));
    resolver
        .grant("a.shx", Arc::from(vec![4u8; 20]), "smaller")
        .unwrap();
    assert_eq!(resolver.used_bytes(), 60);
    resolver
        .grant("a.shx", Arc::from(vec![5u8; 60]), "larger")
        .unwrap();
    assert_eq!(resolver.used_bytes(), 100);
    let issue = resolver
        .grant("a.shx", Arc::from(vec![6u8; 61]), "rejected")
        .unwrap_err();
    assert_eq!(issue.budget, Some(ResourceBudget::TotalBytes));
    assert_eq!(issue.actual, 101);
    assert_eq!(resolver.used_bytes(), 100);
    assert_eq!(resolver.len(), 2);
    let data = resolver.resolve(&request("a.shx")).unwrap();
    assert_eq!(&*data.bytes, &[5u8; 60]);
    assert_eq!(data.license_hint.as_deref(), Some("larger"));
    assert_eq!(
        &*resolver.resolve(&request("b.shx")).unwrap().bytes,
        &[2u8; 40]
    );
}

#[test]
fn chain_falls_back_only_for_missing_resources() {
    struct FailingResolver;
    impl ResourceResolver for FailingResolver {
        fn resolve(&self, _: &ResourceRequest) -> CadResult<ResourceData> {
            Err(CadError::Cancelled)
        }
    }
    let mut bundled = MapResolver::new(ResourceLimits::default());
    bundled
        .grant("romans.shx", Arc::from(b"bundled".to_vec()), "bundled")
        .unwrap();
    let missing = MapResolver::default();
    let chain = ResolverChain {
        user_pack: Some(&missing),
        document_map: None,
        bundled: Some(&bundled),
    };
    assert_eq!(
        &*chain.resolve(&request("romans.shx")).unwrap().bytes,
        b"bundled"
    );
    let failing = FailingResolver;
    for chain in [
        ResolverChain {
            user_pack: Some(&failing),
            document_map: None,
            bundled: Some(&bundled),
        },
        ResolverChain {
            user_pack: Some(&missing),
            document_map: Some(&failing),
            bundled: Some(&bundled),
        },
        ResolverChain {
            user_pack: Some(&missing),
            document_map: None,
            bundled: Some(&failing),
        },
    ] {
        assert!(matches!(
            chain.resolve(&request("romans.shx")),
            Err(CadError::Cancelled)
        ));
    }
    assert!(matches!(
        chain.resolve(&request("missing.shx")),
        Err(CadError::ResourceMissing(_))
    ));
}

#[test]
fn per_resource_and_pixel_budgets_are_explicit() {
    let limits = ResourceLimits {
        max_bytes: 10,
        max_image_pixels: 100,
        ..ResourceLimits::default()
    };
    let mut resolver = MapResolver::new(limits.clone());
    let issue = resolver
        .grant("big.shx", Arc::from(vec![0u8; 11]), "pack")
        .unwrap_err();
    assert_eq!(issue.budget, Some(ResourceBudget::PerResource));
    // Image decoding is capped before allocation.
    assert!(limits.check_image_pixels(10, 10).is_ok());
    let issue = limits.check_image_pixels(11, 10).unwrap_err();
    assert_eq!(issue.budget, Some(ResourceBudget::ImagePixels));
    assert_eq!(issue.kind, Some(ResourceKind::Image));
}

#[test]
fn image_pixel_overflow_is_rejected_even_with_the_maximum_budget() {
    for max_image_pixels in [100, u64::MAX] {
        let limits = ResourceLimits {
            max_image_pixels,
            ..ResourceLimits::default()
        };
        for (width, height) in [(u64::MAX, 2), (2, u64::MAX), (u64::MAX, u64::MAX)] {
            let issue = limits.check_image_pixels(width, height).unwrap_err();
            assert_eq!(issue.code, codes::RESOURCE_SIZE_OVERFLOW);
            assert_eq!(issue.kind, Some(ResourceKind::Image));
            assert_eq!(issue.budget, Some(ResourceBudget::ImagePixels));
            // The overflow code distinguishes this lower bound from an exact count.
            assert_eq!(issue.actual, u64::MAX);
            assert_eq!(issue.limit, max_image_pixels);
            assert_eq!(issue.key, None);
        }
    }
}

#[test]
fn image_pixel_budget_accepts_exact_representable_limits() {
    let limits = ResourceLimits {
        max_image_pixels: 100,
        ..ResourceLimits::default()
    };
    assert_eq!(limits.check_image_pixels(10, 10), Ok(100));
    let limits = ResourceLimits {
        max_image_pixels: u64::MAX,
        ..ResourceLimits::default()
    };
    assert_eq!(limits.check_image_pixels(u64::MAX, 1), Ok(u64::MAX));
    assert_eq!(limits.check_image_pixels(3, u64::MAX / 3), Ok(u64::MAX));
}

#[test]
fn image_pixel_budget_preserves_zero_dimension_behavior() {
    let limits = ResourceLimits {
        max_image_pixels: 0,
        ..ResourceLimits::default()
    };
    for (width, height) in [(0, u64::MAX), (u64::MAX, 0), (0, 0)] {
        assert_eq!(limits.check_image_pixels(width, height), Ok(0));
    }
}

#[test]
fn representable_image_pixel_over_budget_reports_the_exact_count() {
    let limits = ResourceLimits {
        max_image_pixels: 100,
        ..ResourceLimits::default()
    };
    let issue = limits.check_image_pixels(11, 10).unwrap_err();
    assert_eq!(issue.code, codes::RESOURCE_OVER_BUDGET);
    assert_eq!(issue.kind, Some(ResourceKind::Image));
    assert_eq!(issue.budget, Some(ResourceBudget::ImagePixels));
    assert_eq!(issue.actual, 110);
    assert_eq!(issue.limit, 100);
    assert_eq!(issue.key, None);
}

#[test]
fn xref_recursion_limit_is_enforced() {
    let limits = ResourceLimits {
        max_xref_depth: 2,
        ..ResourceLimits::default()
    };
    assert!(limits.check_xref_depth(Some("base.dwg"), 2).is_ok());
    let issue = limits.check_xref_depth(Some("base.dwg"), 3).unwrap_err();
    assert_eq!(issue.code, codes::RESOURCE_RECURSION_LIMIT);
    assert_eq!(issue.budget, Some(ResourceBudget::XrefDepth));
    assert_eq!(issue.actual, 3);
    assert_eq!(issue.limit, 2);
}

#[test]
fn capability_table_does_not_claim_unsupported_categories() {
    let table = resource_capabilities();
    // Every category is described exactly once.
    for kind in [
        ResourceKind::FontTtf,
        ResourceKind::FontShx,
        ResourceKind::BigFont,
        ResourceKind::Image,
        ResourceKind::ExternalReference,
    ] {
        assert_eq!(
            table.iter().filter(|c| c.kind == kind).count(),
            1,
            "missing capability for {}",
            kind.as_str()
        );
    }
    // The old model implied every kind was supported. Image and xref are
    // not implemented and must say so.
    assert_eq!(
        resource_capability(ResourceKind::Image).decode,
        SupportStatus::NotImplemented
    );
    assert_eq!(
        resource_capability(ResourceKind::ExternalReference).resolve,
        SupportStatus::NotImplemented
    );
    assert_eq!(
        resource_capability(ResourceKind::BigFont).decode,
        SupportStatus::NotImplemented
    );
    // Fonts are resolvable but not yet verified as fully decoded.
    assert_ne!(
        resource_capability(ResourceKind::FontTtf).decode,
        SupportStatus::Verified
    );
}

#[test]
fn font_plan_report_accounts_for_unresolved_and_unsupported() {
    let catalog = FontCatalog::from_json(CATALOG).unwrap();
    // Extend the catalog with an unknown-technology entry: it must not be
    // planned as if it were usable.
    let catalog_with_woff2 = FontCatalog::from_json(
        r#"[
                { "file": "simplex.shx", "name": ["simplex"], "type": "shx" },
                { "file": "modern.woff2", "name": ["modern"], "type": "woff2" }
            ]"#,
    )
    .unwrap();
    let requested = vec![
        "simplex".to_string(),
        "modern".to_string(),
        "ghost.ttf".to_string(),
    ];
    let report = plan_fonts_report(&catalog_with_woff2, &requested, DEFAULT_FONT_BASE_URL);
    assert_eq!(report.planned.len(), 1);
    assert_eq!(report.planned[0].file, "simplex.shx");
    assert_eq!(report.unresolved, vec!["ghost.ttf".to_string()]);
    assert_eq!(report.unsupported, vec!["modern".to_string()]);
    assert!(!report.is_complete());
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == codes::FONT_UNRESOLVED));
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == codes::FONT_UNSUPPORTED));
    // The legacy `plan_fonts` still returns just the planned faces.
    assert_eq!(
        plan_fonts(&catalog, &requested, DEFAULT_FONT_BASE_URL).len(),
        1
    );
}
