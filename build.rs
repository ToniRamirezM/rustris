// build.rs
fn main() {
    cc::Build::new()
        .cpp(true)
        .files([
            "src/libs/gb_apu/Gb_Apu.cpp",
            "src/libs/gb_apu/Gb_Oscs.cpp",
            "src/libs/gb_apu/Blip_Buffer.cpp",
            "src/libs/gb_apu/Multi_Buffer.cpp",
            "src/apu_c_wrapper.cpp",
        ])
        .flag_if_supported("-std=c++14")
        .flag_if_supported("-O2")
        .flag_if_supported("-fno-exceptions")
        .flag_if_supported("-fno-rtti")
        .include("libs/gb_apu")
        .compile("gb_apu");
    
    println!("cargo:rustc-link-lib=c++");
}
