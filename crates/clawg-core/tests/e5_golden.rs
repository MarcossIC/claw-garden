//! The vectors of the embedding model, pinned: the texts, their role and every
//! component of every vector as the bits of an `f32`.
//!
//! The texts are built once, when the fixture is blessed, and read back from it
//! afterwards, so they do not depend on any code that changes later. It only
//! uses `eval_api`, the door whose shape does not change with the embedder
//! behind it.
//!
//! Needs the real model: `CLAWG_E5_MODEL_DIR` is the directory that holds
//! `model.onnx` under `onnx/` and the tokenizer files, that is
//! `<data dir>/models/multilingual-e5-small`.
//!
//! To write the fixture again:
//! `CLAWG_BLESS=1 CLAWG_E5_MODEL_DIR=<dir> cargo test -p clawg-core --features eval --test e5_golden -- --ignored`

use std::path::{Path, PathBuf};

use clawg_core::eval_api::{join_pieces, passage_order, EmbedRole, EvalCpu, EvalEmbedder, EvalTokenCounter};
use serde::{Deserialize, Serialize};

const MODEL_DIR_VAR: &str = "CLAWG_E5_MODEL_DIR";
const BLESS_VAR: &str = "CLAWG_BLESS";
const MODEL_SUBDIR: [&str; 2] = ["models", "multilingual-e5-small"];
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/e5_golden.json");
const DIMENSIONS: usize = 384;
const TOLERANCE: f32 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Comparison {
    ExactBits,
    MaxDifference(f32),
}

fn comparison_for(fixture: &EvalCpu, this: &EvalCpu) -> Comparison {
    if fixture == this {
        Comparison::ExactBits
    } else {
        Comparison::MaxDifference(TOLERANCE)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    Query,
    Passage,
}

impl Role {
    fn embed_role(self) -> EmbedRole {
        match self {
            Role::Query => EmbedRole::Query,
            Role::Passage => EmbedRole::Passage,
        }
    }
}

/// One call to `embed_batch`: the texts, their role and the vectors it gave.
#[derive(Debug, Serialize, Deserialize)]
struct Case {
    name: String,
    role: Role,
    texts: Vec<String>,
    bits: Vec<Vec<u32>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Fixture {
    cpu: EvalCpu,
    cases: Vec<Case>,
}

/// One case per line, so a diff of the fixture shows which case moved.
fn render(fixture: &Fixture) -> String {
    let cpu = serde_json::to_string(&fixture.cpu).expect("serialize the cpu");
    let cases: Vec<String> =
        fixture.cases.iter().map(|c| serde_json::to_string(c).expect("serialize a case")).collect();
    format!("{{\n\"cpu\": {cpu},\n\"cases\": [\n{}\n]\n}}\n", cases.join(",\n"))
}

fn max_difference(expected: &[u32], actual: &[f32]) -> f32 {
    expected
        .iter()
        .zip(actual)
        .map(|(e, a)| (f32::from_bits(*e) - a).abs())
        .fold(0.0, f32::max)
}

#[test]
fn the_comparison_is_by_bits_only_on_the_cpu_that_made_the_fixture() {
    let fixture = EvalCpu { os: "windows".into(), arch: "x86_64".into(), avx2: true, avx512f: true, fma: true };
    assert_eq!(comparison_for(&fixture, &fixture.clone()), Comparison::ExactBits);

    let no_avx512 = EvalCpu { avx512f: false, ..fixture.clone() };
    assert_eq!(comparison_for(&fixture, &no_avx512), Comparison::MaxDifference(TOLERANCE));

    let no_avx2 = EvalCpu { avx2: false, ..fixture.clone() };
    assert_eq!(comparison_for(&fixture, &no_avx2), Comparison::MaxDifference(TOLERANCE));

    let no_fma = EvalCpu { fma: false, ..fixture.clone() };
    assert_eq!(comparison_for(&fixture, &no_fma), Comparison::MaxDifference(TOLERANCE));

    let other_arch = EvalCpu { arch: "aarch64".into(), ..fixture.clone() };
    assert_eq!(comparison_for(&fixture, &other_arch), Comparison::MaxDifference(TOLERANCE));

    let other_os = EvalCpu { os: "linux".into(), ..fixture.clone() };
    assert_eq!(comparison_for(&fixture, &other_os), Comparison::MaxDifference(TOLERANCE));
}

#[test]
fn the_fixture_holds_one_vector_of_the_model_size_for_every_text() {
    let fixture: Fixture = serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap_or_else(|e| {
        panic!("read {FIXTURE}: {e}; create it with {BLESS_VAR}=1 {MODEL_DIR_VAR}=<dir> and --ignored")
    }))
    .expect("the fixture is JSON");
    assert!(!fixture.cases.is_empty());
    for case in &fixture.cases {
        assert_eq!(case.texts.len(), case.bits.len(), "{}: a vector per text", case.name);
        for vector in &case.bits {
            assert_eq!(vector.len(), DIMENSIONS, "{}", case.name);
        }
    }
}

const BATCH_OF_17: &str = "a batch of 17";
const BATCH_OF_17_LONGEST_FIRST: &str = "a batch of 17, longest first";

/// The same 17 texts embedded in the order a docs refresh sends them move no
/// vector by more than the tolerance, whatever CPU the fixture was made on.
#[test]
fn a_batch_longest_first_stays_within_the_tolerance_of_the_batch_in_order() {
    let fixture: Fixture = serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap_or_else(|e| {
        panic!("read {FIXTURE}: {e}; create it with {BLESS_VAR}=1 {MODEL_DIR_VAR}=<dir> and --ignored")
    }))
    .expect("the fixture is JSON");
    let case = |name: &str| {
        fixture.cases.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("the fixture has no case named {name:?}"))
    };
    let (in_order, longest_first) = (case(BATCH_OF_17), case(BATCH_OF_17_LONGEST_FIRST));
    assert_eq!(in_order.texts.len(), longest_first.texts.len(), "the same texts");

    let mut same_bits = 0;
    for (text, bits) in longest_first.texts.iter().zip(&longest_first.bits) {
        let at = in_order.texts.iter().position(|t| t == text).unwrap_or_else(|| panic!("{text:?} is not in {BATCH_OF_17:?}"));
        let moved = max_difference(&in_order.bits[at], &bits.iter().map(|b| f32::from_bits(*b)).collect::<Vec<_>>());
        assert!(moved <= TOLERANCE, "{text:?}: the order moved its vector by {moved}, over {TOLERANCE}");
        same_bits += usize::from(in_order.bits[at] == *bits);
    }
    println!("{same_bits} of {} vectors have the same bits in either order", longest_first.texts.len());
}

