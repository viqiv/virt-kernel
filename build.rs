fn main() {
    println!("cargo:rerun-if-changed=src/link.ld");
    // println!("cargo:rustc-link-arg=src/sys.o");
}
