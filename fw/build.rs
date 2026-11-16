fn main() {
    println!("cargo:rerun-if-changed=ws2812.x");
    println!("cargo:rustc-link-arg-bin=ws2812=-Tws2812.x");
    // `--nmagic` is required if memory section addresses are not aligned to 0x10000,
    // for example the FLASH and RAM sections in your `memory.x`.
    // See https://github.com/rust-embedded/cortex-m-quickstart/pull/95
    println!("cargo:rustc-link-arg=--nmagic");

    println!("cargo:rustc-link-arg=-Tlink.x");

    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
