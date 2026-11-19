//! Tests for SemVer versions and ranges (MOD_PACKAGE.md section 5).

use efemod::{Version, VersionReq};

fn v(s: &str) -> Version {
    s.parse().unwrap_or_else(|e| panic!("version {s:?} should parse: {e}"))
}

fn req(s: &str) -> VersionReq {
    s.parse().unwrap_or_else(|e| panic!("requirement {s:?} should parse: {e}"))
}

/// Checks `req` against `(version, expected)` rows.
fn check(req_text: &str, rows: &[(&str, bool)]) {
    let r = req(req_text);
    for &(ver, want) in rows {
        assert_eq!(r.matches(&v(ver)), want, "req {req_text:?} vs version {ver:?}");
    }
}

#[test]
fn valid_versions_round_trip() {
    let long_ok = format!("1.0.0-{}", "a".repeat(58));
    let cases = [
        "0.0.0",
        "1.2.3",
        "10.20.30",
        "4294967295.4294967295.4294967295",
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-0A.is.legal",
        "1.0.0-x-y-z.--",
        "1.0.0-0",
        "1.0.0-rc.1",
        "1.0.0-18446744073709551615",
        "1.0.0-a.b.c.d.e.f.g.h",
        long_ok.as_str(),
    ];
    for case in cases {
        let parsed = v(case);
        assert_eq!(parsed.to_string(), case, "Display must round-trip");
        assert_eq!(v(&parsed.to_string()), parsed);
    }
}

#[test]
fn invalid_versions() {
    let too_long = format!("1.0.0-{}", "a".repeat(59));
    let cases: Vec<&str> = vec![
        "",
        " ",
        "1",
        "1.2",
        "1.2.3.4",
        "01.2.3",
        "1.02.3",
        "1.2.03",
        "1.2.3+build",
        "+1.2.3",
        "1.2.3-",
        "1.2.3-alpha..1",
        "1.2.3-.a",
        "1.2.3-a.",
        "1.2.3-01",
        "1.2.3-01.5",
        "1.2.3-alpha_1",
        "1.2.3-é",
        "1.2.3-a b",
        "1.2.3-\t",
        "1.2.3 ",
        " 1.2.3",
        "1..3",
        "1.2.+3",
        "1.-2.3",
        "-1.2.3",
        "v1.2.3",
        "a.b.c",
        "4294967296.0.0",
        "1.0.0-99999999999999999999",
        "1.0.0-18446744073709551616",
        too_long.as_str(),
    ];
    for case in cases {
        assert!(case.parse::<Version>().is_err(), "version {case:?} must be rejected");
    }
}

#[test]
fn version_length_limit_is_bytes() {
    // 64 bytes is the limit, counted on the whole string.
    assert!(format!("1.0.0-{}", "a".repeat(58)).parse::<Version>().is_ok());
    assert!(format!("1.0.0-{}", "a".repeat(59)).parse::<Version>().is_err());
}

#[test]
fn precedence_chain_from_semver_spec() {
    let chain = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
    ];
    for pair in chain.windows(2) {
        let (lo, hi) = (v(pair[0]), v(pair[1]));
        assert!(lo < hi, "{} must be < {}", pair[0], pair[1]);
        assert!(hi > lo, "{} must be > {}", pair[1], pair[0]);
        assert_ne!(lo, hi);
    }
}

#[test]
fn precedence_across_releases_and_identifier_kinds() {
    let chain = [
        "0.0.1",
        "0.9.9",
        "1.0.0-0",
        "1.0.0-1",
        "1.0.0-9",
        "1.0.0-10",
        "1.0.0-18446744073709551615",
        "1.0.0-a",
        "1.0.0-a.0",
        "1.0.0-a.1",
        "1.0.0-b",
        "1.0.0",
        "1.0.1",
        "1.1.0",
        "1.10.0",
        "2.0.0-alpha",
        "2.0.0",
        "4294967295.0.0",
    ];
    for pair in chain.windows(2) {
        assert!(v(pair[0]) < v(pair[1]), "{} must be < {}", pair[0], pair[1]);
    }
}

#[test]
fn equal_versions_compare_equal() {
    assert_eq!(v("1.2.3-rc.1").cmp(&v("1.2.3-rc.1")), std::cmp::Ordering::Equal);
    assert_eq!(v("1.2.3").partial_cmp(&v("1.2.3")), Some(std::cmp::Ordering::Equal));
}

