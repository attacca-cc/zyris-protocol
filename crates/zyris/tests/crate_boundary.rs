//! What the published face names is a promise, so the boundary is worth a test.
//!
//! `zyris-capkit` is not a library consumer's dependency: it is a set of reference implementations,
//! which is the program's business, and it carries a git fork of `enigo` that could not go to
//! crates.io even if it were. Leaving it out of the face is what makes the fork a non-issue — and
//! leaving it out is a one-line edit away from being undone by anyone adding "just one more
//! re-export".

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::process::Command;

// The JSON parser is the one this crate already re-exports. A test that exists to keep the face's
// dependency list short should not lengthen it in order to run, and `zyris::serde_json` costs
// nothing that `use zyris::Node` does not cost already.
use zyris::serde_json::Value;

fn file(relative: &str) -> String {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("this crate lives under crates/");
    let path = crates.join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// Every crate a consumer links when it depends on this one, by name.
///
/// This is the *resolved* graph, not the manifest. `cargo metadata` answers with what Cargo
/// actually picked, so a rename, a dependency added three crates further down, or a path that only
/// opens under some feature all move this set. A grep over two files moved for none of them.
///
/// `--all-features`, because the promise is about every way this crate can be built rather than the
/// default one: an optional dependency nobody has enabled yet is still a dependency that the next
/// `--features` flag switches on. Dev edges are dropped at every hop — a consumer never builds
/// them, so `[dev-dependencies]` is not part of what depending on this crate pulls in.
fn crates_a_consumer_of_this_one_links() -> BTreeSet<String> {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--all-features", "--manifest-path", manifest])
        .output()
        .expect("cargo is what is running this test, so cargo is on the path");
    assert!(
        output.status.success(),
        "cargo metadata failed, so this test read no graph at all:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value =
        zyris::serde_json::from_slice(&output.stdout).expect("cargo metadata answers with JSON");

    // Package ids are opaque and their spelling has changed between Cargo releases, so they are
    // only ever compared to each other; names come from the package table.
    let mut name_of: HashMap<&str, &str> = HashMap::new();
    for package in metadata["packages"].as_array().expect("metadata lists packages") {
        let (id, name) = (package["id"].as_str(), package["name"].as_str());
        if let (Some(id), Some(name)) = (id, name) {
            name_of.insert(id, name);
        }
    }

    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    let nodes = metadata["resolve"]["nodes"].as_array().expect("the resolve names its nodes");
    for node in nodes {
        let from = node["id"].as_str().expect("every node has an id");
        for dep in node["deps"].as_array().into_iter().flatten() {
            let to = dep["pkg"].as_str().expect("every edge names a package");
            // No `dep_kinds` at all is an older Cargo, which only ever meant a normal edge.
            let linked = match dep["dep_kinds"].as_array() {
                None => true,
                Some(kinds) => kinds.iter().any(|kind| match kind["kind"].as_str() {
                    None => true,             // a normal dependency: null
                    Some("build") => true,    // built here, so it is still pulled in
                    Some(_) => false,         // "dev"
                }),
            };
            if linked {
                edges.entry(from).or_default().push(to);
            }
        }
    }

    let root = nodes
        .iter()
        .filter_map(|node| node["id"].as_str())
        .find(|id| name_of.get(id) == Some(&env!("CARGO_PKG_NAME")))
        .expect("the crate under test is in its own workspace's resolve");

    let mut seen = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let name = name_of.get(id).copied().unwrap_or(id);
        if !seen.insert(name.to_string()) {
            continue;
        }
        stack.extend(edges.get(id).into_iter().flatten().copied());
    }
    seen
}

#[test]
fn the_reference_implementations_are_not_offered_on_crates_io() {
    let manifest = file("zyris-capkit/Cargo.toml");
    assert!(
        manifest.contains("publish = false"),
        "zyris-capkit would be published, and it pins a git fork of enigo that cannot be"
    );
}

#[test]
fn depending_on_the_face_never_pulls_in_the_reference_implementations() {
    let linked = crates_a_consumer_of_this_one_links();

    // The walk has to have walked. `zyris-proto` is two hops out — through `zyris-core`, which is
    // the only crate this one depends on unconditionally — so its absence means the traversal
    // stopped at the root and the assertion below is being made about nothing.
    assert!(
        linked.contains("zyris-proto"),
        "the walk reached {} crate(s) and none of them was the wire format, so it never left the \
         root and proves nothing:\n  {}",
        linked.len(),
        linked.iter().cloned().collect::<Vec<_>>().join("\n  ")
    );

    assert!(
        !linked.contains("zyris-capkit"),
        "depending on {} now links zyris-capkit, so the face carries an unpublishable git fork of \
         enigo. The path may be direct or several crates down — `cargo tree -p {} --all-features \
         -i zyris-capkit` names it.",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_NAME")
    );
}

#[test]
fn the_face_does_not_advertise_a_crates_io_page_that_will_never_exist() {
    let readme = file("zyris/README.md");
    assert!(
        !readme.contains("crates.io/crates/zyris-capkit"),
        "the README links a crates.io page for a crate that is not published"
    );
}
