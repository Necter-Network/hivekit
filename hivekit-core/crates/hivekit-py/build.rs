// Wheels (maturin, `extension-module` feature) must not link or rpath libpython.
// Plain `cargo test` links libpython, so let the test binary find it at runtime.
fn main() {
    if std::env::var_os("CARGO_FEATURE_EXTENSION_MODULE").is_none() {
        pyo3_build_config::add_python_framework_link_args();
        pyo3_build_config::add_libpython_rpath_link_args();
    }
}
