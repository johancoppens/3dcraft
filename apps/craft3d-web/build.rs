//! Embeds the commit this program is built from, for the bug report
//! (`craft3d-buildinfo`); `main` hands it to the engine.

fn main() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    craft3d_buildinfo::emit_commit(&workspace);
}
