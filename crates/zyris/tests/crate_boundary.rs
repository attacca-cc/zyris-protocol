//! What the published face names is a promise, so the boundary is worth a test.
//!
//! Two promises, and they are different. **The face stays thin**: an implementation is the node's
//! business, so depending on `zyris` never drags one in — a one-line edit away from being undone by
//! anyone adding "just one more re-export". And **what says it publishes can**: crates.io refuses
//! any manifest whose graph names a git source, so a git dependency added three crates down turns a
//! publishable crate unpublishable with nothing at compile time to say so. Before the split that
//! was a name check against one crate; now it is a property of every member, checked by walking.
//!
//! `zyris-input` is the single expected exception and is named as one. It pins a fork of `enigo`
//! carrying two patches upstream does not have, and until those land it cannot go to the registry.

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
    linked_from(env!("CARGO_PKG_NAME")).into_keys().collect()
}

/// Every crate reachable from `root`, mapped to where Cargo would fetch it from.
///
/// The source string is what decides publishability: `null` for a workspace member, `registry+…`
/// for crates.io, `git+…` for a checkout. A crate whose graph contains the last of those cannot be
/// published, however deep down it sits and whatever it is called.
fn linked_from(root_name: &str) -> HashMap<String, String> {
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
    let mut source_of: HashMap<&str, &str> = HashMap::new();
    for package in metadata["packages"].as_array().expect("metadata lists packages") {
        let (id, name) = (package["id"].as_str(), package["name"].as_str());
        if let (Some(id), Some(name)) = (id, name) {
            name_of.insert(id, name);
            // A workspace member has no source at all, which is neither a registry nor a git
            // checkout and is exactly what "this repository" looks like from here.
            source_of.insert(id, package["source"].as_str().unwrap_or("workspace"));
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
        .find(|id| name_of.get(id) == Some(&root_name))
        .unwrap_or_else(|| panic!("{root_name} is not in this workspace's resolve"));

    let mut seen: HashMap<String, String> = HashMap::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let name = name_of.get(id).copied().unwrap_or(id);
        let source = source_of.get(id).copied().unwrap_or("workspace");
        if seen.insert(name.to_string(), source.to_string()).is_some() {
            continue;
        }
        stack.extend(edges.get(id).into_iter().flatten().copied());
    }
    seen
}

/// Every workspace member, and whether its manifest says it may be published.
///
/// Read from the manifests rather than from `cargo metadata`'s `publish` field, because that field
/// is `null` both for "publish anywhere" and for versions of Cargo that did not report it, and the
/// difference matters here.
fn members() -> Vec<(String, bool)> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates/ is the parent");
    let mut found = Vec::new();
    for entry in fs::read_dir(crates).expect("crates/ is readable") {
        let dir = entry.expect("a readable directory entry").path();
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = fs::read_to_string(&manifest).expect("a member manifest is readable");
        let name = dir.file_name().expect("a named directory").to_string_lossy().into_owned();
        found.push((name, !text.contains("publish = false")));
    }
    found.sort();
    assert!(found.len() >= 5, "found only {} members, so this test read almost nothing", found.len());

    // A crate directory the root manifest does not list is not built by anything: not by
    // `--workspace`, not by CI, not by `cargo publish`. It looks like code and behaves like a
    // deleted file, which is the worst of the two.
    let root = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(|c| c.parent())
            .expect("the workspace root is two levels up").join("Cargo.toml"),
    )
    .expect("the workspace manifest is readable");
    for (name, _) in &found {
        assert!(
            root.contains(&format!("\"crates/{name}\"")),
            "crates/{name} exists but the workspace `members` list does not name it, so nothing \
             ever builds it"
        );
    }
    found
}

