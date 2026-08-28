//! What `cargo add zyris` asks of the machine it is built on.
//!
//! The promise is narrow and worth stating exactly: depending on this crate compiles on a box with
//! a Rust toolchain and nothing else — no C compiler, no `cmake`, no system packages. Everything
//! short of dialling works there. Dialling needs a TLS provider, both providers compile C, and
//! that price is charged only when one of them is named.
//!
//! It is a promise about the *resolved* graph, and one feature edit undoes it silently: `reqwest`'s
//! `rustls` feature expands to `aws-lc-rs`, and until this was measured that is exactly what
//! `enroll` pulled — a C compiler and `cmake`, for a feature that is about enrolment.
//!
//! `cargo tree` rather than `cargo metadata`, and the difference is the whole test. `metadata`'s
//! resolve is the union of every optional dependency, so it reports `ring` and `cc` under default
//! features when neither is built; `tree` answers with what the features actually reach. Measured
//! on 2026-08-26: metadata says 129 crates including `cc` and `ring`, `tree --target all` reaches
//! neither. `crate_boundary.rs` keeps using `metadata` on purpose — a union is the conservative
//! answer to *"could this ever link capkit"*, and the wrong answer to *"what does this build".*

use std::process::Command;

/// Whether a build of this crate with `features` on reaches `package`.
///
/// `--target all` because the promise is about a bare Linux, macOS *or* Windows box, and a
/// dependency that only opens on one of them would otherwise go unseen from here. `normal,build`
/// because a build script that compiles C costs the same toolchain a normal dependency would;
/// dev edges are not what a consumer builds.
fn reaches(features: &[&str], package: &str) -> bool {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let mut command = Command::new(env!("CARGO"));
    command.args([
        "tree",
        "--manifest-path",
        manifest,
        "-e",
        "normal,build",
        "--target",
        "all",
        "-i",
        package,
    ]);
    if !features.is_empty() {
        command.args(["--features", &features.join(",")]);
    }

    let output = command.output().expect("cargo is what is running this test, so cargo is on PATH");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Two ways to be absent: in the lockfile but unreachable ("nothing to print"), or not resolved
    // at all ("did not match any packages"). Anything else has to be a real tree, and a real tree
    // that named nothing would mean this test stopped testing.
    if said.contains("nothing to print") || said.contains("did not match any packages") {
        return false;
    }
    assert!(
        said.contains(package),
        "cargo tree neither found {package} nor said it could not; this test read nothing:\n{said}"
    );
    true
}

/// Every crate that means "this build shells out to a C compiler". `cc` and `cmake` are the two
/// ways it is done; the other three are named separately so a failure says which door opened.
const NEEDS_A_TOOLCHAIN: [&str; 5] = ["cc", "cmake", "ring", "aws-lc-sys", "openssl-sys"];

#[test]
fn depending_on_the_face_asks_for_no_c_compiler() {
    // The walk has to have walked. `zyris-proto` is two hops out, so its absence would mean
    // `reaches` is answering `false` to everything and the assertions below prove nothing.
    assert!(
        reaches(&[], "zyris-proto"),
        "the default build does not reach the wire format, so this test is measuring nothing"
    );

    // `caps` is declarations — serde types and nothing else — so it is free. `p2p` is not on this
    // list and deliberately so: iroh's default `tls-ring` reaches `ring`, and that is a property of
    // wanting QUIC rather than something this crate chose. `enroll` has its own test below.
    for features in [&[][..], &["caps"][..]] {
        for package in NEEDS_A_TOOLCHAIN {
            assert!(
                !reaches(features, package),
                "`cargo add zyris`{} now pulls `{package}`, so it no longer builds on a machine \
                 with only a Rust toolchain. `cargo tree -p zyris --target all -i {package}{}` \
                 names the path.",
                described(features),
                described(features),
            );
        }
    }
}

/// What `enroll` costs, stated rather than assumed.
///
/// Enrolment is an HTTPS device-grant flow, so it needs TLS and TLS needs a provider and every
/// provider compiles C. That much is unavoidable and this test does not pretend otherwise. What it
/// holds is *which* provider: `ring`, which wants a `cc`, and **not** `aws-lc-rs`, which wants
/// `cmake` as well off its nine named targets. Until 2026-08-27 this was the other way round —
/// `reqwest`'s `rustls` feature expands to `aws-lc-rs`, so a feature about enrolment was choosing
/// the heavier cryptography for the whole graph, silently, and nothing said so.
#[test]
fn enrolling_costs_ring_and_not_the_heavier_one() {
    assert!(
        reaches(&["enroll"], "ring") && reaches(&["enroll"], "cc"),
        "`enroll` no longer reaches ring, so either it stopped implying `tls-ring` or a provider \
         appeared that needs no C — find out which before deleting this test"
    );
    for package in ["aws-lc-sys", "aws-lc-rs", "cmake"] {
        assert!(
            !reaches(&["enroll"], package),
            "`cargo add zyris --features enroll` pulls `{package}` again, so enrolment is back to \
             deciding the whole graph's cryptography. Check `reqwest`'s feature in \
             crates/zyris-core/Cargo.toml — `rustls` means aws-lc, `rustls-no-provider` does not."
        );
    }
}

/// How a feature list reads in a failure message.
fn described(features: &[&str]) -> String {
    if features.is_empty() {
        String::new()
    } else {
        format!(" --features {}", features.join(","))
    }
}

/// The other side of the same promise, and what stops the test above from being vacuous: the
/// toolchain is not gone, it is *priced*. If naming a provider stopped costing a C compiler, one
/// of these crates would have grown a pure-Rust provider — a thing worth noticing rather than
/// discovering later.
#[test]
fn naming_a_tls_provider_is_what_costs_a_c_compiler() {
    assert!(
        reaches(&["tls-ring"], "ring") && reaches(&["tls-ring"], "cc"),
        "`tls-ring` no longer reaches `ring`, so either the feature stopped working or rustls \
         grew a provider that needs no C — check which before deleting this test"
    );
    assert!(
        reaches(&["tls-aws-lc"], "aws-lc-sys"),
        "`tls-aws-lc` no longer reaches `aws-lc-sys`, so the feature is not selecting the \
         provider it names"
    );
}

/// What `full` costs, because the comment on that feature used to say it cost nothing.
///
/// `full` is `caps + enroll + p2p`, and `enroll` is `["dep:reqwest", "tls-ring"]` one layer down —
/// so `full` names a TLS provider whatever else it does. The sentence that said this feature "does
/// not get to make [that] choice for you" was false from the day `enroll` grew its provider, and it
/// was false in the direction that costs a reader a failed build: someone wanting the
/// toolchain-free graph reads `full` as *more of the same* rather than as the line that crosses it.
/// `default` is that graph, and the test above is what holds it.
#[test]
fn full_pays_for_a_c_compiler_and_the_cheaper_provider() {
    assert!(
        reaches(&["full"], "ring") && reaches(&["full"], "cc"),
        "`full` no longer reaches a C toolchain. That is a real change and a welcome one, but the \
         comment on `full` in this crate's Cargo.toml describes the old cost — move it before \
         deleting this test"
    );
    // Still the cheaper of the two. `enroll` and `p2p` both reach rustls, and either one switching
    // to aws-lc would put `cmake` in front of every consumer who typed `--features full`.
    for package in ["aws-lc-sys", "cmake"] {
        assert!(
            !reaches(&["full"], package),
            "`cargo add zyris --features full` pulls `{package}`, so the whole feature now wants \
             `cmake` off aws-lc-sys's nine named targets. `cargo tree -p zyris --features full \
             --target all -i {package}` names the path."
        );
    }
}
