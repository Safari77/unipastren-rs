use super::*;

fn sample_200k_checker() -> PasswordChecker {
    let mut dict_words: Vec<String> = (0..199_992).map(|i| format!("sana{:06}", i)).collect();
    for w in ["aurinko", "metsä", "järvi", "kissa", "koira", "talo", "vene", "pilvi"] {
        dict_words.push(w.to_string());
    }
    PasswordChecker::new().with_max_chars(100).with_dictionary(dict_words)
}

#[test]
fn test_interleaved_sequences_and_words_caught() {
    let no_dict = PasswordChecker::new();

    // 1. "礐a礑a礒a礓a礔" -> CJK sequence "礐礑礒礓礔" interleaved with 'a'
    let res_cjk = no_dict.analyze("礐a礑a礒a礓a礔");
    assert!(
        res_cjk.entropy_bits < 22.0,
        "Expected interleaved CJK sequence < 22 bits, got {}",
        res_cjk.entropy_bits
    );
    assert_eq!(res_cjk.rating, StrengthRating::VeryWeak);
    assert!(res_cjk.segments.iter().any(|s| matches!(s.kind, SegmentKind::Sequence(_))));

    // 2. "h.g.f.e.d.c.b.a" -> reverse sequence "hgfedcba" interleaved with '.'
    let res_hgf = no_dict.analyze("h.g.f.e.d.c.b.a");
    assert!(
        res_hgf.entropy_bits < 16.0,
        "Expected h.g.f.e.d.c.b.a < 16 bits, got {}",
        res_hgf.entropy_bits
    );
    assert_eq!(res_hgf.rating, StrengthRating::VeryWeak);
    assert!(res_hgf.segments.iter().any(|s| matches!(s.kind, SegmentKind::Sequence(_))));

    // 3. "b22a22t22t22e22r22y22" -> "battery" interleaved with "22"
    let res_bat = no_dict.analyze("b22a22t22t22e22r22y22");
    assert!(
        res_bat.entropy_bits < 18.0,
        "Expected b22a22t22t22e22r22y22 < 18 bits, got {}",
        res_bat.entropy_bits
    );
    assert_eq!(res_bat.rating, StrengthRating::VeryWeak);
}

#[test]
fn test_compound_multilingual_unverified_words() {
    let no_dict = PasswordChecker::new();
    // 5 concatenated words across 3 languages (31 chars) should score ~61.2 bits (Reasonable),
    // not 26.57 bits (VeryWeak).
    let res = no_dict.analyze("monkeysikapaviaaniachtungisland");
    assert!(
        res.entropy_bits >= 60.0,
        "Expected >= 60 bits for 31-char 5-word compound passphrase, got {}",
        res.entropy_bits
    );
    assert_eq!(res.rating, StrengthRating::Reasonable);
}