/// The face is what `cargo add zyris` gets, and it has to be publishable.
///
/// Strictly stronger than the name check this replaces: it also catches a git dependency added
/// three crates down in `zyris-p2p` or `zyris-attacca`, and it needs no hand-written exclusion list
/// to go stale the day a sixth addon appears.
#[test]
fn nothing_the_face_links_comes_from_a_git_checkout() {
    let linked = linked_from(env!("CARGO_PKG_NAME"));

    // The walk has to have walked.
    assert!(
        linked.contains_key("zyris-proto"),
        "the walk reached {} crate(s) and none was the wire format, so it never left the root",
        linked.len()
    );

    let from_git: Vec<_> = linked
        .iter()
        .filter(|(_, source)| source.starts_with("git+"))
        .map(|(name, source)| format!("{name} ({source})"))
        .collect();
    assert!(
        from_git.is_empty(),
        "depending on {} now reaches a git checkout, so it can never be published:\n  {}",
        env!("CARGO_PKG_NAME"),
        from_git.join("\n  ")
    );
}

/// The same property, for every member that has not said `publish = false`.
///
/// This is the one that would have caught the split going wrong in the other direction: cutting
/// `zyris-fs` out of `zyris-capkit` is worth doing precisely because it stops one git fork from
/// making four pure-Rust crates unpublishable, and nothing would have said so if the fork had come
/// along by accident.
#[test]
fn every_crate_that_says_it_publishes_can() {
    for (member, publishes) in members() {
        if !publishes {
            // A crate that says `publish = false` is out of scope here — but the *reason* is worth
            // holding on to. `zyris-input` opts out because it pins a fork of `enigo`, and if that
            // fork ever lands upstream the opt-out becomes a crate withheld for no reason. So the
            // reason is asserted rather than trusted: no git source left means it is time to
            // publish this one too.
            if member == "zyris-input" {
                let reaches_git =
                    linked_from(&member).values().any(|source| source.starts_with("git+"));
                assert!(
                    reaches_git,
                    "zyris-input no longer reaches a git checkout, so the reason it carries \
                     `publish = false` is gone — drop the opt-out and let it publish"
                );
            }
            continue;
        }

        let linked = linked_from(&member);
        let from_git: Vec<_> = linked
            .iter()
            .filter(|(_, source)| source.starts_with("git+"))
            .map(|(name, source)| format!("{name} ({source})"))
            .collect();
        assert!(
            from_git.is_empty(),
            "{member} has no `publish = false` but reaches a git checkout, so `cargo publish` \
             would refuse it:\n  {}",
            from_git.join("\n  ")
        );
    }
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

    for implementation in
        ["zyris-fs", "zyris-terminal", "zyris-transfer", "zyris-screen", "zyris-input"]
    {
        assert!(
            !linked.contains(implementation),
            "depending on {} now links {implementation}, so the face decides what a node offers. \
             The path may be direct or several crates down — `cargo tree -p {} --all-features -i \
             {implementation}` names it.",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_NAME")
        );
    }
}

/// Attacca is one deployment, not the protocol. This crate is what someone reaching for "the Zyris
/// protocol" gets, and a particular hub's capability declaration does not belong in that: a node
/// that talks to Attacca names `zyris-attacca` directly, which is what every consumer already does
/// — zyris-code, zyris-daemon and Attacca's own server all list it as a dependency of their own,
/// and none of them has ever asked the face for it.
///
/// `--all-features` is the whole point. An optional dependency nobody has enabled yet is still one
/// `--features` flag away from being linked, so the promise has to be about every way this crate
/// can be built rather than the default one.
#[test]
fn the_face_does_not_name_one_deployments_hub() {
    let linked = crates_a_consumer_of_this_one_links();

    // Same guard as above: if the walk never left the root, the assertion below proves nothing.
    assert!(
        linked.contains("zyris-proto"),
        "the walk reached {} crate(s) and none of them was the wire format, so it never left the \
         root and proves nothing",
        linked.len()
    );

    assert!(
        !linked.contains("zyris-attacca"),
        "depending on {} now links zyris-attacca, so the protocol's face carries one deployment's \
         hub. `cargo tree -p {} --all-features -i zyris-attacca` names the path.",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_NAME")
    );
}

#[test]
fn the_face_does_not_advertise_a_crates_io_page_that_will_never_exist() {
    let readme = file("zyris/README.md");
    assert!(
        !readme.contains("crates.io/crates/zyris-input"),
        "the README links a crates.io page for the one crate that cannot be published"
    );
}