/// `model_dir` is `<data dir>/models/multilingual-e5-small`; `EvalEmbedder::real`
/// wants the data dir.
fn data_dir_of(model_dir: &Path) -> PathBuf {
    let mut dir = model_dir.to_path_buf();
    for part in MODEL_SUBDIR.iter().rev() {
        assert_eq!(
            dir.file_name().and_then(|n| n.to_str()),
            Some(*part),
            "{MODEL_DIR_VAR} must end in {}",
            MODEL_SUBDIR.join("/")
        );
        dir.pop();
    }
    dir
}

fn model_dir() -> PathBuf {
    let value = std::env::var_os(MODEL_DIR_VAR).unwrap_or_else(|| {
        panic!("{MODEL_DIR_VAR} is not set: this test needs the real model, it does not skip without it")
    });
    PathBuf::from(value)
}

/// The text of exactly `tokens` content tokens (no special tokens), made of
/// words the tokenizer keeps whole.
fn text_of_tokens(counter: &EvalTokenCounter, tokens: usize) -> String {
    const WORDS: [&str; 8] = ["casa", "perro", "sol", "mesa", "agua", "libro", "tierra", "viento"];
    let mut text = String::new();
    let mut next = 0;
    let mut stuck = 0;
    while counter.count(&text) < tokens {
        let candidate = if text.is_empty() {
            WORDS[next % WORDS.len()].to_string()
        } else {
            format!("{text} {}", WORDS[next % WORDS.len()])
        };
        next += 1;
        if counter.count(&candidate) <= tokens {
            text = candidate;
            stuck = 0;
        } else {
            stuck += 1;
            assert!(stuck < 2 * WORDS.len(), "no word fits the {tokens} tokens");
        }
    }
    assert_eq!(counter.count(&text), tokens);
    text
}