#[test]
fn test_repeated_literal_delimiters_and_words_preserved() {
    let no_dict = PasswordChecker::new();
    let res =
        no_dict.analyze("/usr/share/dict/enable-moby-wordle.txt correct-horse-battery-staple");

    // All 11 words ("usr", "share", "dict", "enable", "moby", "wordle", "txt",
    // "correct", "horse", "battery", "staple") must be recognized as UnverifiedWord
    let unverified_words: Vec<&str> = res
        .segments
        .iter()
        .filter_map(|s| match &s.kind {
            SegmentKind::UnverifiedWord(w) => Some(w.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        unverified_words,
        vec![
            "usr", "share", "dict", "enable", "moby", "wordle", "txt", "correct", "horse",
            "battery", "staple"
        ]
    );

    // Collect all "/" and "-" literal segments
    let slash_segments: Vec<&Segment> = res
        .segments
        .iter()
        .filter(|s| matches!(s.kind, SegmentKind::Literal(ref l) if l == "/"))
        .collect();
    assert_eq!(slash_segments.len(), 4);
    assert!(
        slash_segments.first().is_some_and(|s| (s.entropy_bits - 33.0_f64.log2()).abs() < 0.01),
        "First '/' should score ~5.04 bits"
    );
    for s in slash_segments.iter().skip(1) {
        assert!(
            (s.entropy_bits - REPEATED_LITERAL_ENTROPY_BITS).abs() < 1e-6,
            "Repeated '/' should score 1.0 bit, got {}",
            s.entropy_bits
        );
    }

    let dash_segments: Vec<&Segment> = res
        .segments
        .iter()
        .filter(|s| matches!(s.kind, SegmentKind::Literal(ref l) if l == "-"))
        .collect();
    assert_eq!(dash_segments.len(), 5);
    assert!(
        dash_segments.first().is_some_and(|s| (s.entropy_bits - 33.0_f64.log2()).abs() < 0.01),
        "First '-' should score ~5.04 bits"
    );
    for s in dash_segments.iter().skip(1) {
        assert!(
            (s.entropy_bits - REPEATED_LITERAL_ENTROPY_BITS).abs() < 1e-6,
            "Repeated '-' should score 1.0 bit, got {}",
            s.entropy_bits
        );
    }
}

#[test]
fn test_multi_script_not_scored_as_top_100() {
    let checker = sample_200k_checker();
    let res = checker.analyze("雪山אלףбелый강물🌺kissa");
    assert!(
        res.entropy_bits > 100.0,
        "Expected > 100 bits for multi-script password, got {}",
        res.entropy_bits
    );
    assert_eq!(res.rating, StrengthRating::Strong);
    assert!(
        !res.segments.iter().any(|s| matches!(s.kind, SegmentKind::UnverifiedWord(_))),
        "Multi-script password should not classify scripts as top-1000 UnverifiedWord"
    );
}

#[test]
fn test_repeats_and_sequences_penalized() {
    let checker = sample_200k_checker();

    let r_666 = checker.analyze("666");
    assert_eq!(r_666.rating, StrengthRating::VeryWeak);
    // log2(10 digits) + log2(3 repeats) ≈ 4.91 bits
    assert!(r_666.entropy_bits <= 5.0);

    let r_aie = checker.analyze("aieaie");
    assert_eq!(r_aie.rating, StrengthRating::VeryWeak);
    assert!(r_aie.entropy_bits < 12.0);

    let r_dates = checker.analyze("2026197020262025");
    assert_eq!(r_dates.rating, StrengthRating::VeryWeak);

    let r_seq = checker.analyze("абвгде12345");
    assert_eq!(r_seq.rating, StrengthRating::VeryWeak);
}

#[test]
fn test_8_words_with_and_without_dictionary() {
    let passphrase = "aurinko metsä järvi kissa koira talo vene pilvi";

    // Without dictionary
    let no_dict = PasswordChecker::new();
    let res_unverified = no_dict.analyze(passphrase);
    assert!(res_unverified.entropy_bits < 90.0);
    assert_eq!(res_unverified.rating, StrengthRating::Strong);

    // With 200k dictionary: ~147.9 bits (with spaces) or ~140.9 bits (compound) -> Cryptographic
    let with_dict = sample_200k_checker();
    let res_verified = with_dict.analyze(passphrase);
    assert!(res_verified.entropy_bits >= 140.0);
    assert_eq!(res_verified.rating, StrengthRating::Cryptographic);

    let compound = "aurinkometsäjärvikissakoiratalovenepilvi";
    let res_compound = with_dict.analyze(compound);
    assert!((res_compound.entropy_bits - 140.88).abs() < 0.5);
    assert_eq!(res_compound.rating, StrengthRating::Cryptographic);
}

#[test]
fn test_nfc_normalization_and_repeated_words() {
    let checker = sample_200k_checker();
    // "metsa\u{0308}" (NFD) and "metsä" (NFC) must be recognized as identical
    let res = checker.analyze("aurinko metsa\u{0308} aurinko metsä");
    assert_eq!(res.rating, StrengthRating::Weak);
    assert!(
        res.segments
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::PreviousRepeat(ref w) if w == "metsä"))
    );
}

#[test]
fn test_disable_warnings_and_zeroize_input() {
    let checker = sample_200k_checker().with_warnings(false).with_segments(false);

    let mut secret = String::from("aurinko metsä aurinko metsä 666");
    let res = checker.analyze_and_zeroize(&mut secret);

    // Input String buffer is zeroized (emptied and wiped)
    assert!(secret.is_empty());
    // No warnings or segments retained that could leak sensitive words
    assert!(res.warnings.is_empty());
    assert!(res.segments.is_empty());
    assert!(res.entropy_bits > 0.0);
}

