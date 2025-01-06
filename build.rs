extern crate vergen;

use vergen::EmitBuilder;

pub fn main() {
  // NOTE: This will output only a build timestamp and long SHA from git.
  // NOTE: This set requires the build and git features.
  // NOTE: See the EmitBuilder documentation for configuration options.
  EmitBuilder::builder()
    .build_timestamp()
    .git_sha(false)
    .emit()
    .unwrap();

  if cfg!(feature = "ffmpeg") && cfg!(target_os = "windows") {
    // https://github.com/microsoft/vcpkg/pull/14082
    println!("cargo:rustc-link-lib=dylib=strmiids");
    println!("cargo:rustc-link-lib=dylib=mfplat");
    println!("cargo:rustc-link-lib=dylib=mfuuid");
    println!("cargo:rustc-link-lib=dylib=mf"); 
  }
}