#[test]
fn version_accessors() {
    let release = v("3.4.5");
    assert_eq!((release.major, release.minor, release.patch), (3, 4, 5));
    assert!(release.pre().is_empty());
    assert_eq!(v("3.4.5-rc.1").pre().len(), 2);
}

#[test]
fn valid_requirements_parse() {
    let cases = [
        "*",
        " * ",
        "1",
        "1.2",
        "1.2.3",
        "=1.2.3",
        ">1",
        ">=1.2",
        "<1.2.3",
        "<=1",
        "~1.2",
        "^0.0.3",
        ">=1.2.3, <2",
        ">= 1.2 , < 1.5",
        "=1.2.3-rc.1",
        "^1.2.3-beta.1",
        "1.2.3,1.2.4,1.2.5,1.2.6",
    ];
    for case in cases {
        req(case);
    }
}

#[test]
fn invalid_requirements() {
    let cases = [
        "",
        " ",
        ",",
        "1,",
        ",1",
        "1,,2",
        ">",
        ">=",
        "=",
        "^",
        "~ ",
        "=>1",
        ">>1",
        "1 2",
        "v1.2.3",
        "01",
        "^01.0.0",
        "1.2.3.4",
        "1.2-beta",
        "1.2.3-",
        "1.2.3-a..b",
        "1.2.3 - 2.0.0",
        "1.2.3+b",
        "^1.2.3+b",
        ">=1.2.3+build",
        "1,2,3,4,5",
        ">1,>2,>3,>4,>5",
        // wildcards are not part of the format
        "1.x",
        "1.X",
        "1.*",
        "1.2.*",
        "*.1",
        "x",
        "^1.x",
        "*, 1",
        "\t1.2.3",
        "1.2.3\n",
    ];
    for case in cases {
        assert!(case.parse::<VersionReq>().is_err(), "requirement {case:?} must be rejected");
    }
}

#[test]
fn wildcard_error_suggests_caret_or_equals() {
    let e = "1.*".parse::<VersionReq>().unwrap_err().to_string();
    assert!(e.contains("wildcard"), "message was: {e}");
    assert!(e.contains("^1") && e.contains("=1"), "message was: {e}");
}

#[test]
fn comparator_limit_is_four() {
    assert!("1,2,3,4".parse::<VersionReq>().is_ok());
    assert!("1,2,3,4,5".parse::<VersionReq>().is_err());
}

#[test]
fn requirement_display_round_trips() {
    let cases = ["*", "^1", "=1.2", ">1.2.3", ">=1.2.3-alpha.1", "<=2.0.0", "~0.1, <0.2"];
    for case in cases {
        let r = req(case);
        assert_eq!(req(&r.to_string()), r, "round trip of {case:?} via {}", r);
    }
    assert_eq!(req("1.2.3").to_string(), "^1.2.3");
    assert_eq!(req("*").to_string(), "*");
}

#[test]
fn range_table_exact() {
    check("=1.2.3", &[("1.2.3", true), ("1.2.4", false), ("1.2.2", false), ("1.3.0", false)]);
    check("=1.2", &[("1.2.0", true), ("1.2.99", true), ("1.3.0", false), ("1.1.9", false)]);
    check("=1", &[("1.0.0", true), ("1.99.9", true), ("2.0.0", false), ("0.9.9", false)]);
}

#[test]
fn range_table_greater() {
    check(">1.2.3", &[("1.2.3", false), ("1.2.4", true), ("1.2.2", false), ("1.3.0", true)]);
    check(">1.2", &[("1.2.9", false), ("1.2.0", false), ("1.3.0", true), ("2.0.0", true)]);
    check(">1", &[("1.9.9", false), ("1.0.0", false), ("2.0.0", true)]);
}

#[test]
fn range_table_greater_equal() {
    check(">=1.2.3", &[("1.2.3", true), ("1.2.2", false), ("1.2.4", true)]);
    check(">=1.2", &[("1.2.0", true), ("1.1.9", false), ("1.2.5", true)]);
    check(">=1", &[("1.0.0", true), ("0.9.9", false), ("9.0.0", true)]);
}

#[test]
fn range_table_less() {
    check("<1.2.3", &[("1.2.2", true), ("1.2.3", false), ("1.2.4", false), ("1.1.9", true)]);
    check("<1.2", &[("1.1.99", true), ("1.2.0", false)]);
    check("<1", &[("0.9.9", true), ("1.0.0", false)]);
}

