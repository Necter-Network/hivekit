//! hivec: build, inspect and run HiveKit modules.
//!
//! ```text
//! hivec build [PATH] [--example NAME]     cargo build for wasm32-unknown-unknown, package a .hbc
//! hivec build module.wasm [--functions a,b]   package an existing hive-wasm-v1 module
//! hivec inspect FILE.hbc                  verify and show manifest, address, imports
//! hivec functions FILE                    exported functions of a .rs / .wasm / .hbc
//! hivec run FILE.hbc FN [INPUT]           execute on NDSR (delegates to the `ndsr` binary)
//! ```

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = cli::main() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod cli {
    use anyhow::{anyhow, bail, Context, Result};
    use clap::{Parser, Subcommand};
    use hivekit_core::{
        inspect_wasm, load_hbc, package_wasm, Artifact, FunctionRegistry, PackageWasmOptions,
    };
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};
    use std::process::Command as Proc;

    const TARGET: &str = "wasm32-unknown-unknown";

    #[derive(Parser)]
    #[command(
        name = "hivec",
        version,
        about = "Build, inspect and run HiveKit (hive-wasm-v1) modules"
    )]
    struct Cli {
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Subcommand)]
    enum Command {
        /// Compile a Rust crate (or one of its examples) for wasm32-unknown-unknown and
        /// package it as .hbc; or package an existing .wasm file.
        Build {
            /// Crate directory, Cargo.toml, or a .wasm file [default: current directory]
            path: Option<PathBuf>,
            /// Build this example of the crate instead of its library
            #[arg(long)]
            example: Option<String>,
            /// Module name in the manifest [default: the cargo target / file name]
            #[arg(long)]
            name: Option<String>,
            /// Output directory for <name>.hbc
            #[arg(short, long, default_value = "dist")]
            out: PathBuf,
            /// Exported function names (comma separated). Needed only for a .wasm
            /// without a hivekit.functions section; otherwise it must match it.
            #[arg(long)]
            functions: Option<String>,
            /// manifest.language for a .wasm input
            #[arg(long, default_value = "rust")]
            language: String,
            /// Optional manifest.version
            #[arg(long = "module-version")]
            module_version: Option<String>,
            /// Optional manifest.description
            #[arg(long)]
            description: Option<String>,
            /// Build with the dev profile instead of --release
            #[arg(long)]
            debug: bool,
            /// Cargo target directory
            #[arg(long)]
            target_dir: Option<PathBuf>,
        },
        /// Verify a .hbc like NDSR's loader and show its manifest, address and imports
        Inspect {
            hbc: PathBuf,
            /// Machine-readable output
            #[arg(long)]
            json: bool,
        },
        /// List exported functions (with func_id) of a .rs source, .wasm or .hbc
        Functions { file: PathBuf },
        /// Execute a function on NDSR (runs `ndsr run`; prints its signed receipt)
        Run {
            hbc: PathBuf,
            function: String,
            /// Input passed to the function as-is (usually JSON)
            input: Option<String>,
            #[arg(long, conflicts_with = "input")]
            input_file: Option<PathBuf>,
            /// Gas limit
            #[arg(long)]
            gas: Option<u64>,
            /// Persist state/ledger here (state survives across runs); omit for an ephemeral run
            #[arg(long)]
            data_dir: Option<PathBuf>,
            /// Make another module callable via hive.call (repeatable). It is
            /// verified and placed in <data-dir>/modules/<address>.hbc.
            #[arg(long = "module")]
            modules: Vec<PathBuf>,
            /// Path to the ndsr binary [default: $NDSR_BIN, `ndsr` on PATH, or tools/ndsr in a parent directory]
            #[arg(long)]
            ndsr: Option<PathBuf>,
        },
    }

    pub fn main() -> Result<()> {
        match Cli::parse().command {
            Command::Build {
                path,
                example,
                name,
                out,
                functions,
                language,
                module_version,
                description,
                debug,
                target_dir,
            } => build(BuildArgs {
                path: path.unwrap_or_else(|| PathBuf::from(".")),
                example,
                name,
                out,
                functions: functions.map(|f| {
                    f.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }),
                language,
                version: module_version,
                description,
                debug,
                target_dir,
            }),
            Command::Inspect { hbc, json } => inspect(&hbc, json),
            Command::Functions { file } => functions(&file),
            Command::Run {
                hbc,
                function,
                input,
                input_file,
                gas,
                data_dir,
                modules,
                ndsr,
            } => {
                let code = run(RunArgs {
                    hbc,
                    function,
                    input,
                    input_file,
                    gas,
                    data_dir,
                    modules,
                    ndsr,
                })?;
                std::process::exit(code);
            }
        }
    }

    // ── build ────────────────────────────────────────────────────────────────

    struct BuildArgs {
        path: PathBuf,
        example: Option<String>,
        name: Option<String>,
        out: PathBuf,
        functions: Option<Vec<String>>,
        language: String,
        version: Option<String>,
        description: Option<String>,
        debug: bool,
        target_dir: Option<PathBuf>,
    }

    struct Built {
        wasm: PathBuf,
        target_name: String,
        src_path: Option<PathBuf>,
    }

    /// Existing user flags (RUSTFLAGS / CARGO_ENCODED_RUSTFLAGS) plus
    /// `--remap-path-prefix` for the crate directory and the cargo home.
    /// Note: env flags take precedence over `build.rustflags` in cargo config.
    fn reproducible_rustflags(manifest: &Path) -> String {
        let mut flags: Vec<String> = match std::env::var("CARGO_ENCODED_RUSTFLAGS") {
            Ok(s) if !s.is_empty() => s.split('\x1f').map(str::to_owned).collect(),
            _ => std::env::var("RUSTFLAGS")
                .map(|s| s.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
        };
        let crate_dir = manifest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let crate_dir = crate_dir.canonicalize().unwrap_or(crate_dir);
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")));
        // Most specific prefix last: rustc applies the last matching remap.
        if let Some(home) = cargo_home {
            flags.push(format!("--remap-path-prefix={}=/cargo", home.display()));
        }
        flags.push(format!("--remap-path-prefix={}=/src", crate_dir.display()));
        flags.join("\x1f")
    }

    fn cargo_build(a: &BuildArgs) -> Result<Built> {
        let manifest = if a.path.is_dir() {
            a.path.join("Cargo.toml")
        } else {
            a.path.clone()
        };
        if !manifest.exists() {
            bail!("{} not found", manifest.display());
        }
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut cmd = Proc::new(cargo);
        cmd.arg("build")
            .arg("--target")
            .arg(TARGET)
            .arg("--message-format=json-render-diagnostics")
            .arg("--manifest-path")
            .arg(&manifest);
        if !a.debug {
            cmd.arg("--release");
        }
        match &a.example {
            Some(e) => {
                cmd.arg("--example").arg(e);
            }
            None => {
                cmd.arg("--lib");
            }
        }
        if let Some(t) = &a.target_dir {
            cmd.arg("--target-dir").arg(t);
        }
        // Panic messages embed source paths, which would make the module address
        // depend on where the crate and the cargo registry live. Remap them to
        // fixed prefixes so the same source builds the same address everywhere.
        cmd.env("CARGO_ENCODED_RUSTFLAGS", reproducible_rustflags(&manifest));
        eprintln!(
            "hivec: cargo build --target {TARGET}{}",
            if a.debug { "" } else { " --release" }
        );
        let out = cmd
            .stderr(std::process::Stdio::inherit())
            .output()
            .context("failed to run cargo")?;
        let mut found: Option<Built> = None;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if v["reason"] != "compiler-artifact" {
                continue;
            }
            let kinds: Vec<&str> = v["target"]["kind"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|k| k.as_str())
                .collect();
            let wanted = match &a.example {
                Some(e) => kinds.contains(&"example") && v["target"]["name"] == e.as_str(),
                None => kinds.contains(&"cdylib"),
            };
            if !wanted {
                continue;
            }
            let wasm = v["filenames"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| f.as_str())
                .find(|f| f.ends_with(".wasm"));
            if let Some(w) = wasm {
                found = Some(Built {
                    wasm: PathBuf::from(w),
                    target_name: v["target"]["name"].as_str().unwrap_or("module").to_string(),
                    src_path: v["target"]["src_path"].as_str().map(PathBuf::from),
                });
            }
        }
        if !out.status.success() {
            bail!("cargo build failed (is the target installed? `rustup target add {TARGET}`)");
        }
        found.ok_or_else(|| {
            anyhow!(
                "cargo produced no .wasm; the crate (or example) needs `crate-type = [\"cdylib\"]` \
                 and a `hive_module!(...)` invocation"
            )
        })
    }

    fn build(a: BuildArgs) -> Result<()> {
        let is_wasm = a.path.extension().is_some_and(|e| e == "wasm");
        let (wasm_path, default_name, src) = if is_wasm {
            let stem = a
                .path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            (a.path.clone(), stem, None)
        } else {
            let b = cargo_build(&a)?;
            (b.wasm, b.target_name, b.src_path)
        };
        let wasm = std::fs::read(&wasm_path)
            .with_context(|| format!("cannot read {}", wasm_path.display()))?;
        let info = inspect_wasm(&wasm)?;

        // Cross-check the module's own dispatch table against source discovery.
        if let (Some(src), Some(declared)) = (&src, &info.declared_functions) {
            if let Ok(text) = std::fs::read_to_string(src) {
                let found = FunctionRegistry::collect(&text, "rust");
                let mut declared = declared.clone();
                declared.sort();
                if !found.is_empty() && found != declared {
                    eprintln!(
                        "hivec: note: #[hive_export] functions in {} are {found:?}, \
                         the module exports {declared:?} (only functions listed in hive_module!() are exported)",
                        src.display()
                    );
                }
            }
        }

        let name = a.name.clone().unwrap_or(default_name);
        let r = package_wasm(PackageWasmOptions {
            name: name.clone(),
            language: if is_wasm {
                a.language.clone()
            } else {
                "rust".into()
            },
            wasm_bytes: wasm,
            functions: a.functions.clone(),
            compiler: Some(format!("hivec-rs/{}", env!("CARGO_PKG_VERSION"))),
            version: a.version.clone(),
            description: a.description.clone(),
        })
        .with_context(|| format!("cannot package {}", wasm_path.display()))?;

        std::fs::create_dir_all(&a.out)?;
        let hbc = a.out.join(format!("{name}.hbc"));
        std::fs::write(&hbc, &r.hbc_bytes)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "hbc": hbc,
                "manifest_address": r.manifest_address,
                "functions": r.functions,
                "wasm_bytes": r.module_bytes.len(),
                "imports": info.imports,
            }))?
        );
        Ok(())
    }

    // ── inspect / functions ──────────────────────────────────────────────────

    fn load(path: &Path) -> Result<Artifact> {
        let bytes =
            std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        load_hbc(&bytes).with_context(|| format!("{} is not a valid .hbc", path.display()))
    }

    fn inspect(path: &Path, as_json: bool) -> Result<()> {
        let a = load(path)?;
        let funcs: Vec<Value> = a
            .manifest
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| json!({ "func_id": i, "name": f }))
            .collect();
        let v = json!({
            "file": path,
            "manifest_address": a.manifest_address,
            "address_declared": a.manifest.manifest_address.is_some(),
            "manifest": a.manifest,
            "functions": funcs,
            "wasm_bytes": a.wasm.len(),
            "imports": a.wasm_info.imports,
            "exports": a.wasm_info.exports,
            "executable": a.is_executable(),
            "abi_error": a.wasm_info.abi_error,
        });
        if as_json {
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            println!("file:             {}", path.display());
            println!("manifest_address: {} (verified)", a.manifest_address);
            println!("name:             {}", a.manifest.name);
            println!("language:         {}", a.manifest.language);
            println!("compiler:         {}", a.manifest.compiler);
            println!("runtime:          {}", a.manifest.runtime);
            if let Some(v) = &a.manifest.version {
                println!("version:          {v}");
            }
            if let Some(d) = &a.manifest.description {
                println!("description:      {d}");
            }
            println!("module.wasm:      {} bytes", a.wasm.len());
            println!("imports:          {}", a.wasm_info.imports.join(", "));
            println!("functions:");
            for (i, f) in a.manifest.functions.iter().enumerate() {
                println!("  {i:>3}  {f}");
            }
            match &a.wasm_info.abi_error {
                None => println!("abi:              hive-wasm-v1 ok"),
                Some(e) => println!("abi:              INVALID: {e}"),
            }
        }
        if let Some(e) = &a.wasm_info.abi_error {
            bail!("module is not executable on NDSR: {e}");
        }
        Ok(())
    }

    fn functions(path: &Path) -> Result<()> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let list = match ext.as_str() {
            "hbc" => load(path)?.manifest.functions,
            "wasm" => {
                let w = std::fs::read(path)?;
                let mut d = inspect_wasm(&w)?.declared_functions.ok_or_else(|| {
                    anyhow!("{} has no hivekit.functions section", path.display())
                })?;
                d.sort();
                d
            }
            "rs" => FunctionRegistry::collect(&std::fs::read_to_string(path)?, "rust"),
            "py" => FunctionRegistry::collect(&std::fs::read_to_string(path)?, "python"),
            "go" => FunctionRegistry::collect(&std::fs::read_to_string(path)?, "go"),
            "js" | "ts" => FunctionRegistry::collect(&std::fs::read_to_string(path)?, "javascript"),
            other => {
                bail!("unsupported file type .{other} (expected .rs .wasm .hbc .py .go .js .ts)")
            }
        };
        if list.is_empty() {
            bail!("no exported functions found in {}", path.display());
        }
        for (i, f) in list.iter().enumerate() {
            println!("{i}\t{f}");
        }
        Ok(())
    }

    // ── run ──────────────────────────────────────────────────────────────────

    struct RunArgs {
        hbc: PathBuf,
        function: String,
        input: Option<String>,
        input_file: Option<PathBuf>,
        gas: Option<u64>,
        data_dir: Option<PathBuf>,
        modules: Vec<PathBuf>,
        ndsr: Option<PathBuf>,
    }

    fn is_executable_file(p: &Path) -> bool {
        p.is_file()
    }

    fn find_ndsr(explicit: Option<&Path>, hbc: &Path) -> Result<PathBuf> {
        if let Some(p) = explicit {
            return Ok(p.to_path_buf());
        }
        if let Some(p) = std::env::var_os("NDSR_BIN") {
            return Ok(PathBuf::from(p));
        }
        if let Some(paths) = std::env::var_os("PATH") {
            for d in std::env::split_paths(&paths) {
                let c = d.join("ndsr");
                if is_executable_file(&c) {
                    return Ok(c);
                }
            }
        }
        let mut starts = vec![std::env::current_dir()?];
        if let Ok(h) = hbc.canonicalize() {
            starts.push(h);
        }
        for s in starts {
            for dir in s.ancestors() {
                let c = dir.join("tools").join("ndsr");
                if is_executable_file(&c) {
                    return Ok(c);
                }
            }
        }
        bail!(
            "the ndsr binary was not found. hivec run executes modules on the real NDSR runtime \
             (it has no built-in interpreter); pass --ndsr PATH, set NDSR_BIN, or put ndsr on PATH"
        )
    }

    fn run(a: RunArgs) -> Result<i32> {
        // Fail early with a clear message if the artifact is not loadable.
        let art = load(&a.hbc)?;
        if art.manifest.func_id(&a.function).is_none() {
            bail!(
                "function {:?} is not in the manifest; available: {:?}",
                a.function,
                art.manifest.functions
            );
        }
        let ndsr = find_ndsr(a.ndsr.as_deref(), &a.hbc)?;

        // hive.call targets are resolved from <data-dir>/modules/<address>.hbc.
        let mut temp: Option<PathBuf> = None;
        let data_dir = match (&a.data_dir, a.modules.is_empty()) {
            (Some(d), _) => Some(d.clone()),
            (None, true) => None,
            (None, false) => {
                let d = std::env::temp_dir().join(format!("hivec-run-{}", std::process::id()));
                temp = Some(d.clone());
                Some(d)
            }
        };
        if let Some(d) = &data_dir {
            if !a.modules.is_empty() {
                let mdir = d.join("modules");
                std::fs::create_dir_all(&mdir)?;
                for m in &a.modules {
                    let bytes =
                        std::fs::read(m).with_context(|| format!("cannot read {}", m.display()))?;
                    let addr = load_hbc(&bytes)
                        .with_context(|| format!("{} is not a valid .hbc", m.display()))?
                        .manifest_address;
                    std::fs::write(mdir.join(format!("{addr}.hbc")), &bytes)?;
                    eprintln!("hivec: {} available to hive.call at {addr}", m.display());
                }
            }
        }

        let mut cmd = Proc::new(&ndsr);
        cmd.arg("run").arg(&a.hbc).arg(&a.function);
        match (&a.input, &a.input_file) {
            (Some(i), _) => {
                cmd.arg("--input").arg(i);
            }
            (None, Some(f)) => {
                cmd.arg("--input-file").arg(f);
            }
            (None, None) => {}
        }
        if let Some(g) = a.gas {
            cmd.arg("--gas").arg(g.to_string());
        }
        if let Some(d) = &data_dir {
            cmd.arg("--data-dir").arg(d);
        }
        let status = cmd
            .status()
            .with_context(|| format!("failed to run {}", ndsr.display()));
        if let Some(t) = temp {
            let _ = std::fs::remove_dir_all(t);
        }
        Ok(status?.code().unwrap_or(1))
    }
}
