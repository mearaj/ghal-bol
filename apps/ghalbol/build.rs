//! Bake `env/development.env` or `env/production.env` (under this crate) into the binary.
//
//! Selection:
//! - `GHALBOL_ENV=development|production` if set (scripts export this)
//! - else Cargo `PROFILE=release` → production, otherwise development

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

fn defaults(profile: &str) -> HashMap<&'static str, &'static str> {
    match profile {
        "production" => HashMap::from([
            ("GHALBOL_ENV", "production"),
            ("GHALBOL_APP_ID", "com.ghalbol"),
            ("GHALBOL_APP_LABEL", "Ghal Bol"),
            ("GHALBOL_SHOW_SIZE_BADGE", "0"),
        ]),
        _ => HashMap::from([
            ("GHALBOL_ENV", "development"),
            ("GHALBOL_APP_ID", "com.ghalbol.debug"),
            ("GHALBOL_APP_LABEL", "Ghal Bol Dev"),
            ("GHALBOL_SHOW_SIZE_BADGE", "1"),
        ]),
    }
}

fn main() {
    // Tell rustc about custom cfgs set below (avoids unexpected_cfgs warnings).
    println!("cargo:rustc-check-cfg=cfg(ghabol_hide_console)");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    let ghalbol_env = env::var("GHALBOL_ENV").unwrap_or_else(|_| {
        match env::var("PROFILE").as_deref() {
            Ok("release") => "production".into(),
            _ => "development".into(),
        }
    });

    let env_path = manifest_dir.join("env").join(format!("{ghalbol_env}.env"));
    println!("cargo:rerun-if-changed={}", env_path.display());
    println!("cargo:rerun-if-env-changed=GHALBOL_ENV");
    println!("cargo:rerun-if-env-changed=PROFILE");

    let mut vars: HashMap<String, String> = defaults(&ghalbol_env)
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
        println!("cargo:rustc-cfg=ghabol_hide_console");
    }

    for (key, value) in vars {
        println!("cargo:rustc-env={key}={value}");
    }
}