#[test]
fn test_keyboard_walks_across_layouts() {
    let checker = PasswordChecker::new();

    let cases = [
        ("qwertyuiop", "QWERTY top row"),
        ("asdfghjklöä", "Nordic home row"),
        ("POIUYTREWQ", "Reverse uppercase QWERTY"),
        ("!@#$%^&*()", "US shifted number row"),
        ("!\"#¤%&/()=?", "Nordic shifted number row"),
        ("1qaz2wsx3edc", "QWERTY vertical columns"),
        ("qwertzuiopü+", "QWERTZ top row"),
        ("yxcvbnm,.-", "QWERTZ bottom row"),
        ("azertyuiop^$", "AZERTY top row"),
        ("qsdfghjklmù*", "AZERTY home row"),
        ("',.pyfgcrl", "Dvorak top row"),
        ("aoeuidhtns", "Dvorak home row"),
        ("qwfpgjluy;", "Colemak top row"),
        ("arstdhneio", "Colemak home row"),
        ("74108520963.", "Numpad vertical columns"),
        ("йцукенгшщзхъ", "Cyrillic JCUKEN top row"),
        ("фывапролджэ", "Cyrillic JCUKEN home row"),
        ("1йфя2цыч", "Cyrillic JCUKEN vertical columns"),
    ];

    for (pw, desc) in cases {
        let res = checker.analyze(pw);
        assert!(
            res.segments.iter().any(|s| matches!(s.kind, SegmentKind::KeyboardSequence(_))),
            "Expected KeyboardSequence segment for {} ({}), got {:?}",
            desc,
            pw,
            res.segments
        );
        assert_eq!(
            res.rating,
            StrengthRating::VeryWeak,
            "Expected VeryWeak for {} ({}), got {:?} ({} bits)",
            desc,
            pw,
            res.rating,
            res.entropy_bits
        );
        assert!(
            res.warnings.iter().any(|w| w.contains("Keyboard walk detected")),
            "Expected keyboard walk warning for {} ({})",
            desc,
            pw
        );
    }
}

#[test]
fn test_multi_turn_and_interleaved_keyboard_walks() {
    let checker = PasswordChecker::new();

    // Multi-turn back-and-forth walk on a single row ("qwertyuiopoiuytrewq")
    let res_turn = checker.analyze("qwertyuiopoiuytrewq");
    assert_eq!(res_turn.segments.len(), 1);
    assert!(
        res_turn
            .segments
            .first()
            .is_some_and(|s| matches!(s.kind, SegmentKind::KeyboardSequence(_)))
    );
    assert_eq!(res_turn.rating, StrengthRating::VeryWeak);

    // Interleaved keyboard walk ("q.w.e.r.t.y.u.i")
    let res_interleaved = checker.analyze("q.w.e.r.t.y.u.i");
    assert!(
        res_interleaved.segments.iter().any(|s| matches!(s.kind, SegmentKind::KeyboardSequence(_))),
        "Expected interleaved keyboard walk to be caught as KeyboardSequence, got {:?}",
        res_interleaved.segments
    );
    assert_eq!(res_interleaved.rating, StrengthRating::VeryWeak);
}

#[test]
fn test_short_adjacent_words_not_false_flagged_as_keyboard_walk() {
    let checker = sample_200k_checker();
    // 3-letter adjacent combinations inside real words or short tokens shouldn't override dictionary words
    let res = checker.analyze("aurinko metsä järvi kissa");
    assert!(
        !res.segments.iter().any(|s| matches!(s.kind, SegmentKind::KeyboardSequence(_))),
        "Dictionary words should not be misclassified as KeyboardSequence"
    );
}

#[test]
fn test_zwj_emoji_grapheme_clusters_and_repeats() {
    let checker = PasswordChecker::new();

    // Single complex ZWJ family emoji ("👨‍👩‍👧‍👦" = 7 codepoints, 1 grapheme) should score as 1 emoji (~9.64 bits)
    let res_single = checker.analyze("👨‍👩‍👧‍👦");
    assert!(
        (res_single.entropy_bits - 800.0_f64.log2()).abs() < 0.1,
        "Expected single ZWJ emoji to score ~9.64 bits, got {}",
        res_single.entropy_bits
    );

    // 3x repeated modified emoji ("👍🏽👍🏽👍🏽") should be caught as a single TandemRepeat
    let res_repeat = checker.analyze("👍🏽👍🏽👍🏽");
    assert!(
        res_repeat
            .segments
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::TandemRepeat { repeats: 3, .. })),
        "Expected TandemRepeat of 3 for 👍🏽👍🏽👍🏽, got {:?}",
        res_repeat.segments
    );
    assert_eq!(res_repeat.rating, StrengthRating::VeryWeak);
}

