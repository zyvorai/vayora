// Rebuild when migrations change, so embedded migrations never go stale.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
