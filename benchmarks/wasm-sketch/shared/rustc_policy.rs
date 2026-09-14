//! Actual upstream parser fixtures shared by both runtime candidates.
use zccache_compiler::{
    detect_family, parse_rustc_plan_with_syntax, CompilerFamily, RustcHost, RustcOutputPlan,
    RustcPathSyntax, RustcPlan,
};

pub fn proof() -> bool {
    if detect_family(r"C:\tools\rustc.exe") != CompilerFamily::Rustc {
        return false;
    }
    for (host, expected) in [
        (RustcHost::Linux, "libfixture.so"),
        (RustcHost::Macos, "libfixture.dylib"),
        (RustcHost::Windows, "fixture.dll"),
    ] {
        if !output_and_dylint(host) {
            return false;
        }
        let args = [
            "fixture.rs",
            "--crate-type=proc-macro",
            "--target=wasm32-unknown-unknown",
        ]
        .map(String::from);
        let plan = parse_rustc_plan_with_syntax("rustc", &args, host, false, RustcPathSyntax::Unix);
        if !matches!(plan, RustcPlan::Cacheable { output: RustcOutputPlan::Explicit(ref path), .. } if path == expected)
        {
            return false;
        }
        for (syntax, expected) in [
            (RustcPathSyntax::Unix, r"libC:\src\fixture.rlib"),
            (RustcPathSyntax::Windows, "libfixture.rlib"),
        ] {
            let args = [r"C:\src\fixture.rs", "--crate-type=rlib"].map(String::from);
            let plan = parse_rustc_plan_with_syntax("rustc", &args, host, false, syntax);
            if !matches!(plan, RustcPlan::Cacheable { output: RustcOutputPlan::Explicit(ref path), .. } if path == expected)
            {
                return false;
            }
        }
        let args = ["fixture.rs", "--test"].map(String::from);
        for opt_in in [false, true] {
            let plan =
                parse_rustc_plan_with_syntax("rustc", &args, host, opt_in, RustcPathSyntax::Unix);
            if matches!(plan, RustcPlan::Cacheable { .. }) != opt_in {
                return false;
            }
        }
    }
    true
}

fn output_and_dylint(host: RustcHost) -> bool {
    let args = [
        "fixture.rs",
        "--crate-name=fixture",
        "--crate-type=rlib",
        "--emit=metadata,dep-info",
        "--out-dir",
        "build/../out",
        "--future-policy-flag",
    ]
    .map(String::from);
    let expected = RustcPlan::Cacheable {
        source: "fixture.rs".into(),
        output: RustcOutputPlan::InDirectory {
            directory: "build/../out".into(),
            filename: "libfixture.rmeta".into(),
        },
        unknown_flags: vec!["--future-policy-flag".into()],
    };
    if parse_rustc_plan_with_syntax("rustc", &args, host, false, RustcPathSyntax::Unix) != expected
    {
        return false;
    }
    let args = [
        "rustc",
        "fixture.rs",
        "--crate-name=fixture",
        "--crate-type=cdylib",
        "--out-dir",
        "target/dylint/libraries",
        "-C",
        "linker=dylint-link",
    ]
    .map(String::from);
    let plan =
        parse_rustc_plan_with_syntax("dylint-driver", &args, host, false, RustcPathSyntax::Unix);
    if matches!(plan, RustcPlan::Cacheable { .. }) != (host != RustcHost::Windows) {
        return false;
    }
    let malformed = ["not-rustc", "fixture.rs"].map(String::from);
    matches!(
        parse_rustc_plan_with_syntax(
            "dylint-driver",
            &malformed,
            host,
            false,
            RustcPathSyntax::Unix
        ),
        RustcPlan::NonCacheable { .. }
    )
}
