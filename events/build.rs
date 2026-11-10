// protox compiles the proto in Rust, so building needs no protoc install.
fn main() {
    let proto = "proto/octobot/events/v1/events.proto";
    println!("cargo:rerun-if-changed={}", proto);

    let descriptors = protox::compile([proto], ["proto"]).expect("compile events.proto");
    tonic_prost_build::configure()
        .compile_fds(descriptors)
        .expect("generate events.proto code");
}
