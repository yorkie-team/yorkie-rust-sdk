#![forbid(unsafe_code)]

fn main() {
    let out = protogen::generated_file();
    let dir = out.parent().expect("generated dir");
    protogen::generate(dir).expect("generate protos");
    println!("wrote {}", out.display());
}
