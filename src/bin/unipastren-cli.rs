#![allow(clippy::indexing_slicing)]
use std::{fs, path::PathBuf, process, time::Instant};

use clap::Parser;
use unicode_normalization::UnicodeNormalization;
use unipastren::{PasswordChecker, SegmentKind, StrengthRating, UnicodePool};
use zeroize::Zeroize;

#[derive(Parser, Debug)]
#[command(
    name = "unipastren-cli",
    about = "Unicode-aware password entropy and strength analyzer",
    version
)]
struct Cli {
    /// Load a custom newline-separated UTF-8 dictionary file (can be specified multiple times)
    #[arg(long = "dict", value_name = "PATH")]
    dict: Vec<PathBuf>,

    /// Run without any dictionary (default uses a 200,000-word demo dictionary)
    #[arg(long = "no-dict", conflicts_with = "dict")]
    no_dict: bool,

    /// Maximum NFC characters to analyze
    #[arg(long = "max-chars", default_value_t = 100, value_name = "N")]
    max_chars: usize,

    /// Disable generating warning strings
    #[arg(long = "no-warnings")]
    no_warnings: bool,

    /// Password to analyze (prompted securely if omitted)
    #[arg(value_name = "PASSWORD")]
    password: Option<String>,
}

fn build_demo_200k_dictionary() -> Vec<String> {
    let mut words: Vec<String> = (0..199_992).map(|i| format!("sana{:06}", i)).collect();
    for w in ["aurinko", "metsä", "järvi", "kissa", "koira", "talo", "vene", "pilvi"] {
        words.push(w.to_string());
    }
    words
}