#[test]
fn random256chars() {
    let checker = PasswordChecker::new().with_max_chars(256);
    let res_single = checker.analyze("3hN2nsnZfHwz7vCDdX2nZ35ueXJ3auKubW0uFzzTzUcSOt0UQVWuBH1DSCED0qcPyHlWxy5E4CCrWhsF7SqVgQ3W7ZjJ4Sm8HhWY4JkNo13XS16XixjOd2yj0wAe7FQ4yrN4XIq5g3lhC5Y0NkmatREyvZxk9XkkA7PCWbKQ1H4ccITpr0FrXLnAddZjT3ol5u8K4ffaZLXgpl1DSKrQGOVRf0ILAl3Kg5brMaMbhx6Eb0yYwKkFIqYNXPQKnBGX");
    assert!(
        // 256*log2(62) ~1524.27
        res_single.entropy_bits > 1000.0,
        "Should be over 1000 bits of entropy, got {}",
        res_single.entropy_bits
    );
}

#[test]
fn test_empty_and_truncation_handling() {
    let checker = PasswordChecker::new().with_max_chars(10);

    let empty_res = checker.analyze("");
    assert_eq!(empty_res.normalized_length, 0);
    assert!(!empty_res.truncated);
    assert_eq!(empty_res.entropy_bits, 0.0);
    assert_eq!(empty_res.rating, StrengthRating::VeryWeak);

    let long_res = checker.analyze("abcdefghijklmnop");
    assert_eq!(long_res.normalized_length, 10);
    assert!(long_res.truncated);
    assert!(long_res.warnings.iter().any(|w| w.contains("exceeded 10 characters")));
}

