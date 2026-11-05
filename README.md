# unipastren

**Universal UTF-8 Password & Passphrase Strength Evaluator in Rust**

unipastren is a fast, memory-safe, language-agnostic password and passphrase strength checker for Rust. Unlike traditional English-centric evaluators, unipastren uses dynamic programming (minimum-entropy path segmentation) across all Unicode scripts, paired with zero-copy FST and O(n) Aho-Corasick compound word splitting, Unicode NFC normalization, and automatic memory zeroization.

---

## Performance Highlights

- **Checker Init Time:** ~11.3 microseconds using compile-time embedded FST over 15.2 million unique words.
- **Analysis Latency:** Under 200 microseconds for a 24-character compound passphrase across mixed Unicode blocks.
- **Word Ingestion Scale:** Ingests and compiles dozens of millions of lines via multi-threaded Rayon compilation in `build.rs`.
- **Passphrase Resolution:** Accurately split "trústno1nisrinav.i.r.t.a.h.ē.p.o." into 3 dictionary words ("trustno1", "nisrina", "virtahepo") (NFKD normalization) at 23.86 bits/word.

---

## Features

- **All Human Languages & UTF-8 Scripts:** Natively evaluates Latin, Cyrillic, Greek, Arabic, Hebrew, Devanagari, Hangul, Hiragana/Katakana, CJK Ideographs, Emojis, and arbitrary Unicode planes.

- **Instant Zero-Cost Dictionary Loading (FST):**
  - **Embedded FST:** Compile multi-million word dictionaries directly into the binary at build time. Loading at runtime takes ~11 microseconds with zero runtime trie allocation.
  - **Runtime Dictionaries:** Load one or more custom files at runtime (`with_dictionary` / `with_dictionaries`) using Aho-Corasick pattern matching.
  - **Without a Dictionary (Conservative Fallback):** Unverified single-script words are never optimistically assumed to come from a huge list; they are conservatively scored against a configurable top-1000 pool (~10 bits/word).

- **Multi-Script Collage Awareness:** Mixing multiple distinct Unicode scripts (e.g., "雪山אלףбелый강물🌺kissa") preserves full character-pool entropy rather than collapsing unfamiliar scripts into small word pools.

- **Unicode NFC Normalization & Diacritic Folding:** Precomposed characters ("ä") and decomposed sequences ("a" + "\u{0308}") evaluate identically. Accented dictionary matches (such as "trúst" matching "trust") are recognized automatically.

- **Pattern & Repetition Compression:**
  - **Tandem Repeats:** Immediate periodic repetitions ("666", "aieaie", "ababab") are compressed to a single period plus log2(repeats) and penalized.
  - **Gapped Repeats (LZ77 Backreferences):** Repeated words or substrings earlier in the password ("aurinko metsä aurinko metsä" or "2026197020262025") are caught and penalized.
  - **Monotonic Unicode Sequences:** Detects sequential codepoints across scripts ("12345", "abcdef", "абвгде", "αβγδε").
  - **Interleaved Steganography:** Detects words and sequences hidden behind constant separators (e.g. "b22a22t22t22e22r22y22" or "h.g.f.e.d.c.b.a").

- **Defense-in-Depth Security:**
  - **Memory Zeroization (zeroize):** All internal temporary buffers, DP states, and password structs implement Zeroize / ZeroizeOnDrop. Includes `analyze_and_zeroize(&mut String)` to wipe caller buffers immediately.
  - **Sensitive Substring Redaction:** Disable warning generation (`.with_warnings(false)`) and diagnostic segment retention (`.with_segments(false)`) in production so password substrings are never stored in logs or memory.
  - **Bounded CPU Usage:** Configurable character limit (default: 100 NFC scalar values) guarantees microsecond execution time and prevents Denial-of-Service on oversized inputs.

---

## Installation

Add unipastren to your Cargo.toml:

```toml
[dependencies]
unipastren = "0.2"
```

---

## Usage

### 1. Using the Embedded FST Dictionary (Fastest)

If built with embedded dictionaries (see Build Options below), `with_embedded_dictionary()` loads millions of words in microseconds:

```rust
use unipastren::{PasswordChecker, StrengthRating};

fn main() {
    let checker = PasswordChecker::new().with_embedded_dictionary();

    let mut password = String::from("trústno1nisrinavirtahēpo");
    let analysis = checker.analyze_and_zeroize(&mut password);

    assert!(password.is_empty()); // Zeroized in memory
    println!("Dictionary size: {} words", checker.dictionary_size());
    println!("Total Entropy:   {:.2} bits", analysis.entropy_bits);
    println!("Rating:         {:?}", analysis.rating);
}
```

### 2. Using Runtime Dictionaries

```rust
use unipastren::PasswordChecker;

fn main() {
    // Single dictionary
    let checker = PasswordChecker::new()
        .with_dictionary(["aurinko", "metsä", "järvi", "kissa"]);

    // Or multiple dictionaries
    let dict1 = ["apple", "banana", "cherry"];
    let dict2 = ["apfel", "banane", "kirsche"];
    let checker_multi = PasswordChecker::new()
        .with_dictionaries([&dict1[..], &dict2[..]]);
}
```

### 3. Running Without a Dictionary

```rust
use unipastren::PasswordChecker;

fn main() {
    let checker = PasswordChecker::new(); // Fallback mode (~10 bits per unverified word)
    let analysis = checker.analyze("correct horse battery staple");
    println!("Entropy: {:.2} bits", analysis.entropy_bits);
}
```

### 4. Production Mode (Zero Substring Retention)

When running inside an authentication service where warnings or diagnostic segments might accidentally be logged, disable warnings and segments:

```rust
let checker = PasswordChecker::new()
    .with_embedded_dictionary()
    .with_warnings(false)
    .with_segments(false);

let mut secret = String::from("correct-horse-battery-staple");
let result = checker.analyze_and_zeroize(&mut secret);

assert!(result.warnings.is_empty());
assert!(result.segments.is_empty());
```

---

## Build Options (Embedding Dictionaries)

Dictionaries can be compiled ahead of time into a compact, zero-copy FST:

- `UNIPASTREN_BUILD_DICTS`: Colon-separated list of dictionary files to compile into the binary.
- `UNIPASTREN_BUILD_SKIP_INVALID`: When set to `1` or `true`, skips lines containing invalid UTF-8 bytes instead of lossy conversion.

Example:

```bash
UNIPASTREN_BUILD_SKIP_INVALID=1 \
UNIPASTREN_BUILD_DICTS="/usr/share/dict/rockyou.txt:/usr/share/dict/ngerman" \
cargo build --release
```

If neither environment variable is set, the crate compiles cleanly with a lightweight 0-word stub FST.

A Finnish wordlist can be generated with the script in `assets/dumpwords.py`.

---

## CLI Debug Utility

The CLI utility supports inspecting segmentation, normalization, and script pools:

```bash
# Analyze using the embedded FST dictionary
UNIPASTREN_BUILD_DICTS="/usr/share/dict/suomi.txt" cargo run --release --bin unipastren-cli -- "virtahÉpo"

# Analyze with one or more custom runtime dictionary files
cargo run --release --bin unipastren-cli -- --dict /usr/share/dict/words --dict /usr/share/dict/ngerman "passwort"

# Analyze without any dictionary
cargo run --release --bin unipastren-cli -- --no-dict "aurinko metsä järvi kissa"
```

---

## Testing & Benchmarks

Run the unit test suite:

```bash
cargo test
```

Run benchmarks:

```bash
cargo bench
```

---

## License

Licensed under GPL-3.0-only.