fn inputs(counter: &EvalTokenCounter, embedder: &EvalEmbedder) -> Vec<(String, Role, Vec<String>)> {
    let one = |name: &str, role: Role, text: String| (name.to_string(), role, vec![text]);
    let batch_of_17: Vec<String> = (0..17)
        .map(|i| match i % 4 {
            0 => format!("Nota {i}: el daemon guarda las memorias en una base SQLite."),
            1 => format!("Entry {i}: the bridge asks the daemon to index the project files."),
            2 => format!("記録{i}：検索は意味と語句の両方で行われる。"),
            _ => text_of_tokens(counter, 40 + 37 * i),
        })
        .collect();
    let refresh_order = passage_order(embedder, counter, &batch_of_17);
    let batch_of_17_longest_first: Vec<String> = refresh_order.iter().map(|&k| batch_of_17[k].clone()).collect();
    vec![
        one("short query in English", Role::Query, "how do I restart the daemon".into()),
        one("short query in Spanish", Role::Query, "¿dónde se guarda la memoria del agente?".into()),
        one("single word", Role::Query, "memoria".into()),
        one(
            "Spanish passage",
            Role::Passage,
            "Decidimos que el daemon sea el único dueño de la base: cada puente le pide las búsquedas \
             por HTTP y nunca abre el archivo. Así no hay dos procesos escribiendo a la vez."
                .into(),
        ),
        one("Japanese", Role::Passage, "今日は天気がいいので、散歩に行きながら記憶の検索について考えましょう。".into()),
        one("Chinese", Role::Query, "本地优先的代理记忆系统如何处理重复的内容".into()),
        one("emoji", Role::Passage, "🚀🔥 deploy done ✅ 🎉 👨‍👩‍👧‍👦 🇦🇷 ¡buenísimo!".into()),
        one("about 2000 tokens", Role::Passage, text_of_tokens(counter, 2000)),
        one("509 content tokens", Role::Passage, text_of_tokens(counter, 509)),
        one("510 content tokens", Role::Passage, text_of_tokens(counter, 510)),
        one("511 content tokens", Role::Passage, text_of_tokens(counter, 511)),
        one("511 content tokens as a query", Role::Query, text_of_tokens(counter, 511)),
        one("special tokens as text", Role::Query, "<s> </s> <pad> <unk> <mask>".into()),
        one("special tokens inside a passage", Role::Passage, "antes <s> medio </s> después <pad> <unk> <mask> fin".into()),
        one("empty", Role::Passage, String::new()),
        (BATCH_OF_17.to_string(), Role::Passage, batch_of_17),
        (BATCH_OF_17_LONGEST_FIRST.to_string(), Role::Passage, batch_of_17_longest_first),
    ]
}

async fn embed(embedder: &EvalEmbedder, role: Role, texts: &[String]) -> Vec<Vec<f32>> {
    embedder.embed_batch(texts, role.embed_role()).await.expect("embed a case")
}

async fn bless(embedder: &EvalEmbedder, base: &Path) -> ! {
    let counter = EvalTokenCounter::real(base).expect("the tokenizer loads");
    let mut cases = Vec::new();
    for (name, role, texts) in inputs(&counter, embedder) {
        let vectors = embed(embedder, role, &texts).await;
        let bits = vectors.iter().map(|v| v.iter().map(|x| x.to_bits()).collect()).collect();
        cases.push(Case { name, role, texts, bits });
    }
    let fixture = Fixture { cpu: EvalCpu::this(), cases };
    std::fs::create_dir_all(Path::new(FIXTURE).parent().expect("the fixture has a directory"))
        .expect("create the fixture directory");
    std::fs::write(FIXTURE, render(&fixture)).expect("write the fixture");
    panic!("fixture written: review the diff of {FIXTURE} and run the test again without {BLESS_VAR}");
}

/// A long save in English and in Spanish, with paths, identifiers and signs.
/// Every sentence is numbered, so no stretch of it repeats.
fn mixed_save() -> String {
    (0..40)
        .map(|i| {
            format!(
                "Step {i:03}: the worker in crates/clawg-core/src/memory/worker.rs calls \
                 Store::insert_unless_present() for save #{i:03}, then link_pieces(); see \
                 `memory_piece.context` and HEAD_SEPARATOR (\"\\n…\\n\"). Paso {i:03}: el daemon \
                 guarda la memoria número {i:03} en ~/.clawg-dev/memory.db; ¿por qué? Porque \
                 split_save() corta las piezas, ¡así de fácil!\n\n"
            )
        })
        .collect()
}