#[test]
fn range_table_less_equal() {
    check("<=1.2.3", &[("1.2.3", true), ("1.2.4", false), ("1.2.2", true)]);
    check("<=1.2", &[("1.2.99", true), ("1.3.0", false)]);
    check("<=1", &[("1.99.0", true), ("2.0.0", false)]);
}

#[test]
fn range_table_caret() {
    check("^1.2.3", &[("1.2.3", true), ("1.9.0", true), ("1.2.2", false), ("2.0.0", false)]);
    check("^1.2", &[("1.2.0", true), ("1.9.9", true), ("1.1.9", false), ("2.0.0", false)]);
    check("^1", &[("1.0.0", true), ("1.99.99", true), ("0.9.9", false), ("2.0.0", false)]);
    check("^0.2.3", &[("0.2.3", true), ("0.2.9", true), ("0.3.0", false), ("0.2.2", false)]);
    check("^0.2", &[("0.2.0", true), ("0.2.9", true), ("0.3.0", false), ("0.1.9", false)]);
    check("^0.0.3", &[("0.0.3", true), ("0.0.4", false), ("0.0.2", false), ("0.1.0", false)]);
    check("^0.0", &[("0.0.0", true), ("0.0.9", true), ("0.1.0", false)]);
    check("^0", &[("0.0.0", true), ("0.9.9", true), ("1.0.0", false)]);
}

#[test]
fn range_table_tilde() {
    check("~1.2.3", &[("1.2.3", true), ("1.2.9", true), ("1.3.0", false), ("1.2.2", false)]);
    check("~1.2", &[("1.2.0", true), ("1.2.9", true), ("1.3.0", false), ("1.1.9", false)]);
    check("~1", &[("1.0.0", true), ("1.9.9", true), ("2.0.0", false), ("0.9.9", false)]);
}

#[test]
fn bare_requirement_is_caret() {
    check("1.2.3", &[("1.9.0", true), ("2.0.0", false), ("1.2.2", false)]);
    check("0.2", &[("0.2.9", true), ("0.3.0", false)]);
}

#[test]
fn star_matches_every_release() {
    check("*", &[("0.0.0", true), ("99.0.0", true), ("1.0.0-rc.1", false)]);
    check(" * ", &[("1.0.0", true)]);
}

#[test]
fn multiple_comparators_are_and() {
    check(">=1.2.3, <1.5", &[("1.2.3", true), ("1.4.9", true), ("1.5.0", false), ("1.2.2", false)]);
    check(">= 1.2.3 ,<2", &[("1.2.3", true), ("1.9.9", true), ("2.0.0", false)]);
}

#[test]
fn bare_numbers_under_all_operators() {
    // `>=1`, `<1` and `<=1` are grammatical though the table omits them.
    check(">=1", &[("1.0.0", true), ("0.9.9", false), ("9.0.0", true)]);
    check("<1", &[("0.9.9", true), ("1.0.0", false)]);
    check("<=1", &[("1.99.0", true), ("2.0.0", false)]);
}

#[test]
fn pre_release_rule_examples() {
    check(
        "^1.2.0-beta.1",
        &[
            ("1.2.0-beta.2", true),
            ("1.2.0-beta.1", true),
            ("1.2.0-alpha", false),
            ("1.2.0", true),
            ("1.3.0-rc.1", false),
            ("1.2.1-beta.1", false),
            ("2.0.0", false),
        ],
    );
    check("^1.0", &[("1.3.0-rc.1", false), ("1.3.0", true)]);
    check("=1.2.3-rc.1", &[("1.2.3-rc.1", true), ("1.2.3-rc.2", false), ("1.2.3", false)]);
    check(
        ">=1.0.0-alpha, <2",
        &[("1.0.0-alpha.1", true), ("1.5.0-beta", false), ("1.0.0", true)],
    );
    check("~1.2.3-beta", &[("1.2.3-rc", true), ("1.2.3-alpha", false), ("1.2.4-beta.1", false), ("1.2.4", true)]);
    check(">1.2.3", &[("1.2.4-rc.1", false), ("1.2.3-rc.1", false)]);
    check(">=1", &[("1.0.0-rc.1", false)]);
}

#[test]
fn pre_release_full_precedence_inside_one_comparator_pair() {
    // A full comparator compares by SemVer precedence, so `<1.2.3` admits 1.2.3-beta
    // once `>=1.2.3-alpha` names the same release with a pre-release.
    check(
        ">=1.2.3-alpha, <1.2.3",
        &[("1.2.3-beta", true), ("1.2.3-alpha", true), ("1.2.3", false), ("1.2.3-0", false)],
    );
}