fn main() {
    let mut cli = Cli::parse();

    // Take ownership of the password argument or prompt interactively via rpassword
    let mut password = match cli.password.take() {
        Some(p) => p,
        None => rpassword::prompt_password("Password: ").unwrap_or_else(|e| {
            eprintln!("Failed to read password: {}", e);
            process::exit(1);
        }),
    };

    // Configure checker
    let init_start = Instant::now();
    let mut checker =
        PasswordChecker::new().with_max_chars(cli.max_chars).with_warnings(!cli.no_warnings);

    let dict_description = if !cli.dict.is_empty() {
        let mut file_contents = Vec::with_capacity(cli.dict.len());
        let mut total_lines = 0;
        for path in &cli.dict {
            let content = fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("Failed to read dictionary file '{}': {}", path.display(), e);
                process::exit(1);
            });
            file_contents.push(content);
        }

        let dict_slices: Vec<Vec<&str>> = file_contents
            .iter()
            .map(|content| {
                let lines: Vec<&str> = content.lines().collect();
                total_lines += lines.len();
                lines
            })
            .collect();

        checker = checker.with_dictionaries(&dict_slices);

        if cli.dict.len() == 1 {
            format!(
                "Custom file '{}' ({} lines, {} unique)",
                cli.dict[0].display(),
                total_lines,
                checker.dictionary_size()
            )
        } else {
            let paths: Vec<String> =
                cli.dict.iter().map(|p| format!("'{}'", p.display())).collect();
            format!(
                "{} custom files [{}] ({} total lines, {} unique)",
                cli.dict.len(),
                paths.join(", "),
                total_lines,
                checker.dictionary_size()
            )
        }
    } else if !cli.no_dict {
        checker = checker.with_embedded_dictionary();
        if checker.dictionary_size() > 0 {
            format!(
                "Embedded FST dictionary ({} unique words, {:.2} bits/word)",
                checker.dictionary_size(),
                (checker.dictionary_size() as f64).log2()
            )
        } else {
            let words = build_demo_200k_dictionary();
            checker = checker.with_dictionary(words);
            "Built-in 200,000-word demo dictionary (~17.61 bits/word)".to_string()
        }
    } else {
        "None (unverified single-script words use top-1000 fallback = ~10 bits/word)".to_string()
    };

    let init_elapsed = init_start.elapsed();

    println!("================================================================================");
    println!("                        UNIPASTREN DEBUG PASSWORD REPORT                        ");
    println!("================================================================================");
    println!("Input Password       : \"{}\"", password);
    println!("Dictionary Mode      : {}", dict_description);
    println!("Checker Init Time    : {:?}", init_elapsed);
    println!("Max Chars Limit      : {}", cli.max_chars);
    println!("--------------------------------------------------------------------------------");
    println!("1. UTF-8 & UNICODE NORMALIZATION BREAKDOWN");
    println!("--------------------------------------------------------------------------------");

    let raw_bytes = password.len();
    let raw_chars: Vec<char> = password.chars().collect();
    let mut nfc_string: String = password.nfc().collect();
    let nfc_chars: Vec<char> = nfc_string.chars().collect();

    println!("Raw UTF-8 Byte Len   : {} bytes", raw_bytes);
    println!("Raw Scalar Count     : {} chars", raw_chars.len());
    println!(
        "NFC Scalar Count     : {} chars (NFC == Raw: {})",
        nfc_chars.len(),
        raw_chars.len() == nfc_chars.len()
    );
    println!();
    println!(
        "  {:>4} | {:<6} | {:<10} | {:<18} | {:<15} | {:>10}",
        "Idx", "Char", "Codepoint", "UnicodePool", "ScriptFamily", "Pool Size"
    );
    println!("  {}", "-".repeat(74));

    for (idx, &ch) in nfc_chars.iter().take(cli.max_chars).enumerate() {
        let pool = UnicodePool::classify(ch);
        let family = UnicodePool::script_family(ch);
        let pool_size = if ch.is_whitespace() { 2.0 } else { pool.pool_size() };
        let display_ch =
            if ch.is_control() { format!("\\u{{{:04x}}}", ch as u32) } else { ch.to_string() };
        println!(
            "  {:>4} | {:<6} | U+{:06X}   | {:<18} | {:<15} | {:>6.0} ({:>4.1}b)",
            idx,
            display_ch,
            ch as u32,
            format!("{:?}", pool),
            format!("{:?}", family),
            pool_size,
            pool_size.log2()
        );
    }

    nfc_string.zeroize();
    let analyze_start = Instant::now();
    let analysis = checker.analyze_and_zeroize(&mut password);
    let analyze_elapsed = analyze_start.elapsed();

    println!();
    println!("--------------------------------------------------------------------------------");
    println!("2. MINIMUM-ENTROPY DYNAMIC PROGRAMMING SEGMENTATION");
    println!("--------------------------------------------------------------------------------");
    for (idx, seg) in analysis.segments.iter().enumerate() {
        let (kind_label, detail) = match &seg.kind {
            SegmentKind::Literal(s) => ("Literal", format!("\"{}\"", s)),
            SegmentKind::DictionaryWord(s) => ("DictionaryWord", format!("\"{}\"", s)),
            SegmentKind::UnverifiedWord(s) => ("UnverifiedWord", format!("\"{}\"", s)),
            SegmentKind::TandemRepeat { unit, repeats, full_match } => {
                ("TandemRepeat", format!("\"{}\" (\"{}\" x {})", full_match, unit, repeats))
            }
            SegmentKind::PreviousRepeat(s) => ("PreviousRepeat", format!("\"{}\"", s)),
            SegmentKind::Sequence(s) => ("Sequence", format!("\"{}\"", s)),
            SegmentKind::KeyboardSequence(s) => ("KeyboardSequence", format!("\"{}\"", s)),
        };
        println!(
            "  Segment #{:<2} [{:<14}] : {:<32} -> {:>6.2} bits",
            idx + 1,
            kind_label,
            detail,
            seg.entropy_bits
        );
    }

    println!();
    println!("--------------------------------------------------------------------------------");
    println!("3. FINAL STRENGTH VERDICT & SECURITY STATUS");
    println!("--------------------------------------------------------------------------------");
    println!("Total Entropy        : {:.2} bits", analysis.entropy_bits);
    println!("Strength Rating      : {:?}", analysis.rating);
    println!("Better than Weak?    : {:?}", analysis.rating > StrengthRating::Weak);
    println!(
        "Analyzed Length      : {} NFC chars (Truncated: {})",
        analysis.normalized_length, analysis.truncated
    );
    println!("Analysis Time        : {:?}", analyze_elapsed);
    println!(
        "Input Zeroized       : {} (password.len() == {})",
        password.is_empty(),
        password.len()
    );

    if !analysis.warnings.is_empty() {
        println!();
        println!("Warnings ({}):", analysis.warnings.len());
        for w in &analysis.warnings {
            println!("  [!] {}", w);
        }
    }
    println!("================================================================================");
}
