#![allow(clippy::indexing_slicing)]
use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs::File,
    io::{BufWriter, Read},
    path::Path,
};

use rayon::prelude::*;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

fn fold_char(c: char) -> impl Iterator<Item = char> {
    let mut buf = ['\0'; 8];
    let mut len = 0usize;
    for d in c.nfkd().filter(|&d| !is_combining_mark(d)).flat_map(|d| d.to_lowercase()) {
        if let Some(slot) = buf.get_mut(len) {
            *slot = d;
            len += 1;
        }
    }
    if len == 0 {
        for d in c.to_lowercase() {
            if let Some(slot) = buf.get_mut(len) {
                *slot = d;
                len += 1;
            }
        }
    }
    buf.into_iter().take(len)
}

fn process_chunk(chunk: &[u8], skip_invalid: bool) -> BTreeSet<String> {
    let mut words = BTreeSet::new();

    for line in chunk.split(|&b| b == b'\n') {
        let line = if let Some(stripped) = line.strip_suffix(b"\r") { stripped } else { line };

        if line.is_empty() {
            continue;
        }

        if skip_invalid {
            // Strict UTF-8: skip any line that contains invalid UTF-8 sequences
            if let Ok(valid_str) = std::str::from_utf8(line) {
                let trimmed = valid_str.trim();
                if !trimmed.is_empty() {
                    let normalized: String = trimmed.nfc().flat_map(fold_char).collect();
                    if normalized.chars().count() >= 2 {
                        words.insert(normalized);
                    }
                }
            }
        } else {
            // Lossy UTF-8: convert non-UTF-8 bytes and clean up replacement characters
            let lossy = String::from_utf8_lossy(line);
            let trimmed = lossy.trim().trim_matches('\u{FFFD}');
            if !trimmed.is_empty() {
                let normalized: String =
                    trimmed.nfc().flat_map(fold_char).filter(|&c| c != '\u{FFFD}').collect();
                if normalized.chars().count() >= 2 {
                    words.insert(normalized);
                }
            }
        }
    }

    words
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-env-changed=UNIPASTREN_BUILD_DICTS");
    println!("cargo:rerun-if-env-changed=UNIPASTREN_BUILD_SKIP_INVALID");

    let out_dir = env::var("OUT_DIR")?;
    let dest_path = Path::new(&out_dir).join("embedded_dict.fst");

    let skip_invalid = env::var("UNIPASTREN_BUILD_SKIP_INVALID")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let mut words = BTreeSet::new();

    if let Ok(dict_list) = env::var("UNIPASTREN_BUILD_DICTS") {
        for raw_path in dict_list.split([':', ';']).filter(|s| !s.trim().is_empty()) {
            let path = Path::new(raw_path.trim());
            println!("cargo:rerun-if-changed={}", path.display());

            let mut file = File::open(path).map_err(|e| {
                format!(
                    "Failed to open dictionary file '{}' from UNIPASTREN_BUILD_DICTS: {}",
                    path.display(),
                    e
                )
            })?;

            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;

            // Split buffer across newline boundaries into ~8MB chunks for rayon
            const CHUNK_TARGET: usize = 8 * 1024 * 1024;
            let mut chunks = Vec::new();
            let mut start = 0;

            while start < buffer.len() {
                let end = (start + CHUNK_TARGET).min(buffer.len());
                let actual_end = if end < buffer.len() {
                    // Advance to next newline to avoid splitting words across chunk boundaries
                    buffer[end..]
                        .iter()
                        .position(|&b| b == b'\n')
                        .map(|pos| end + pos + 1)
                        .unwrap_or(buffer.len())
                } else {
                    buffer.len()
                };

                chunks.push(&buffer[start..actual_end]);
                start = actual_end;
            }

            // Normalize and collect words concurrently across all CPU threads
            let file_words: BTreeSet<String> = chunks
                .into_par_iter()
                .map(|chunk| process_chunk(chunk, skip_invalid))
                .reduce(BTreeSet::new, |mut a, mut b| {
                    if a.len() < b.len() {
                        b.append(&mut a);
                        b
                    } else {
                        a.append(&mut b);
                        a
                    }
                });

            if words.is_empty() {
                words = file_words;
            } else {
                words.extend(file_words);
            }
        }
    }

    let out_file = BufWriter::new(File::create(&dest_path)?);
    let mut builder = fst::SetBuilder::new(out_file)?;
    for word in words {
        builder.insert(word.as_bytes())?;
    }
    builder.finish()?;

    Ok(())
}
