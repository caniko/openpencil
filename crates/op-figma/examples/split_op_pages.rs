//! Extract named pages from a multi-page `.op` into standalone files.
//!
//! Usage: `split_op_pages <input.op> <output-dir> <page-name>...`

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else if ch.is_whitespace() {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(
        args.len() >= 3,
        "usage: split_op_pages <input.op> <output-dir> <page-name>..."
    );
    let input = &args[0];
    let output_dir = PathBuf::from(&args[1]);
    let requested = &args[2..];
    std::fs::create_dir_all(&output_dir).expect("create output directory");

    let source = std::fs::read_to_string(input).expect("read input document");
    let loaded = jian_ops_schema::compat::load_str(&source).expect("load input document");
    drop(source);
    let mut document = loaded.value;
    let pages = document.pages.take().expect("input document has pages");
    let mut found = Vec::new();

    for page in pages {
        if !requested.iter().any(|name| name == &page.name) {
            continue;
        }
        let page_name = page.name.clone();
        document.pages = Some(vec![page]);
        let mut value = serde_json::to_value(&document).expect("serialize selected page");
        jian_ops_schema::image_table::externalize_images(&mut value);
        let target = output_dir.join(format!("{}.op", safe_name(&page_name)));
        let writer = BufWriter::new(File::create(&target).expect("create selected-page file"));
        serde_json::to_writer(writer, &value).expect("write selected-page file");
        println!("{}\t{}", page_name, display_path(&target));
        found.push(page_name);
        document.pages = None;
    }

    for name in requested {
        assert!(
            found.iter().any(|found_name| found_name == name),
            "page not found: {name}"
        );
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
