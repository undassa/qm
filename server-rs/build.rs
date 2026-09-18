use std::{env, fs, path::Path};

fn main() {
    let root = Path::new("../instrument");
    let mut files: Vec<(String, String)> = Vec::new();
    collect(root, root, &mut files);
    files.sort();
    let mut out = String::from("pub(crate) static FILES: &[(&str, &str)] = &[\n");
    for (name, path) in &files {
        println!("cargo:rerun-if-changed={path}");
        out.push_str(&format!("    ({name:?}, include_str!({path:?})),\n"));
    }
    out.push_str("];\n");
    let to = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("instrument.rs");
    fs::write(&to, out).expect("объявления прибора не записаны");
}

/// Имя в дереве — ключ, путь — то, что `include_str!` вшивает в двоичный файл.
///
/// Путь абсолютный: `include_str!` считает относительный от файла, в который
/// включён, а включается это из `OUT_DIR`, который лежит в другом месте.
fn collect(root: &Path, at: &Path, into: &mut Vec<(String, String)>) {
    // Каталог тоже под наблюдением: заведённый файл не меняет ни одного из
    // прежних, и пересборка по одним файлам его бы не заметила.
    println!("cargo:rerun-if-changed={}", at.display());
    let entries = fs::read_dir(at)
        .unwrap_or_else(|e| panic!("объявления прибора не читаются ({}): {e}", at.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, into);
        } else if let Ok(name) = path.strip_prefix(root) {
            let full = fs::canonicalize(&path).unwrap_or(path.clone());
            into.push((name.to_string_lossy().into_owned(), full.to_string_lossy().into_owned()));
        }
    }
}