/// Whether a cut at byte `b` of `text` leaves every word of it whole.
fn is_word_boundary(text: &str, b: usize) -> bool {
    b == 0
        || b == text.len()
        || text[..b].chars().next_back().is_some_and(char::is_whitespace)
        || text[b..].chars().next().is_some_and(char::is_whitespace)
}

/// The text on both sides of byte `b`, for a failure message.
fn around(text: &str, b: usize) -> String {
    let before: String = text[..b].chars().rev().take(16).collect::<Vec<_>>().into_iter().rev().collect();
    let after: String = text[b..].chars().take(16).collect();
    format!("{before:?} | {after:?}")
}

#[test]
#[ignore = "needs the real model in CLAWG_E5_MODEL_DIR (<data dir>/models/multilingual-e5-small)"]
fn the_real_tokenizer_cuts_no_piece_inside_a_word() {
    let counter = EvalTokenCounter::real(&data_dir_of(&model_dir())).expect("the tokenizer loads");
    let text = mixed_save();
    let (pieces, head) = counter.split_save(&text);
    assert!(pieces.len() > 10, "the fixture is a long save: {} pieces", pieces.len());

    let head = head.expect("a long save has a head");
    assert!(text.starts_with(&head));
    assert!(is_word_boundary(&text, head.len()), "the head ends inside a word: {}", around(&text, head.len()));
    assert!(counter.count(&head) <= 48, "the head has {} tokens", counter.count(&head));

    let (mut from, mut covered) = (0, 0);
    for (k, piece) in pieces.iter().enumerate() {
        let start = from + text[from..].find(piece.as_str()).unwrap_or_else(|| panic!("piece {k} is not in the text"));
        let end = start + piece.len();
        assert!(start <= covered, "bytes {covered}..{start} before piece {k} are in no piece");
        assert!(is_word_boundary(&text, start), "piece {k} starts inside a word: {}", around(&text, start));
        assert!(is_word_boundary(&text, end), "piece {k} ends inside a word: {}", around(&text, end));
        assert!(counter.count(piece) <= 96, "piece {k} has {} tokens, from {}", counter.count(piece), around(&text, start));
        from = start + piece.chars().next().map_or(1, char::len_utf8);
        covered = end;
    }
    assert_eq!(covered, text.len(), "the last piece reaches the end of the text");
    let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
    assert_eq!(join_pieces(&pieces), text, "the save reads back whole");
}

#[tokio::test]
#[ignore = "needs the real model in CLAWG_E5_MODEL_DIR (<data dir>/models/multilingual-e5-small)"]
async fn the_vectors_are_the_ones_fastembed_gave() {
    let base = data_dir_of(&model_dir());
    let embedder = EvalEmbedder::real(&base).await.expect("the model loads");
    if std::env::var_os(BLESS_VAR).is_some_and(|v| v == "1") {
        bless(&embedder, &base).await;
    }

    let fixture: Fixture =
        serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap_or_else(|e| {
            panic!("read {FIXTURE}: {e}; create it with {BLESS_VAR}=1 and --ignored")
        }))
        .expect("the fixture is JSON");
    let comparison = comparison_for(&fixture.cpu, &EvalCpu::this());
    match comparison {
        Comparison::ExactBits => {
            println!("compared exact bits: this CPU is the one that made the fixture ({:?})", fixture.cpu)
        }
        Comparison::MaxDifference(max) => println!(
            "compared with a maximum difference of {max}: the fixture was made on {:?}, this is {:?}",
            fixture.cpu,
            EvalCpu::this()
        ),
    }

    for case in &fixture.cases {
        let vectors = embed(&embedder, case.role, &case.texts).await;
        assert_eq!(vectors.len(), case.bits.len(), "{}", case.name);
        for (i, (vector, expected)) in vectors.iter().zip(&case.bits).enumerate() {
            assert_eq!(vector.len(), DIMENSIONS, "{} #{i}", case.name);
            match comparison {
                Comparison::ExactBits => {
                    let actual: Vec<u32> = vector.iter().map(|x| x.to_bits()).collect();
                    assert!(
                        actual == *expected,
                        "{} #{i}: the bits differ (largest difference {})",
                        case.name,
                        max_difference(expected, vector)
                    );
                }
                Comparison::MaxDifference(max) => {
                    let diff = max_difference(expected, vector);
                    assert!(diff <= max, "{} #{i}: largest difference {diff} is over {max}", case.name);
                }
            }
        }
    }
}
