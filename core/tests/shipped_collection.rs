//! The collection in server/launcher-rules.json must parse on its own. Rules::parse falls back to the
//! built-in list when the object is bad, so a typo would otherwise ship unnoticed.
use launcher_core::collection::Collection;

#[test]
fn the_shipped_rules_file_carries_a_valid_collection() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../server/launcher-rules.json");
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let c = Collection::parse(v["collection"].to_string().as_bytes()).expect("collection in launcher-rules.json parses");
    assert!(c.mods.iter().all(|m| !m.files.is_empty()), "every listed mod needs a detection file");
    assert_eq!(c.revision, v["collection"]["revision"].as_u64().unwrap() as u32);
}