#[test]
fn test_fst_dictionary_matching() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = fst::SetBuilder::memory();
    for w in ["alpha", "bravo", "charlie", "delta"] {
        builder.insert(w)?;
    }
    let fst_bytes = builder.into_inner()?;

    let checker = PasswordChecker::new().with_fst_bytes(fst_bytes)?;
    assert_eq!(checker.dictionary_size(), 4);

    let res = checker.analyze("AlphaBravoCharlieDelta");
    let dict_words: Vec<&str> = res
        .segments
        .iter()
        .filter_map(|s| match &s.kind {
            SegmentKind::DictionaryWord(w) => Some(w.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(dict_words, vec!["Alpha", "Bravo", "Charlie", "Delta"]);
    Ok(())
}

#[test]
fn test_multirrow_qwerty_diagonal_walks() {
    let checker = PasswordChecker::new();
    let res = checker.analyze("azsxdcfv");
    assert!(
        res.segments.iter().any(|s| matches!(s.kind, SegmentKind::KeyboardSequence(_))),
        "Expected multi-row QWERTY diagonal walk 'azsxdcfv' to be caught, got {:?}",
        res.segments
    );
    assert_eq!(res.rating, StrengthRating::VeryWeak);
}

#[test]
fn test_small_alphabet_cap_and_low_diversity_warnings() {
    let checker = PasswordChecker::new();

    // 1. 3-unique-char password with all distinct 3-grams ("xY7x7Y77xYx", 11 chars):
    //    DP literals alone score ~46.19 bits, so the empirical small-alphabet bound (~30.16 bits) must cap it.
    let res_capped = checker.analyze("xY7x7Y77xYx");
    assert!(
        res_capped.entropy_bits < 31.0,
        "Expected small-alphabet cap < 31 bits for 3-char 11-len string, got {}",
        res_capped.entropy_bits
    );
    assert_eq!(res_capped.rating, StrengthRating::VeryWeak);
    assert!(
        res_capped.warnings.iter().any(|w| w.contains("entropy capped at")),
        "Expected small-alphabet cap warning, got {:?}",
        res_capped.warnings
    );

    // 2. 2-unique-char 36-char mash must not trigger interleaved UnverifiedWord false positive
    //    and must stay within log2(C(26, 2)) + 36 (~44.34 bits) with a low-diversity warning.
    let res_binary = checker.analyze("rrkrkrkkkrkrkkrrkkrkkrrkkrrrkkrrkrkr");
    assert!(
        !res_binary.segments.iter().any(|s| matches!(s.kind, SegmentKind::UnverifiedWord(_))),
        "2-char mash should not be classified as UnverifiedWord, got {:?}",
        res_binary.segments
    );
    assert!(
        res_binary.entropy_bits <= 44.5,
        "Expected 36-char 2-unique-char entropy <= 44.5 bits, got {}",
        res_binary.entropy_bits
    );
    assert!(
        res_binary.warnings.iter().any(|w| w.contains("Low character diversity")),
        "Expected low character diversity warning, got {:?}",
        res_binary.warnings
    );
}

#[test]
fn test_ranked_fst_dictionary_scoring() -> Result<(), Box<dyn std::error::Error>> {
    let mut entries: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for i in 0..9_997u64 {
        entries.insert(format!("word{:05}", i), i + 200);
    }
    entries.insert("common".to_string(), 1);
    entries.insert("medium".to_string(), 100);
    entries.insert("zephyr".to_string(), 8_000);

    let mut ranked_builder = fst::MapBuilder::memory();
    for (word, rank) in &entries {
        ranked_builder.insert(word.as_bytes(), *rank)?;
    }
    let ranked_checker = PasswordChecker::new().with_fst_bytes(ranked_builder.into_inner()?)?;
    assert_eq!(ranked_checker.dictionary_size(), 10_000);

    let res_common = ranked_checker.analyze("common");
    let res_medium = ranked_checker.analyze("medium");
    let res_rare = ranked_checker.analyze("zephyr");

    assert!(
        (res_common.entropy_bits - 1.0).abs() < 0.01,
        "Rank-1 word should score 1.0 bit, got {}",
        res_common.entropy_bits
    );
    assert!(
        (res_medium.entropy_bits - 100.0_f64.log2()).abs() < 0.01,
        "Rank-100 word should score ~6.64 bits, got {}",
        res_medium.entropy_bits
    );
    assert!(
        (res_rare.entropy_bits - 8_000.0_f64.log2()).abs() < 0.01,
        "Rank-8000 word should score ~12.97 bits, got {}",
        res_rare.entropy_bits
    );
    assert!(res_common.entropy_bits < res_medium.entropy_bits);
    assert!(res_medium.entropy_bits < res_rare.entropy_bits);

    // Interleaved match against a ranked FST must also use the word's rank entropy
    let res_interleaved = ranked_checker.analyze("c.o.m.m.o.n");
    assert!(
        res_interleaved.segments.iter().any(|s| matches!(s.kind, SegmentKind::DictionaryWord(_))),
        "Expected DictionaryWord for interleaved ranked match, got {:?}",
        res_interleaved.segments
    );
    assert!(
        res_interleaved.entropy_bits < 8.0,
        "Expected interleaved rank-1 word < 8 bits, got {}",
        res_interleaved.entropy_bits
    );

    // Unranked FST (all values == 0) must fall back to log2(dict_size)
    let mut unranked_builder = fst::MapBuilder::memory();
    for word in entries.keys() {
        unranked_builder.insert(word.as_bytes(), 0)?;
    }
    let unranked_checker = PasswordChecker::new().with_fst_bytes(unranked_builder.into_inner()?)?;
    let res_unranked = unranked_checker.analyze("common");
    assert!(
        (res_unranked.entropy_bits - 10_000.0_f64.log2()).abs() < 0.01,
        "Unranked FST entry should fall back to log2(dict_size) (~13.29 bits), got {}",
        res_unranked.entropy_bits
    );

    Ok(())
}

#[test]
fn test_tandem_repeat_zwj_grapheme_scaling() {
    let checker = PasswordChecker::new();

    // 3x repeated 7-scalar ZWJ emoji ("👨‍👩‍👧‍👦") must scale unit entropy by 1/7
    // so TandemRepeat wins (~11.3 bits) instead of falling back to 3 literals (~28.9 bits).
    let res = checker.analyze("👨‍👩‍👧‍👦👨‍👩‍👧‍👦👨‍👩‍👧‍👦");
    assert!(
        res.segments.iter().any(|s| matches!(s.kind, SegmentKind::TandemRepeat { repeats: 3, .. })),
        "Expected TandemRepeat of 3 for 3x ZWJ family emoji, got {:?}",
        res.segments
    );
    assert!(
        res.entropy_bits < 15.0,
        "Expected 3x ZWJ family emoji < 15 bits after grapheme scaling, got {}",
        res.entropy_bits
    );
    assert_eq!(res.rating, StrengthRating::VeryWeak);
}

#[test]
fn test_short_dict_substrings_do_not_fracture_unverified_word() {
    let mut words: Vec<String> = (0..4_998).map(|i| format!("w{:05}", i)).collect();
    words.push("is".to_string());
    words.push("an".to_string());
    let checker = PasswordChecker::new().with_dictionary(words);

    // "is" is in the 5000-word dictionary (~12.29 bits > 9.40 bits literal cost),
    // so it must not mark covered_by_dict and fracture "kissa" into literals (~23.5 bits).
    let res = checker.analyze("kissa");
    assert!(
        res.segments
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::UnverifiedWord(ref w) if w == "kissa")),
        "Expected 'kissa' to remain a single UnverifiedWord, got {:?}",
        res.segments
    );
    assert!(
        res.entropy_bits < 11.0,
        "Expected ~9.97 bits for unverified 'kissa', got {}",
        res.entropy_bits
    );
}

