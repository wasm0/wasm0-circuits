use std::{
    env,
    io::{self, Write},
};

fn main() {
    let lib_name = "geth-utils";
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // Build
    let mut build = gobuild::Build::new();

    // Replace to a custom go-ethereum for scroll.
    // #[cfg(feature = "scroll")]
    // build.modfile("scroll.mod");

    if let Err(e) = build.file("./lib/lib.go").try_compile(lib_name) {
        // The error type is private so have to check the error string
        if format!("{}", e).starts_with("Failed to find tool.") {
            fail(
                " Failed to find Go. Please install Go 1.16 or later \
                following the instructions at https://golang.org/doc/install.
                On linux it is also likely available as a package."
                    .to_string(),
            );
        } else {
            fail(format!("{}", e));
        }
    }

    // Files the lib depends on that should recompile the lib
    let dep_files = vec![
        "./gethutil/asm.go",
        "./gethutil/trace.go",
        "./gethutil/util.go",
        "./go.mod",
        "./go.sum",
        "./scroll.mod",
    ];
    for file in dep_files {
        println!("cargo:rerun-if-changed={}", file);
    }

    // Link
    println!("cargo:rustc-link-search=native={}", out_dir);
    println!("cargo:rustc-link-lib=static={}", lib_name);

    let external_libs = vec![
        ("zkwasm-gas-injector", "gas_injector"),
        ("zkwasm-wasmi", "wasmi_c_api"),
    ];

    let mut local_libs_paths: Vec<String> = vec![];
    let mut local_libs_names: Vec<String> = vec![];
    let arch = env::consts::ARCH;
    for (go_package_name, go_lib_name) in external_libs {
        local_libs_names.push(go_lib_name.to_string());
        let mut local_libs_subdirs = vec![];
        let go_mod_file_rel_path = manifest_dir.as_str();
        let go_mod_file_name = "go.mod";
        let go_package_path = golang_utils::go_package_system_path(
            go_package_name,
            go_mod_file_name,
            go_mod_file_rel_path
        ).unwrap();
        match env::consts::OS {
            "linux" => {
                if arch.contains("x86_64") || arch.contains("amd64") {
                    local_libs_subdirs.push("linux-amd64");
                } else {
                    panic!("unsupported arch '{}'", arch)
                }
            },
            "macos" => {
                if arch.contains("aarch64") { local_libs_subdirs.push("darwin-aarch64"); }
                else if arch.contains("x86_64") || arch.contains("amd64") {
                    local_libs_subdirs.push("darwin-amd64");
                } else {
                    panic!("unsupported arch '{}'", arch)
                }
            },
            platform => panic!("unsupported build platform '{}'", platform)
        }
        for subdir in local_libs_subdirs {
            let local_libs_path = go_package_path.clone() + "/packaged/lib/" + subdir;
            local_libs_paths.push(local_libs_path);
        }
    }
    for (i, local_lib_path) in local_libs_paths.iter().enumerate() {
        println!("cargo:rustc-link-lib={}", local_libs_names[i]);
        println!("cargo:rustc-link-search={}", local_lib_path);
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", local_lib_path);

    }
}

fn fail(message: String) {
    let _ = writeln!(
        io::stderr(),
        "\n\nError while building geth-utils: {}\n\n",
        message
    );
    std::process::exit(1);
}
