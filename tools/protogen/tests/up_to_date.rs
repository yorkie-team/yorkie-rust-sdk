#[test]
fn protogen_output_is_up_to_date() {
    let tmp = std::env::temp_dir().join(format!("protogen-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    protogen::generate(&tmp).unwrap();
    let fresh = std::fs::read(tmp.join("yorkie.v1.rs")).unwrap();
    let checked_in = std::fs::read(protogen::generated_file()).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        fresh == checked_in,
        "generated file is stale; run `cargo run -p protogen`"
    );
}