#[test]
fn test_repeat_pricing_monotonic_without_post_dp_subtraction() {
    let checker = PasswordChecker::new();

    // Appending a repeated 3-char prefix must never reduce total entropy below the base prefix.
    let base = checker.analyze("z9!k");
    let extended = checker.analyze("z9!kz9!");
    assert!(
        extended.segments.iter().any(|s| matches!(s.kind, SegmentKind::PreviousRepeat(_))),
        "Expected PreviousRepeat in 'z9!kz9!', got {:?}",
        extended.segments
    );
    assert!(
        extended.entropy_bits >= base.entropy_bits,
        "Appending characters reduced entropy: base={} bits, extended={} bits",
        base.entropy_bits,
        extended.entropy_bits
    );
}

#[test]
fn test_interleaved_mixed_case_random_not_unverified_word() {
    let checker = PasswordChecker::new();

    // Non-sequential, non-keyboard, random mixed-case letters with a constant delimiter
    // must not be misclassified as an interleaved UnverifiedWord.
    let res = checker.analyze("k-M-p-X-q-Z");
    assert!(
        !res.segments.iter().any(|s| matches!(s.kind, SegmentKind::UnverifiedWord(_))),
        "Mixed-case interleaved random chars should not match UnverifiedWord, got {:?}",
        res.segments
    );
}

#[test]
fn test_unverified_run_entropy_smooth_above_9_chars() {
    let checker = PasswordChecker::new();

    // 9 -> 10 chars with no sequences, keyboard walks, or repeats must ramp smoothly
    // rather than jumping by ~18 bits at length 10.
    let res_9 = checker.analyze("bkpvmtcgz");
    let res_10 = checker.analyze("bkpvmtcgzf");
    let delta = res_10.entropy_bits - res_9.entropy_bits;
    assert!(
        delta > 0.0 && delta < 5.0,
        "Expected smooth entropy increase from 9 to 10 chars, got {} -> {} (delta {})",
        res_9.entropy_bits,
        res_10.entropy_bits,
        delta
    );
}

#[test]
fn test_prefix_relaxed_sequence_overlap_and_trailing_unverified_word() {
    // 1. Sequence "abcd" overlapping dictionary word "deer" in "abcdeer":
    //    Prefix relaxation must allow Sequence("abc") + DictionaryWord("deer").
    let mut dict: Vec<String> = (0..99).map(|i| format!("w{:04}", i)).collect();
    dict.push("deer".to_string());
    let dict_checker = PasswordChecker::new().with_dictionary(dict);

    let res_overlap = dict_checker.analyze("abcdeer");
    assert!(
        res_overlap
            .segments
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::Sequence(ref w) if w == "abc")),
        "Expected prefix Sequence('abc'), got {:?}",
        res_overlap.segments
    );
    assert!(
        res_overlap
            .segments
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::DictionaryWord(ref w) if w == "deer")),
        "Expected overlapping DictionaryWord('deer'), got {:?}",
        res_overlap.segments
    );

    // 2. Prefix sequence must not block a trailing UnverifiedWord on the same script run.
    let no_dict = PasswordChecker::new();
    let res_trailing = no_dict.analyze("abcdefghikissa");
    assert!(
        res_trailing
            .segments
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::Sequence(ref w) if w == "abcdefghi")),
        "Expected leading Sequence('abcdefghi'), got {:?}",
        res_trailing.segments
    );
    assert!(
        res_trailing
            .segments
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::UnverifiedWord(ref w) if w == "kissa")),
        "Expected trailing UnverifiedWord('kissa'), got {:?}",
        res_trailing.segments
    );
}

