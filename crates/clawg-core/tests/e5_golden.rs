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

use clawg_core::eval_api::{EmbedRole, EvalEmbedder, EvalTokenCounter};
use serde::{Deserialize, Serialize};

const MODEL_DIR_VAR: &str = "CLAWG_E5_MODEL_DIR";
const BLESS_VAR: &str = "CLAWG_BLESS";
const MODEL_SUBDIR: [&str; 2] = ["models", "multilingual-e5-small"];
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/e5_golden.json");
const DIMENSIONS: usize = 384;
const TOLERANCE: f32 = 1e-6;

/// What decides which kernels ONNX Runtime picks, so whether another machine
/// can be expected to give the same bits. The system counts too: ONNX Runtime
/// is built apart for each one, and on Linux a CPU with the same features as
/// the Windows one that made the fixture moved the last bits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cpu {
    os: String,
    arch: String,
    avx2: bool,
    avx512f: bool,
    fma: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Comparison {
    ExactBits,
    MaxDifference(f32),
}

fn comparison_for(fixture: &Cpu, this: &Cpu) -> Comparison {
    if fixture == this {
        Comparison::ExactBits
    } else {
        Comparison::MaxDifference(TOLERANCE)
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn this_cpu() -> Cpu {
    Cpu {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        avx2: is_x86_feature_detected!("avx2"),
        avx512f: is_x86_feature_detected!("avx512f"),
        fma: is_x86_feature_detected!("fma"),
    }
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn this_cpu() -> Cpu {
    Cpu {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        avx2: false,
        avx512f: false,
        fma: false,
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
    cpu: Cpu,
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
    let fixture = Cpu { os: "windows".into(), arch: "x86_64".into(), avx2: true, avx512f: true, fma: true };
    assert_eq!(comparison_for(&fixture, &fixture.clone()), Comparison::ExactBits);

    let no_avx512 = Cpu { avx512f: false, ..fixture.clone() };
    assert_eq!(comparison_for(&fixture, &no_avx512), Comparison::MaxDifference(TOLERANCE));

    let no_avx2 = Cpu { avx2: false, ..fixture.clone() };
    assert_eq!(comparison_for(&fixture, &no_avx2), Comparison::MaxDifference(TOLERANCE));

    let no_fma = Cpu { fma: false, ..fixture.clone() };
    assert_eq!(comparison_for(&fixture, &no_fma), Comparison::MaxDifference(TOLERANCE));

    let other_arch = Cpu { arch: "aarch64".into(), ..fixture.clone() };
    assert_eq!(comparison_for(&fixture, &other_arch), Comparison::MaxDifference(TOLERANCE));

    let other_os = Cpu { os: "linux".into(), ..fixture.clone() };
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

fn inputs(counter: &EvalTokenCounter) -> Vec<(String, Role, Vec<String>)> {
    let one = |name: &str, role: Role, text: String| (name.to_string(), role, vec![text]);
    let batch_of_17: Vec<String> = (0..17)
        .map(|i| match i % 4 {
            0 => format!("Nota {i}: el daemon guarda las memorias en una base SQLite."),
            1 => format!("Entry {i}: the bridge asks the daemon to index the project files."),
            2 => format!("記録{i}：検索は意味と語句の両方で行われる。"),
            _ => text_of_tokens(counter, 40 + 37 * i),
        })
        .collect();
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
        ("a batch of 17".to_string(), Role::Passage, batch_of_17),
    ]
}

async fn embed(embedder: &EvalEmbedder, role: Role, texts: &[String]) -> Vec<Vec<f32>> {
    embedder.embed_batch(texts, role.embed_role()).await.expect("embed a case")
}

async fn bless(embedder: &EvalEmbedder, base: &Path) -> ! {
    let counter = EvalTokenCounter::real(base).expect("the tokenizer loads");
    let mut cases = Vec::new();
    for (name, role, texts) in inputs(&counter) {
        let vectors = embed(embedder, role, &texts).await;
        let bits = vectors.iter().map(|v| v.iter().map(|x| x.to_bits()).collect()).collect();
        cases.push(Case { name, role, texts, bits });
    }
    let fixture = Fixture { cpu: this_cpu(), cases };
    std::fs::create_dir_all(Path::new(FIXTURE).parent().expect("the fixture has a directory"))
        .expect("create the fixture directory");
    std::fs::write(FIXTURE, render(&fixture)).expect("write the fixture");
    panic!("fixture written: review the diff of {FIXTURE} and run the test again without {BLESS_VAR}");
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
    let comparison = comparison_for(&fixture.cpu, &this_cpu());
    match comparison {
        Comparison::ExactBits => {
            println!("compared exact bits: this CPU is the one that made the fixture ({:?})", fixture.cpu)
        }
        Comparison::MaxDifference(max) => println!(
            "compared with a maximum difference of {max}: the fixture was made on {:?}, this is {:?}",
            fixture.cpu,
            this_cpu()
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
