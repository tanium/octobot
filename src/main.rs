mod server;

fn main() {
    // TODO: pass in config
    server::start("0.0.0.0:3000").expect("Failed to start server");
}
