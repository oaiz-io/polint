fn main() {
    println!("cargo:rerun-if-changed=vendor/tree-sitter-go/src/parser.c");
    println!("cargo:rerun-if-changed=vendor/tree-sitter-go/src/tree_sitter/parser.h");
    if std::env::var_os("CARGO_FEATURE_LANG_GO").is_none() {
        return;
    }
    compile_go_grammar();
}

fn compile_go_grammar() {
    let src_dir = std::path::Path::new("vendor/tree-sitter-go/src");
    let mut build = cc::Build::new();
    build.std("c11").include(src_dir);
    #[cfg(target_env = "msvc")]
    build.flag("-utf-8");
    let parser = src_dir.join("parser.c");
    build.file(&parser);
    build.compile("tree-sitter-go");
}
