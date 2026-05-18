fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protos = [
        "proto/yorkie/v1/resources.proto",
        "proto/yorkie/v1/yorkie.proto",
    ];
    let includes = ["proto"];

    prost_build::Config::new()
        .btree_map(["."])
        .compile_protos(&protos, &includes)?;

    for proto in protos {
        println!("cargo:rerun-if-changed={proto}");
    }

    Ok(())
}