#[test]
fn test_case_insensitive_sequence_and_stride2_default_off() {
    let checker = PasswordChecker::new();

    let res_lower = checker.analyze("abcdefgh");
    let res_mixed = checker.analyze("aBcDeFgH");
    assert!(
        res_mixed.segments.iter().any(|s| matches!(s.kind, SegmentKind::Sequence(_))),
        "Expected case-insensitive Sequence for 'aBcDeFgH', got {:?}",
        res_mixed.segments
    );
    assert!(
        res_mixed.entropy_bits > res_lower.entropy_bits && res_mixed.entropy_bits < 20.0,
        "Expected mixed-case sequence to score above lowercase ({}) but below literals, got {}",
        res_lower.entropy_bits,
        res_mixed.entropy_bits
    );

    // Stride-2 sequences are disabled by default via ENABLE_STRIDE2_SEQUENCES
    if !ENABLE_STRIDE2_SEQUENCES {
        let res_stride2 = checker.analyze("13579");
        assert!(
            !res_stride2.segments.iter().any(|s| matches!(s.kind, SegmentKind::Sequence(_))),
            "Stride-2 '13579' should not be flagged when ENABLE_STRIDE2_SEQUENCES is false, got {:?}",
            res_stride2.segments
        );
    }
}

#[test]
fn test_raw_slice_entropy_discounts_whitespace() {
    let expected = 26.0_f64.log2() + REPEATED_LITERAL_ENTROPY_BITS;
    let actual = PasswordChecker::raw_slice_entropy(&['a', ' ']);
    assert!(
        (actual - expected).abs() < 1e-6,
        "Expected raw_slice_entropy(&['a', ' ']) == {}, got {}",
        expected,
        actual
    );
}

#[cfg(test)]
mod strength_rating_tests {
    use crate::{ParseStrengthRatingError, StrengthRating};

    #[test]
    fn test_as_str_and_display() {
        let cases = [
            (StrengthRating::VeryWeak, "Very Weak"),
            (StrengthRating::Weak, "Weak"),
            (StrengthRating::Reasonable, "Reasonable"),
            (StrengthRating::Strong, "Strong"),
            (StrengthRating::Cryptographic, "Cryptographic"),
        ];

        for (rating, expected) in cases {
            assert_eq!(rating.as_str(), expected);
            assert_eq!(rating.to_string(), expected);
            assert_eq!(rating.as_ref(), expected);
        }
    }

    #[test]
    fn test_from_str_and_from_text_valid_variations() {
        let test_cases = [
            // VeryWeak variations
            ("WEAK", StrengthRating::Weak),
            ("very_weak", StrengthRating::VeryWeak),
            ("rEaSoNaBlE", StrengthRating::Reasonable),
        ];

        for (input, expected) in test_cases {
            assert_eq!(
                input.parse::<StrengthRating>(),
                Ok(expected),
                "Failed to parse via FromStr: {input:?}"
            );
            assert_eq!(
                StrengthRating::from_text(input),
                Some(expected),
                "Failed to parse via from_text: {input:?}"
            );
        }
    }

    #[test]
    fn test_from_str_and_from_text_invalid() {
        let invalid_inputs = ["", "very weak extra", "cryptographicalllllllllllllllllllllll"];

        for input in invalid_inputs {
            assert_eq!(
                input.parse::<StrengthRating>(),
                Err(ParseStrengthRatingError),
                "Expected parse error for: {input:?}"
            );
            assert_eq!(
                StrengthRating::from_text(input),
                None,
                "Expected None from from_text for: {input:?}"
            );
        }
    }

    #[test]
    fn test_error_display_and_trait() {
        let err = ParseStrengthRatingError;
        assert_eq!(err.to_string(), "invalid password strength rating");
        let err_trait: &dyn std::error::Error = &err;
        assert!(err_trait.source().is_none());
    }
}
