use criterion::{Criterion, criterion_group, criterion_main};
use unipastren::PasswordChecker;

fn benchmark_password_checker(c: &mut Criterion) {
    let mut dict_words: Vec<String> = (0..199_992).map(|i| format!("sana{:06}", i)).collect();
    for w in ["aurinko", "metsä", "järvi", "kissa", "koira", "talo", "vene", "pilvi"] {
        dict_words.push(w.to_string());
    }

    let checker = PasswordChecker::new()
        .with_max_chars(100)
        .with_warnings(false)
        .with_segments(false)
        .with_dictionary(dict_words);

    let mut group = c.benchmark_group("utf8_pwcheck");

    group.bench_function("short_repeat_666", |b| {
        b.iter(|| checker.analyze(std::hint::black_box("666")))
    });

    group.bench_function("8_word_compound_passphrase", |b| {
        b.iter(|| checker.analyze(std::hint::black_box("aurinkometsäjärvikissakoiratalovenepilvi")))
    });

    group.bench_function("ax100", |b| {
        b.iter(|| checker.analyze(std::hint::black_box("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")))
    });

    group.bench_function("abx50", |b| {
        b.iter(|| checker.analyze(std::hint::black_box("abababababababababababababababababababababababababababababababababababababababababababababababababab")))
    });

    group.bench_function("multi_script_utf8", |b| {
        b.iter(|| checker.analyze(std::hint::black_box("雪山אלףбелый강물🌺kissa")))
    });

    let long_input = "雪山אלףбелый강물🌺kissa_aurinko_metsä_1234567890_".repeat(10);
    group.bench_function("long_input_capped_at_100_chars", |b| {
        b.iter(|| checker.analyze(std::hint::black_box(&long_input)))
    });

    group.finish();
}

criterion_group!(benches, benchmark_password_checker);
criterion_main!(benches);
