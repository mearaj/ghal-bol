//! Bake `env/development.env` or `env/production.env` (under this crate) into the binary.
//
//! Selection:
//! - `GHALBOL_ENV=development|production` if set (scripts export this)
//! - else Cargo `PROFILE=release` → production, otherwise development
//!
//! Output: `$OUT_DIR/ghalbol.env` (embedded by `main.rs` via `include_str!`) and the
//! `GHALBOL_ENV` compile-time variable naming the profile.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn defaults(profile: &str) -> BTreeMap<&'static str, &'static str> {
    match profile {
        "production" => BTreeMap::from([
            ("GHALBOL_APP_ID", "com.ghalbol"),
            ("GHALBOL_APP_LABEL", "Ghal Bol"),
        ]),
        _ => BTreeMap::from([
            ("GHALBOL_APP_ID", "com.ghalbol.debug"),
            ("GHALBOL_APP_LABEL", "Ghal Bol Dev"),
        ]),
    }
}

/// `env/` beside this crate's `Cargo.toml`. cargo-makepad's Android build compiles the app
/// through a generated wrapper manifest elsewhere, with `build` pointing back at this file,
/// so fall back to the directory holding `build.rs`.
fn env_dir() -> PathBuf {
    let manifest_env = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("env");
    if manifest_env.is_dir() {
        return manifest_env;
    }
    Path::new(file!())
        .parent()
        .map(|dir| dir.join("env"))
        .unwrap_or(manifest_env)
}

fn main() {
    // Tell rustc about custom cfgs set below (avoids unexpected_cfgs warnings).
    println!("cargo:rustc-check-cfg=cfg(ghalbol_hide_console)");

    let ghalbol_env = env::var("GHALBOL_ENV").unwrap_or_else(|_| {
        match env::var("PROFILE").as_deref() {
            Ok("release") => "production".into(),
            _ => "development".into(),
        }
    });

    let env_path = env_dir().join(format!("{ghalbol_env}.env"));
    println!("cargo:rerun-if-changed={}", env_path.display());
    println!("cargo:rerun-if-env-changed=GHALBOL_ENV");

    let mut vars: BTreeMap<String, String> = defaults(&ghalbol_env)
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    if let Ok(text) = fs::read_to_string(&env_path) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !key.is_empty() {
                vars.insert(key.to_string(), value.to_string());
            }
        }
    } else {
        println!(
            "cargo:warning=GhalBol env file missing at {} — using built-in defaults",
            env_path.display()
        );
    }

    // Ensure ENV key matches the selected profile even if the file omits it.
    vars.insert("GHALBOL_ENV".into(), ghalbol_env.clone());

    // Production Windows PE: no console when the user double-clicks the exe.
    // Dev packages keep the console. Driven by GHALBOL_ENV, not Cargo profile
    // (Windows Dev packages also use --release — Cargo debug PE is huge).
    if ghalbol_env == "production" {
        println!("cargo:rustc-cfg=ghalbol_hide_console");
    }

    let baked: String = vars.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("ghalbol.env");
    fs::write(&out, baked).expect("write baked env");
    println!("cargo:rustc-env=GHALBOL_ENV={ghalbol_env}");
}