#[test]
fn pre_release_partial_comparators_follow_cargo() {
    // Cargo's semantics (semver crate, eval.rs): partial comparators never admit a
    // pre-release of the same MAJOR.MINOR, even when another comparator names it.
    check(">=1.2.5-rc.1, <=1.2", &[("1.2.5-rc.2", false)]);
    check(">=1.2.5-rc.1, =1.2", &[("1.2.5-rc.2", false)]);
    check(">=1.2.5-rc.1, <=1.2.5", &[("1.2.5-rc.2", true)]);
}

#[test]
fn caret_and_tilde_do_not_cross_majors_or_minors_for_pre_releases() {
    check("^1, <=2.0.0-rc.5", &[("2.0.0-rc.1", false)]);
    check("^1, <=2.0.0-rc.5", &[("1.9.0", true)]);
    check("^1.2.3, >=2.0.0-rc.1", &[("2.0.0-rc.2", false)]);
    check(">=2.0.0-rc.1, <3", &[("2.0.0-rc.2", true), ("2.0.0-rc.0", false)]);
}

#[test]
fn numeric_limits_are_accepted() {
    check("^4294967295", &[("4294967295.1.0", true), ("4294967295.0.0", true)]);
    check("<=4294967295.4294967295.4294967295", &[("4294967295.0.0", true)]);
    check("=0.0.0", &[("0.0.0", true)]);
}

/// Deterministic xorshift generator, so the never-panic loop needs no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const ALPHABET: &[char] = &[
    '0', '1', '2', '9', '.', '-', '+', '*', 'x', 'X', '^', '~', '<', '>', '=', ',', ' ', 'a', 'Z',
    'é', '\t',
];

const SEEDS: &[&str] = &[
    "1.2.3-alpha.1",
    "1.0.0-0A.is.legal",
    "^1.2",
    ">=1.0.0, <2",
    "~0.0.3",
    "*",
    "=1.2.3-rc.1",
    ">= 1.2 , < 1.5",
    "4294967295.0.0",
];

fn random_string(rng: &mut Rng) -> String {
    let len = rng.below(16);
    (0..len).map(|_| ALPHABET[rng.below(ALPHABET.len())]).collect()
}

fn mutate(rng: &mut Rng, seed: &str) -> String {
    let mut chars: Vec<char> = seed.chars().collect();
    match rng.below(3) {
        0 => {
            let at = rng.below(chars.len() + 1);
            chars.insert(at, ALPHABET[rng.below(ALPHABET.len())]);
        }
        1 => {
            if !chars.is_empty() {
                let at = rng.below(chars.len());
                chars.remove(at);
            }
        }
        _ => {
            if !chars.is_empty() {
                let at = rng.below(chars.len());
                chars[at] = ALPHABET[rng.below(ALPHABET.len())];
            }
        }
    }
    chars.into_iter().collect()
}

#[test]
fn never_panics_on_generated_strings() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut versions: Vec<Version> = Vec::new();
    let mut reqs: Vec<VersionReq> = Vec::new();

    for i in 0..20_000 {
        let input = if i % 2 == 0 {
            random_string(&mut rng)
        } else {
            let seed = SEEDS[rng.below(SEEDS.len())];
            mutate(&mut rng, seed)
        };
        if let Ok(parsed) = input.parse::<Version>() {
            assert_eq!(parsed.to_string().parse::<Version>(), Ok(parsed.clone()));
            if versions.len() < 400 {
                versions.push(parsed);
            }
        }
        if let Ok(parsed) = input.parse::<VersionReq>() {
            assert_eq!(parsed.to_string().parse::<VersionReq>(), Ok(parsed.clone()));
            if reqs.len() < 400 {
                reqs.push(parsed);
            }
        }
    }

    assert!(versions.len() > 50, "fuzzer should reach valid versions, got {}", versions.len());
    assert!(reqs.len() > 50, "fuzzer should reach valid requirements, got {}", reqs.len());

    for a in &versions {
        for b in &versions {
            assert_eq!(a.cmp(b).reverse(), b.cmp(a));
            assert_eq!(a.cmp(b) == std::cmp::Ordering::Equal, a == b);
        }
        for r in &reqs {
            let _ = r.matches(a);
        }
    }
}
