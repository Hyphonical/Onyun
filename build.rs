//! Generates the embedded `web/dist` asset table, the `u64` WGSL shader variant, and optional Tauri build metadata.

use std::{
    env,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=web/dist");
    println!("cargo:rerun-if-changed=src/core/engine/ed25519_mint.wgsl");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "missing OUT_DIR environment variable",
        )
    })?);
    let dest = out_dir.join("web_assets.rs");
    let mut out = File::create(dest)?;

    let dist = Path::new("web/dist");
    let mut entries = Vec::new();
    if dist.is_dir() {
        collect_files(dist, dist, &mut entries);
        entries.sort_by(|a, b| a.0.cmp(&b.0));
    }

    writeln!(
        out,
        "pub(super) static WEB_ASSETS: &[(&str, &str, &[u8])] = &["
    )?;
    for (route, abs_path) in entries {
        let mime = mime_for(&route);
        let escaped_path = abs_path.to_string_lossy().replace('\\', "/");
        writeln!(
            out,
            "\t({route:?}, {mime:?}, include_bytes!({escaped_path:?})),"
        )?;
    }
    writeln!(out, "];")?;
    generate_u64_shader(&out_dir)?;
    if env::var_os("CARGO_FEATURE_DESKTOP").is_some()
        && let Err(err) = tauri_build::try_build(
            tauri_build::Attributes::new().config_path("web/tauri.conf.json"),
        )
    {
        return Err(io::Error::other(err));
    }
    Ok(())
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() {
            collect_files(root, &path, out);
        } else if ft.is_file()
            && let Ok(rel) = path.strip_prefix(root)
            && let Ok(abs) = fs::canonicalize(&path)
        {
            let clean_abs = abs
                .to_str()
                .and_then(|s| s.strip_prefix(r"\\?\"))
                .map_or_else(|| abs.clone(), PathBuf::from);
            let route = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
            out.push((route, clean_abs));
        }
    }
}

fn mime_for(route: &str) -> &'static str {
    match Path::new(route)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "png" => "image/png",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn generate_u64_shader(out_dir: &Path) -> io::Result<()> {
    let src = fs::read_to_string("src/core/engine/ed25519_mint.wgsl")?;
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("var ")
            && let Some((lhs, rhs)) = rest.split_once(" = mul32_to_64(")
            && let Some(args) = rhs.strip_suffix(");")
            && let Some((a, b)) = args.split_once(", ")
        {
            out.push_str("    var ");
            out.push_str(lhs);
            out.push_str(": u64 = u64(");
            out.push_str(a);
            out.push_str(") * u64(");
            out.push_str(b);
            out.push_str(");\n");
        } else if let Some((lhs, rhs)) = trimmed.split_once(" = add64(")
            && let Some(inner) = rhs.strip_suffix(");")
            && let Some((_, mul_call)) = inner.split_once(", mul32_to_64(")
            && let Some(args) = mul_call.strip_suffix(')')
            && let Some((a, b)) = args.split_once(", ")
        {
            out.push_str("    ");
            out.push_str(lhs);
            out.push_str(" += u64(");
            out.push_str(a);
            out.push_str(") * u64(");
            out.push_str(b);
            out.push_str(");\n");
        } else if trimmed.contains(" = add64(")
            && trimmed.contains("shr64(")
            && let Some((add_stmt, var_stmt)) = trimmed.split_once("; ")
            && let Some((lhs, rhs)) = add_stmt.split_once(" = add64(")
            && let Some(inner) = rhs.strip_suffix(')')
            && let Some((_, shr_call)) = inner.split_once(", shr64(")
            && let Some(shr_args) = shr_call.strip_suffix(')')
            && let Some((src_t, shift)) = shr_args.split_once(", ")
        {
            let var_converted = var_stmt.replace(".x &", ") &").replace("= t", "= u32(t");
            out.push_str("    ");
            out.push_str(lhs);
            out.push_str(" += ");
            out.push_str(src_t);
            out.push_str(" >> ");
            out.push_str(shift);
            out.push_str("; ");
            out.push_str(&var_converted);
            out.push('\n');
        } else {
            let replaced = match trimmed {
                "let c9 = shr64(t9, 25u);" => "    let c9: u64 = t9 >> 25u;",
                "let r9 = t9.x & MASK25;" => "    let r9 = u32(t9) & MASK25;",
                "let c9_19 = mul32_to_64(c9.x, 19u);" => "    let c9_19: u64 = c9 * u64(19u);",
                "let r0_64 = add64(vec2<u32>(r0, 0u), c9_19);" => {
                    "    let r0_64: u64 = u64(r0) + c9_19;"
                }
                "r0 = r0_64.x & MASK26;" => "    r0 = u32(r0_64) & MASK26;",
                "r1 = r1 + shr64(r0_64, 26u).x;" => "    r1 = r1 + u32(r0_64 >> 26u);",
                _ => line,
            };
            out.push_str(replaced);
            out.push('\n');
        }
    }
    fs::write(out_dir.join("ed25519_mint_u64.wgsl"), out)
}
