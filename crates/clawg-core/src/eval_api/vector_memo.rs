//! A memory of the vectors the real embedder returned for the passages of a
//! docs index, kept in a file between runs.
//!
//! It is only sound for the exact contract that made it: the same model file,
//! prefixes, dimension, kernels and threads give the same bits, and any other
//! gives a different last bit that no tolerance covers. Everything a vector
//! depends on goes into its key.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{EvalCpu, EvalEmbedder};
use crate::memory::embedder::{apply_role_prefix, EmbedError, EmbedRole, Embedder};
use crate::memory::model_download::MODEL_SHA256;
use crate::metrics::Recorder;

/// Everything but the text that decides the bits of a vector.
struct MemoContract {
    model_id: String,
    dimensions: usize,
    query_prefix: String,
    passage_prefix: String,
    model_sha256: String,
    cpu: EvalCpu,
    threads: usize,
}

type Key = [u8; 32];

/// A text enters a hash behind its length in bytes, so the end of one
/// component cannot be taken for the start of the next.
fn put_text(hasher: &mut Sha256, text: &str) {
    hasher.update((text.len() as u64).to_le_bytes());
    hasher.update(text.as_bytes());
}

/// The contract's components in the order of the key, around the part that
/// belongs to one text: the role and the prefixed text come after the model
/// file's hash and before the machine.
fn hash_contract(contract: &MemoContract, text: Option<(EmbedRole, &str)>) -> Key {
    let mut hasher = Sha256::new();
    put_text(&mut hasher, &contract.model_id);
    hasher.update((contract.dimensions as u64).to_le_bytes());
    put_text(&mut hasher, &contract.query_prefix);
    put_text(&mut hasher, &contract.passage_prefix);
    put_text(&mut hasher, &contract.model_sha256);
    if let Some((role, prefixed)) = text {
        hasher.update([match role {
            EmbedRole::Query => 0u8,
            EmbedRole::Passage => 1u8,
        }]);
        put_text(&mut hasher, prefixed);
    }
    put_text(&mut hasher, &contract.cpu.os);
    put_text(&mut hasher, &contract.cpu.arch);
    hasher.update([u8::from(contract.cpu.avx2), u8::from(contract.cpu.avx512f), u8::from(contract.cpu.fma)]);
    hasher.update((contract.threads as u64).to_le_bytes());
    hasher.finalize().into()
}

/// The key of the vector of `prefixed`, the text as the model receives it.
fn memo_key(contract: &MemoContract, role: EmbedRole, prefixed: &str) -> Key {
    hash_contract(contract, Some((role, prefixed)))
}

/// Names a contract and heads its file, so a file of another contract is
/// recognised before any of its vectors is looked at.
fn contract_digest(contract: &MemoContract) -> Key {
    hash_contract(contract, None)
}

const MARK: &[u8; 8] = b"clawgvec";
const VERSION: u32 = 1;
/// The mark, the version, the contract's digest, the dimension and the count.
const HEADER_BYTES: usize = 8 + 4 + 32 + 4 + 8;
const CHECKSUM_BYTES: usize = 32;

/// Why a memo file gives no vector. A file that does not read whole gives none
/// of them: half a memo is not trusted.
#[derive(Debug, thiserror::Error)]
enum MemoUnreadable {
    #[error("cannot be read: {0}")]
    Read(#[source] io::Error),
    #[error("it is cut short or has a length its header does not say")]
    Truncated,
    #[error("it does not start with the memo mark")]
    Mark,
    #[error("it is of format version {0}, and only 1 is known")]
    Version(u32),
    #[error("it was made under another contract")]
    OtherContract,
    #[error("it holds vectors of {0} dimensions")]
    Dimensions(u32),
    #[error("its checksum does not match its bytes")]
    Checksum,
}

/// The file of a contract: one per contract, so a run reads only its own.
fn memo_file_name(contract: &MemoContract) -> String {
    format!("docs-vectors-{}.bin", &super::hex(&contract_digest(contract))[..16])
}

/// Writes the memo: the header, the entries in key order (so the same content
/// gives the same bytes) and the SHA-256 of all of it.
fn encode(out: &mut (impl Write + ?Sized), contract: &MemoContract, entries: &BTreeMap<Key, Vec<f32>>) -> io::Result<()> {
    struct Hashing<'a, W: Write + ?Sized> {
        out: &'a mut W,
        hasher: Sha256,
    }
    impl<W: Write + ?Sized> Hashing<'_, W> {
        fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.hasher.update(bytes);
            self.out.write_all(bytes)
        }
    }

    let mut w = Hashing { out, hasher: Sha256::new() };
    w.put(MARK)?;
    w.put(&VERSION.to_le_bytes())?;
    w.put(&contract_digest(contract))?;
    w.put(&(contract.dimensions as u32).to_le_bytes())?;
    w.put(&(entries.len() as u64).to_le_bytes())?;
    for (key, vector) in entries {
        if vector.len() != contract.dimensions {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a vector of another dimension"));
        }
        w.put(key)?;
        for x in vector {
            w.put(&x.to_le_bytes())?;
        }
    }
    let checksum: Key = w.hasher.finalize_reset().into();
    w.out.write_all(&checksum)
}

/// Reads a memo made under `contract`; anything else is an error and no vector.
fn decode(bytes: &[u8], contract: &MemoContract) -> Result<HashMap<Key, Vec<f32>>, MemoUnreadable> {
    if bytes.len() < HEADER_BYTES + CHECKSUM_BYTES {
        return Err(MemoUnreadable::Truncated);
    }
    if &bytes[..8] != MARK {
        return Err(MemoUnreadable::Mark);
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().expect("four bytes"));
    if version != VERSION {
        return Err(MemoUnreadable::Version(version));
    }
    if bytes[12..44] != contract_digest(contract) {
        return Err(MemoUnreadable::OtherContract);
    }
    let dimensions = u32::from_le_bytes(bytes[44..48].try_into().expect("four bytes"));
    if dimensions as usize != contract.dimensions {
        return Err(MemoUnreadable::Dimensions(dimensions));
    }
    let count = u64::from_le_bytes(bytes[48..56].try_into().expect("eight bytes"));
    let entry_bytes = 32 + 4 * contract.dimensions;
    let expected = usize::try_from(count)
        .ok()
        .and_then(|n| n.checked_mul(entry_bytes))
        .and_then(|n| n.checked_add(HEADER_BYTES + CHECKSUM_BYTES));
    if expected != Some(bytes.len()) {
        return Err(MemoUnreadable::Truncated);
    }
    let (body, checksum) = bytes.split_at(bytes.len() - CHECKSUM_BYTES);
    if Sha256::digest(body).as_slice() != checksum {
        return Err(MemoUnreadable::Checksum);
    }
    Ok(body[HEADER_BYTES..]
        .chunks_exact(entry_bytes)
        .map(|entry| {
            let key: Key = entry[..32].try_into().expect("a 32-byte key");
            let vector = entry[32..]
                .chunks_exact(4)
                .map(|f| f32::from_le_bytes(f.try_into().expect("four bytes")))
                .collect();
            (key, vector)
        })
        .collect())
}

/// What a run starts from.
struct Loaded {
    entries: HashMap<Key, Vec<f32>>,
    /// A file of this contract read whole.
    used: bool,
    warning: Option<String>,
}

/// Reads the memo at `path`. A file that is not there is the first run, an
/// empty memo with nothing to say; one that cannot be used is an empty memo
/// and a warning that names it and why.
fn load_memo(path: &Path, contract: &MemoContract) -> Loaded {
    let read = match std::fs::read(path) {
        Ok(bytes) => decode(&bytes, contract),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Loaded { entries: HashMap::new(), used: false, warning: None };
        }
        Err(e) => Err(MemoUnreadable::Read(e)),
    };
    match read {
        Ok(entries) => Loaded { entries, used: true, warning: None },
        Err(cause) => Loaded {
            entries: HashMap::new(),
            used: false,
            warning: Some(format!("the vector memo {} is ignored: {cause}", file_name_of(path))),
        },
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Writes the file whole to a temporary one beside `path` and renames it over
/// `path`, so a reader sees the old file or the new one and a failed write
/// leaves the old one as it was.
fn replace_file(path: &Path, write: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut temporary: PathBuf = dir.to_path_buf();
    temporary.push(format!("{}.tmp-{}", file_name_of(path), uuid::Uuid::new_v4()));
    let written = (|| {
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        let mut buffered = io::BufWriter::new(file);
        write(&mut buffered)?;
        buffered.flush()?;
        buffered.get_ref().sync_all()?;
        drop(buffered);
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// A vector the memo may keep: the contract's length and finite everywhere.
/// What fails this reaches the caller as it came, so the refresh rejects it
/// exactly as it would without a memo.
fn usable(vector: &[f32], dimensions: usize) -> bool {
    vector.len() == dimensions && vector.iter().all(|x| x.is_finite())
}

/// The vectors the memo holds for `keys`, in place, and the positions it does
/// not hold.
fn split_hits(keys: &[Key], known: &HashMap<Key, Vec<f32>>) -> (Vec<Option<Vec<f32>>>, Vec<usize>) {
    let slots: Vec<Option<Vec<f32>>> = keys.iter().map(|k| known.get(k).cloned()).collect();
    let missing = slots.iter().enumerate().filter(|(_, s)| s.is_none()).map(|(i, _)| i).collect();
    (slots, missing)
}

/// Puts the vectors the embedder answered for the positions in `missing` back
/// among the ones the memo held. An answer of another count is not put back
/// anywhere: the list has the hits plus the answer, a length the caller did not
/// ask for, which is how it would see the same answer without a memo.
fn merge(slots: Vec<Option<Vec<f32>>>, missing: &[usize], fetched: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    if fetched.len() != missing.len() {
        return slots.into_iter().flatten().chain(fetched).collect();
    }
    let mut slots = slots;
    for (&at, vector) in missing.iter().zip(fetched) {
        slots[at] = Some(vector);
    }
    slots.into_iter().flatten().collect()
}

#[derive(Default)]
struct Tally {
    /// What this run embedded, one entry per key, the first answer.
    fresh: HashMap<Key, Vec<f32>>,
    from_memo: usize,
    embedded: usize,
}

/// The embedder a run indexes through: passages the memo had are answered from
/// it and the rest go to the real embedder in one call, in the order they came.
/// Questions go straight through.
struct MemoEmbedder {
    inner: Arc<dyn Embedder>,
    contract: MemoContract,
    /// The memo as the run found it. What the run adds is not looked up.
    known: HashMap<Key, Vec<f32>>,
    tally: Mutex<Tally>,
}

#[async_trait]
impl Embedder for MemoEmbedder {
    async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        self.inner.embed_with_role(text, role).await
    }

    async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
        if role == EmbedRole::Query {
            return self.inner.embed_batch(texts, role).await;
        }
        let keys: Vec<Key> = texts
            .iter()
            .map(|text| {
                let prefixed =
                    apply_role_prefix(role, text, &self.contract.query_prefix, &self.contract.passage_prefix);
                memo_key(&self.contract, role, &prefixed)
            })
            .collect();
        let (slots, missing) = split_hits(&keys, &self.known);
        let fetched = if missing.is_empty() {
            Vec::new()
        } else {
            let asked: Vec<String> = missing.iter().map(|&at| texts[at].clone()).collect();
            self.inner.embed_batch(&asked, role).await?
        };
        let mut tally = self.tally.lock().expect("the tally is never held across a panic");
        tally.from_memo += keys.len() - missing.len();
        tally.embedded += missing.len();
        if fetched.len() == missing.len() {
            for (&at, vector) in missing.iter().zip(&fetched) {
                if usable(vector, self.contract.dimensions) {
                    tally.fresh.entry(keys[at]).or_insert_with(|| vector.clone());
                }
            }
        }
        drop(tally);
        Ok(merge(slots, &missing, fetched))
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn query_prefix(&self) -> &str {
        self.inner.query_prefix()
    }

    fn passage_prefix(&self) -> &str {
        self.inner.passage_prefix()
    }
}

/// What a run took from the memo and added to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvalMemoReport {
    /// The memo's file name, empty when the run had no file.
    pub file: String,
    /// A file of this contract was read whole.
    pub used: bool,
    /// Passages answered from the memo.
    pub from_memo: usize,
    /// Passages sent to the real embedder, a repeated text each time.
    pub embedded: usize,
    /// Reading the memo and writing it back, with their hashes.
    pub seconds: f64,
    /// Everything the run had to say about the file, reading first.
    pub warnings: Vec<String>,
}

/// The memo of one run: it is read when the run is set up, it embeds the run's
/// passages through [`embedder`](Self::embedder) and what the run added is
/// written back by [`finish`](Self::finish).
pub struct EvalMemo {
    decorator: Arc<MemoEmbedder>,
    metrics: Arc<Recorder>,
    /// None when the run neither reads nor writes a file.
    file: PathBuf,
    used: bool,
    warnings: Vec<String>,
    read_seconds: f64,
}

impl EvalMemo {
    /// Reads the memo of `embedder`'s contract in `dir`. It never fails: a file
    /// that cannot be used is a warning and an empty memo that the run's own
    /// vectors replace.
    pub fn open(embedder: &EvalEmbedder, dir: &Path) -> Self {
        let contract = MemoContract {
            model_id: embedder.model_id().to_string(),
            dimensions: embedder.dimensions(),
            query_prefix: embedder.query_prefix().to_string(),
            passage_prefix: embedder.passage_prefix().to_string(),
            model_sha256: MODEL_SHA256.to_string(),
            cpu: EvalCpu::this(),
            threads: super::session_threads(),
        };
        Self::with_contract(embedder, dir, contract)
    }

    fn with_contract(embedder: &EvalEmbedder, dir: &Path, contract: MemoContract) -> Self {
        let file = dir.join(memo_file_name(&contract));
        let started = Instant::now();
        let loaded = load_memo(&file, &contract);
        let read_seconds = started.elapsed().as_secs_f64();
        let decorator = Arc::new(MemoEmbedder {
            inner: embedder.inner.clone(),
            contract,
            known: loaded.entries,
            tally: Mutex::default(),
        });
        EvalMemo {
            decorator,
            metrics: embedder.metrics.clone(),
            file,
            used: loaded.used,
            warnings: loaded.warning.into_iter().collect(),
            read_seconds,
        }
    }

    /// Why the memo was not read, to say before the run starts.
    pub fn warning(&self) -> Option<&str> {
        self.warnings.first().map(String::as_str)
    }

    /// The embedder to index through: the real one behind the memo, with the
    /// real one's recorder.
    pub fn embedder(&self) -> EvalEmbedder {
        EvalEmbedder::of(self.decorator.clone(), self.metrics.clone())
    }

    /// Writes the memo back with what the run added, if it added anything, and
    /// reports the run. A failed write leaves the earlier file and is a warning.
    pub fn finish(&self) -> EvalMemoReport {
        let started = Instant::now();
        let mut warnings = self.warnings.clone();
        let (from_memo, embedded, fresh) = {
            let mut tally = self.decorator.tally.lock().expect("the tally is never held across a panic");
            (tally.from_memo, tally.embedded, std::mem::take(&mut tally.fresh))
        };
        if !fresh.is_empty() {
            let path = &self.file;
            let mut all: BTreeMap<Key, Vec<f32>> =
                self.decorator.known.iter().map(|(k, v)| (*k, v.clone())).collect();
            all.extend(fresh);
            if let Err(e) = replace_file(path, |w| encode(w, &self.decorator.contract, &all)) {
                warnings.push(format!("the vector memo {} was not written: {e}", file_name_of(path)));
            }
        }
        EvalMemoReport {
            file: file_name_of(&self.file),
            used: self.used,
            from_memo,
            embedded,
            seconds: self.read_seconds + started.elapsed().as_secs_f64(),
            warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> MemoContract {
        MemoContract {
            model_id: "m".to_string(),
            dimensions: 3,
            query_prefix: "q: ".to_string(),
            passage_prefix: "p: ".to_string(),
            model_sha256: "abc".to_string(),
            cpu: EvalCpu { os: "os".to_string(), arch: "arch".to_string(), avx2: true, avx512f: false, fma: true },
            threads: 4,
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    use std::collections::{BTreeMap, HashMap};
    use std::io::Write;

    fn entries() -> BTreeMap<Key, Vec<f32>> {
        (0u8..3)
            .map(|i| (memo_key(&contract(), EmbedRole::Passage, &format!("text {i}")), vec![f32::from(i), 0.5, -1.25]))
            .collect()
    }

    fn encoded() -> Vec<u8> {
        let mut bytes = Vec::new();
        encode(&mut bytes, &contract(), &entries()).unwrap();
        bytes
    }

    fn memo_path(dir: &crate::test_dir::TestDir) -> std::path::PathBuf {
        dir.path().join(memo_file_name(&contract()))
    }

    #[test]
    fn a_memo_reads_back_what_was_written_and_writes_the_same_bytes_again() {
        let bytes = encoded();
        let read = decode(&bytes, &contract()).unwrap();
        assert_eq!(read, entries().into_iter().collect::<HashMap<_, _>>());
        let mut again = Vec::new();
        encode(&mut again, &contract(), &read.into_iter().collect()).unwrap();
        assert_eq!(again, bytes);
        assert!(bytes.starts_with(b"clawgvec"));
        assert_eq!(bytes.len(), 56 + 3 * (32 + 4 * 3) + 32);
    }

    #[test]
    fn a_truncated_memo_is_ignored_with_a_warning() {
        let dir = crate::test_dir::test_dir();
        let bytes = encoded();
        let cut = &bytes[..bytes.len() - 32 - 6];
        assert!(matches!(decode(cut, &contract()), Err(MemoUnreadable::Truncated)));
        std::fs::write(memo_path(&dir), cut).unwrap();

        let loaded = load_memo(&memo_path(&dir), &contract());

        assert!(loaded.entries.is_empty(), "no vector of a file that does not read whole");
        assert!(!loaded.used);
        let warning = loaded.warning.expect("a warning");
        assert!(warning.contains(&memo_file_name(&contract())), "{warning}");
    }

    #[test]
    fn a_memo_with_an_unknown_header_is_ignored() {
        let good = encoded();
        let mut other_contract = contract();
        other_contract.threads += 1;
        let mut cases: Vec<(&str, Vec<u8>, MemoContract)> = Vec::new();
        let mut mark = good.clone();
        mark[0] ^= 0xff;
        cases.push(("mark", mark, contract()));
        let mut version = good.clone();
        version[8..12].copy_from_slice(&2u32.to_le_bytes());
        cases.push(("version", version, contract()));
        cases.push(("contract", good.clone(), other_contract));
        let mut dimensions = good.clone();
        dimensions[52..56].copy_from_slice(&4u32.to_le_bytes());
        cases.push(("dimensions", dimensions, contract()));
        let mut middle = good.clone();
        middle[56 + 32 + 2] ^= 0x01;
        cases.push(("a byte in the middle", middle, contract()));

        for (name, bytes, under) in cases {
            let dir = crate::test_dir::test_dir();
            let path = dir.path().join(memo_file_name(&under));
            std::fs::write(&path, &bytes).unwrap();
            assert!(decode(&bytes, &under).is_err(), "{name}");
            let loaded = load_memo(&path, &under);
            assert!(loaded.entries.is_empty() && !loaded.used, "{name}");
            assert!(loaded.warning.is_some(), "{name}");
        }
        assert!(matches!(decode(&{ let mut b = good.clone(); b[0] ^= 1; b }, &contract()), Err(MemoUnreadable::Mark)));
        assert!(matches!(
            decode(&{ let mut b = good.clone(); b[8] = 2; b }, &contract()),
            Err(MemoUnreadable::Version(2))
        ));
        assert!(matches!(
            decode(&{ let mut b = good.clone(); b[60] ^= 1; b }, &contract()),
            Err(MemoUnreadable::Checksum)
        ));
    }

    #[test]
    fn an_unreadable_memo_is_ignored() {
        let dir = crate::test_dir::test_dir();
        std::fs::create_dir(memo_path(&dir)).unwrap();

        let loaded = load_memo(&memo_path(&dir), &contract());

        assert!(loaded.entries.is_empty() && !loaded.used);
        assert!(loaded.warning.is_some_and(|w| w.contains(&memo_file_name(&contract()))));
    }

    #[test]
    fn a_missing_memo_is_an_empty_one_without_a_warning() {
        let dir = crate::test_dir::test_dir();

        let loaded = load_memo(&memo_path(&dir), &contract());

        assert!(loaded.entries.is_empty() && !loaded.used && loaded.warning.is_none());
    }

    #[test]
    fn a_memo_that_reads_whole_is_used() {
        let dir = crate::test_dir::test_dir();
        replace_file(&memo_path(&dir), |w| encode(w, &contract(), &entries())).unwrap();

        let loaded = load_memo(&memo_path(&dir), &contract());

        assert_eq!((loaded.entries.len(), loaded.used, loaded.warning), (3, true, None));
    }

    /// A writer that fails once `left` more bytes were written.
    struct FailsAfter<'a> {
        inner: &'a mut dyn Write,
        left: usize,
    }

    impl Write for FailsAfter<'_> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Err(std::io::Error::other("disk full"));
            }
            let n = buf.len().min(self.left);
            self.left -= n;
            self.inner.write(&buf[..n])
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    #[test]
    fn a_failed_write_leaves_the_previous_memo() {
        let dir = crate::test_dir::test_dir();
        let path = memo_path(&dir);
        replace_file(&path, |w| encode(w, &contract(), &entries())).unwrap();
        let before = std::fs::read(&path).unwrap();

        let mut more = entries();
        more.insert(memo_key(&contract(), EmbedRole::Passage, "another"), vec![9.0, 9.0, 9.0]);
        let failed = replace_file(&path, |w| encode(&mut FailsAfter { inner: w, left: 100 }, &contract(), &more));

        assert!(failed.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, [std::ffi::OsString::from(memo_file_name(&contract()))], "no partial file stays");
    }

    #[test]
    fn writing_creates_the_folder_and_leaves_no_temporary_file() {
        let dir = crate::test_dir::test_dir();
        let path = dir.path().join("cache").join(memo_file_name(&contract()));

        replace_file(&path, |w| encode(w, &contract(), &entries())).unwrap();

        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1);
        assert_eq!(decode(&std::fs::read(&path).unwrap(), &contract()).unwrap().len(), 3);
    }

    #[test]
    fn the_memo_key_changes_with_each_component_of_the_contract() {
        let base = memo_key(&contract(), EmbedRole::Passage, "hello");
        type Change = fn(&mut MemoContract);
        let contract_rows: [(&str, Change); 11] = [
            ("model", |c| c.model_id.push('2')),
            ("dimensions", |c| c.dimensions += 1),
            ("query prefix", |c| c.query_prefix.push('x')),
            ("passage prefix", |c| c.passage_prefix.push('x')),
            ("model sha", |c| c.model_sha256.push('d')),
            ("os", |c| c.cpu.os.push('x')),
            ("arch", |c| c.cpu.arch.push('x')),
            ("avx2", |c| c.cpu.avx2 = !c.cpu.avx2),
            ("avx512f", |c| c.cpu.avx512f = !c.cpu.avx512f),
            ("fma", |c| c.cpu.fma = !c.cpu.fma),
            ("threads", |c| c.threads += 1),
        ];
        for (name, change) in contract_rows {
            let mut changed = contract();
            change(&mut changed);
            assert_ne!(memo_key(&changed, EmbedRole::Passage, "hello"), base, "{name}");
        }
        assert_ne!(memo_key(&contract(), EmbedRole::Query, "hello"), base, "role");
        assert_ne!(memo_key(&contract(), EmbedRole::Passage, "hellO"), base, "one character");
        assert_ne!(memo_key(&contract(), EmbedRole::Passage, "hello "), base, "a space");
    }

    #[test]
    fn the_memo_key_separates_neighbouring_components() {
        let (mut a, mut b) = (contract(), contract());
        (a.model_id, a.query_prefix) = ("ab".to_string(), "c".to_string());
        (b.model_id, b.query_prefix) = ("a".to_string(), "bc".to_string());
        assert_ne!(memo_key(&a, EmbedRole::Passage, "t"), memo_key(&b, EmbedRole::Passage, "t"));
        assert_ne!(contract_digest(&a), contract_digest(&b));

        let (mut c, mut d) = (contract(), contract());
        (c.passage_prefix, c.model_sha256) = ("ab".to_string(), "c".to_string());
        (d.passage_prefix, d.model_sha256) = ("a".to_string(), "bc".to_string());
        assert_ne!(memo_key(&c, EmbedRole::Passage, "t"), memo_key(&d, EmbedRole::Passage, "t"));
    }

    #[test]
    fn the_memo_key_is_stable() {
        assert_eq!(
            hex(&memo_key(&contract(), EmbedRole::Passage, "hello")),
            "79d694c180b36a7f486d7c7ee1e8218d4e6f900ecaf7999529bc42aacf1a9cb1"
        );
    }

    use super::super::{EvalDocIndex, EvalDocReport};

    /// How a [`Recording`] spoils the answer to a batch.
    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        /// One vector fewer than texts.
        Short,
        /// A vector of the wrong length for every text holding `BROKEN`.
        WrongLength,
        /// A NaN in the vector of every text holding `BROKEN`.
        NotFinite,
        /// One vector fewer, only for a batch that holds `BROKEN`.
        ShortWhenBroken,
    }

    /// An embedder that writes down each call it gets, with the model, the
    /// dimension and the prefix a test sets.
    struct Recording {
        model: &'static str,
        dimensions: usize,
        passage_prefix: &'static str,
        calls: Mutex<Vec<(EmbedRole, Vec<String>)>>,
        fault: Option<Fault>,
    }

    impl Recording {
        fn with(model: &'static str, dimensions: usize, passage_prefix: &'static str, fault: Option<Fault>) -> Arc<Self> {
            Arc::new(Recording { model, dimensions, passage_prefix, calls: Mutex::default(), fault })
        }

        fn new() -> Arc<Self> {
            Recording::with("m1", 384, "query: ", None)
        }

        fn faulty(fault: Fault) -> Arc<Self> {
            Recording::with("m1", 384, "query: ", Some(fault))
        }

        fn calls(&self) -> Vec<(EmbedRole, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }

        fn passages(&self) -> Vec<String> {
            self.calls().into_iter().filter(|(r, _)| *r == EmbedRole::Passage).flat_map(|(_, t)| t).collect()
        }

        fn vector(&self, text: &str) -> Vec<f32> {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            text.hash(&mut h);
            let seed = h.finish();
            let mut v: Vec<f32> = (0..self.dimensions).map(|i| ((seed >> (i % 40)) & 0xff) as f32 / 255.0 + 0.1).collect();
            if text.contains("BROKEN") {
                match self.fault {
                    Some(Fault::WrongLength) => {
                        v.pop();
                    }
                    Some(Fault::NotFinite) => v[0] = f32::NAN,
                    _ => {}
                }
            }
            v
        }
    }

    #[async_trait]
    impl Embedder for Recording {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            self.calls.lock().unwrap().push((role, vec![text.to_string()]));
            Ok(self.vector(text))
        }

        async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
            self.calls.lock().unwrap().push((role, texts.to_vec()));
            let mut out: Vec<Vec<f32>> = texts.iter().map(|t| self.vector(t)).collect();
            let broken = texts.iter().any(|t| t.contains("BROKEN"));
            if self.fault == Some(Fault::Short) || (self.fault == Some(Fault::ShortWhenBroken) && broken) {
                out.pop();
            }
            Ok(out)
        }

        fn dimensions(&self) -> usize {
            self.dimensions
        }

        fn model_id(&self) -> &str {
            self.model
        }

        fn query_prefix(&self) -> &str {
            "query: "
        }

        fn passage_prefix(&self) -> &str {
            self.passage_prefix
        }
    }

    fn handle(recording: &Arc<Recording>) -> EvalEmbedder {
        EvalEmbedder::of(recording.clone(), Arc::default())
    }

    fn contract_for(recording: &Recording, model_sha256: &str) -> MemoContract {
        MemoContract {
            model_id: recording.model.to_string(),
            dimensions: recording.dimensions,
            query_prefix: "query: ".to_string(),
            passage_prefix: recording.passage_prefix.to_string(),
            model_sha256: model_sha256.to_string(),
            cpu: EvalCpu::this(),
            threads: 4,
        }
    }

    fn memo_over(recording: &Arc<Recording>, dir: &Path, model_sha256: &str) -> EvalMemo {
        EvalMemo::with_contract(&handle(recording), dir, contract_for(recording, model_sha256))
    }

    fn texts(of: &[&str]) -> Vec<String> {
        of.iter().map(|t| t.to_string()).collect()
    }

    fn write_doc(root: &Path, name: &str, sections: usize, edit: &[(usize, &str)]) {
        let mut text = format!("# {name}\n");
        for i in 0..sections {
            let body = edit.iter().find(|(at, _)| *at == i).map_or("original", |(_, b)| *b);
            text.push_str(&format!("## Section {i}\nthe {body} body of {name} number {i}\n"));
        }
        std::fs::write(root.join(name), text).unwrap();
    }

    /// Indexes `root` into a store of its own and returns what it wrote and the
    /// bytes of every passage's vector, in path and ordinal order.
    async fn index(embedder: &EvalEmbedder, root: &Path) -> (EvalDocReport, Vec<(String, i64, Vec<u8>)>) {
        let store_dir = crate::test_dir::test_dir();
        let index = EvalDocIndex::open(store_dir.path(), embedder, store_dir.path()).await.unwrap();
        let report = index.index(root, &[]).await.unwrap().report;
        let vectors = index
            .store
            .conn
            .call(|c| -> rusqlite::Result<Vec<(String, i64, Vec<u8>)>> {
                c.prepare(
                    "SELECT c.path, c.ordinal, v.embedding FROM project_doc_chunk c \
                     JOIN project_doc_vectors v ON v.rowid = c.id ORDER BY c.path, c.ordinal",
                )?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
            })
            .await
            .unwrap();
        index.close().await.unwrap();
        (report, vectors)
    }

    fn folder_bytes(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap_or_default())
            })
            .collect();
        files.sort();
        files
    }

    #[tokio::test]
    async fn a_full_memo_embeds_nothing_and_stores_the_same_vector_bytes() {
        let root = crate::test_dir::test_dir();
        let cache = crate::test_dir::test_dir();
        write_doc(root.path(), "big.md", 20, &[]);
        write_doc(root.path(), "small.md", 3, &[]);

        let first = Recording::new();
        let memo = memo_over(&first, cache.path(), "sha");
        let (report, first_vectors) = index(&memo.embedder(), root.path()).await;
        let made = memo.finish();
        assert!(!made.used && made.from_memo == 0 && made.embedded == report.passages, "{made:?}");
        let file = folder_bytes(cache.path());
        assert_eq!(file.len(), 1, "the memo is one file");

        let second = Recording::new();
        let memo = memo_over(&second, cache.path(), "sha");
        assert!(memo.warning().is_none());
        let (again, second_vectors) = index(&memo.embedder(), root.path()).await;
        let reused = memo.finish();

        assert!(second.calls().is_empty(), "nothing reaches the embedder");
        assert!(reused.used && reused.embedded == 0 && reused.from_memo == again.passages, "{reused:?}");
        assert_eq!(second_vectors, first_vectors);
        assert_eq!(folder_bytes(cache.path()), file, "a run that added nothing writes nothing");
    }

    #[tokio::test]
    async fn an_empty_memo_embeds_in_the_batches_production_does() {
        let root = crate::test_dir::test_dir();
        write_doc(root.path(), "big.md", 20, &[]);
        write_doc(root.path(), "small.md", 3, &[]);

        let plain = Recording::new();
        let (report, _) = index(&handle(&plain), root.path()).await;
        let cache = crate::test_dir::test_dir();
        let decorated = Recording::new();
        let memo = memo_over(&decorated, cache.path(), "sha");
        index(&memo.embedder(), root.path()).await;

        assert!(report.passages >= 23, "{report:?}");
        assert_eq!(decorated.calls(), plain.calls(), "same calls, same order, same texts");
        assert!(plain.calls().iter().any(|(_, t)| t.len() >= 20), "a doc goes in one call");
    }

    #[tokio::test]
    async fn a_changed_doc_embeds_only_its_new_texts() {
        let root = crate::test_dir::test_dir();
        let cache = crate::test_dir::test_dir();
        write_doc(root.path(), "big.md", 20, &[]);
        write_doc(root.path(), "small.md", 3, &[]);
        let first = Recording::new();
        let memo = memo_over(&first, cache.path(), "sha");
        index(&memo.embedder(), root.path()).await;
        memo.finish();

        write_doc(root.path(), "big.md", 21, &[(4, "edited"), (20, "added")]);
        let second = Recording::new();
        let memo = memo_over(&second, cache.path(), "sha");
        let (report, _) = index(&memo.embedder(), root.path()).await;
        let run = memo.finish();

        let known: std::collections::HashSet<String> = first.passages().into_iter().collect();
        let sent = second.passages();
        assert_eq!(second.calls().len(), 1, "the changed doc goes in one call and the others in none");
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|t| !known.contains(t)));
        assert_eq!(run.embedded, 2);
        assert_eq!(run.from_memo + run.embedded, report.passages);
    }

    #[tokio::test]
    async fn what_a_run_adds_is_not_used_until_the_next_run() {
        let cache = crate::test_dir::test_dir();
        let recording = Recording::new();
        let memo = memo_over(&recording, cache.path(), "sha");
        let embedder = memo.embedder();

        embedder.embed_batch(&texts(&["same"]), EmbedRole::Passage).await.unwrap();
        embedder.embed_batch(&texts(&["same"]), EmbedRole::Passage).await.unwrap();
        let run = memo.finish();

        assert_eq!(recording.passages(), ["same", "same"]);
        assert_eq!((run.from_memo, run.embedded), (0, 2));
        let file = std::fs::read(cache.path().join(&run.file)).unwrap();
        assert_eq!(decode(&file, &contract_for(&recording, "sha")).unwrap().len(), 1, "one entry for the text");
    }

    #[tokio::test]
    async fn a_run_that_adds_nothing_does_not_touch_the_file() {
        let cache = crate::test_dir::test_dir();
        let first = Recording::new();
        let memo = memo_over(&first, cache.path(), "sha");
        memo.embedder().embed_batch(&texts(&["same"]), EmbedRole::Passage).await.unwrap();
        let file = cache.path().join(memo.finish().file);
        let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(86_400);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(long_ago).unwrap();

        let second = Recording::new();
        let memo = memo_over(&second, cache.path(), "sha");
        memo.embedder().embed_batch(&texts(&["same"]), EmbedRole::Passage).await.unwrap();
        let run = memo.finish();

        assert_eq!((run.from_memo, run.embedded), (1, 0));
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), long_ago, "the file was written again");
    }

    #[tokio::test]
    async fn what_a_run_embedded_before_the_index_failed_is_kept() {
        let root = crate::test_dir::test_dir();
        let cache = crate::test_dir::test_dir();
        write_doc(root.path(), "a.md", 3, &[]);
        write_doc(root.path(), "b.md", 3, &[(1, "BROKEN")]);
        let recording = Recording::faulty(Fault::ShortWhenBroken);
        let memo = memo_over(&recording, cache.path(), "sha");

        let store_dir = crate::test_dir::test_dir();
        let embedder = memo.embedder();
        let index = EvalDocIndex::open(store_dir.path(), &embedder, store_dir.path()).await.unwrap();
        let failed = index.index(root.path(), &[]).await;
        index.close().await.unwrap();
        let run = memo.finish();

        assert!(failed.is_err(), "the second doc comes back short");
        assert_eq!(recording.calls().len(), 2, "the first doc was embedded before the failure");
        let bytes = std::fs::read(cache.path().join(&run.file)).unwrap();
        assert_eq!(decode(&bytes, &contract_for(&recording, "sha")).unwrap().len(), 3, "the sound doc's passages");
    }

    #[tokio::test]
    async fn questions_never_go_through_the_memo() {
        let cache = crate::test_dir::test_dir();
        let recording = Recording::new();
        let memo = memo_over(&recording, cache.path(), "sha");
        memo.embedder().embed_batch(&texts(&["same"]), EmbedRole::Passage).await.unwrap();
        memo.finish();
        let file = folder_bytes(cache.path());

        let second = Recording::new();
        let memo = memo_over(&second, cache.path(), "sha");
        let embedder = memo.embedder();
        embedder.embed_batch(&texts(&["same", "other"]), EmbedRole::Query).await.unwrap();
        embedder.embed_with_role("same", EmbedRole::Query).await.unwrap();
        let run = memo.finish();

        assert_eq!(second.calls(), [(EmbedRole::Query, texts(&["same", "other"])), (EmbedRole::Query, texts(&["same"]))]);
        assert_eq!((run.from_memo, run.embedded), (0, 0));
        assert_eq!(folder_bytes(cache.path()), file, "a question stores nothing");
    }

    #[tokio::test]
    async fn a_vector_the_embedder_returns_unusable_is_not_stored() {
        for fault in [Fault::Short, Fault::WrongLength, Fault::NotFinite] {
            let cache = crate::test_dir::test_dir();
            let recording = Recording::faulty(fault);
            let memo = memo_over(&recording, cache.path(), "sha");
            let asked = texts(&["sound", "BROKEN"]);

            let got = memo.embedder().embed_batch(&asked, EmbedRole::Passage).await.unwrap();
            let run = memo.finish();

            match fault {
                Fault::Short => assert_ne!(got.len(), asked.len(), "the count is wrong, as without the memo"),
                Fault::WrongLength => assert_ne!(got[1].len(), 384),
                Fault::NotFinite => assert!(got[1][0].is_nan()),
                Fault::ShortWhenBroken => unreachable!("not one of the faults this test goes through"),
            }
            let file = std::fs::read(cache.path().join(&run.file));
            let stored = file.map_or(0, |bytes| decode(&bytes, &contract_for(&recording, "sha")).unwrap().len());
            assert_eq!(stored, usize::from(fault != Fault::Short), "only the sound vector is stored");
        }
    }

    #[tokio::test]
    async fn the_memo_file_holds_no_text() {
        let cache = crate::test_dir::test_dir();
        let recording = Recording::new();
        let memo = memo_over(&recording, cache.path(), "sha");
        let text = texts(&["UNMISTAKABLE passage text of docs/secret/path.md"]);
        memo.embedder().embed_batch(&text, EmbedRole::Passage).await.unwrap();
        let run = memo.finish();

        let bytes = std::fs::read(cache.path().join(&run.file)).unwrap();
        let holds = |needle: &str| bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
        assert!(!holds("UNMISTAKABLE") && !holds("secret") && !holds("path.md"));
    }

    #[tokio::test]
    async fn a_memo_made_under_another_contract_is_not_used() {
        let cache = crate::test_dir::test_dir();
        let asked = texts(&["one", "two", "three"]);
        let first = Recording::new();
        let memo = memo_over(&first, cache.path(), "sha1");
        memo.embedder().embed_batch(&asked, EmbedRole::Passage).await.unwrap();
        memo.finish();

        let same = Recording::new();
        let memo = memo_over(&same, cache.path(), "sha1");
        memo.embedder().embed_batch(&asked, EmbedRole::Passage).await.unwrap();
        assert_eq!(memo.finish().from_memo, 3, "the same contract does use it");

        let others: [(&str, Arc<Recording>, &str); 4] = [
            ("passage prefix", Recording::with("m1", 384, "passage: ", None), "sha1"),
            ("model", Recording::with("m2", 384, "query: ", None), "sha1"),
            ("dimensions", Recording::with("m1", 768, "query: ", None), "sha1"),
            ("model sha", Recording::new(), "sha2"),
        ];
        for (name, recording, sha) in others {
            let memo = memo_over(&recording, cache.path(), sha);
            memo.embedder().embed_batch(&asked, EmbedRole::Passage).await.unwrap();
            let run = memo.finish();
            assert_eq!((run.used, run.from_memo, run.embedded), (false, 0, 3), "{name}");
            assert_eq!(recording.passages(), asked, "{name}");
        }

        assert_eq!(folder_bytes(cache.path()).len(), 5, "one file per contract");
        let back = Recording::new();
        let memo = memo_over(&back, cache.path(), "sha1");
        memo.embedder().embed_batch(&asked, EmbedRole::Passage).await.unwrap();
        let run = memo.finish();
        assert_eq!((run.used, run.from_memo, run.embedded), (true, 3, 0), "the first contract kept its memo");
        assert!(back.calls().is_empty());
    }

    #[tokio::test]
    async fn a_memo_that_cannot_be_read_is_replaced_by_what_the_run_embedded() {
        let cache = crate::test_dir::test_dir();
        let recording = Recording::new();
        let contract = contract_for(&recording, "sha");
        std::fs::write(cache.path().join(memo_file_name(&contract)), b"clawgvec but cut").unwrap();

        let memo = memo_over(&recording, cache.path(), "sha");
        assert!(memo.warning().is_some_and(|w| w.contains(&memo_file_name(&contract))));
        memo.embedder().embed_batch(&texts(&["one"]), EmbedRole::Passage).await.unwrap();
        let run = memo.finish();

        assert!(!run.used);
        assert_eq!(run.warnings.len(), 1);
        let bytes = std::fs::read(cache.path().join(&run.file)).unwrap();
        assert_eq!(decode(&bytes, &contract).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_write_that_fails_is_a_warning_and_not_an_error() {
        let cache = crate::test_dir::test_dir();
        let recording = Recording::new();
        let contract = contract_for(&recording, "sha");
        let memo = memo_over(&recording, cache.path(), "sha");
        memo.embedder().embed_batch(&texts(&["one"]), EmbedRole::Passage).await.unwrap();
        std::fs::create_dir(cache.path().join(memo_file_name(&contract))).unwrap();

        let run = memo.finish();

        assert_eq!(run.embedded, 1);
        assert_eq!(run.warnings.len(), 1, "{:?}", run.warnings);
        assert!(run.warnings[0].contains(&memo_file_name(&contract)));
    }

    #[tokio::test]
    async fn open_names_the_contract_of_the_embedder_it_wraps() {
        let cache = crate::test_dir::test_dir();
        let memo = EvalMemo::open(&EvalEmbedder::fake(), cache.path());
        memo.embedder().embed_batch(&texts(&["one"]), EmbedRole::Passage).await.unwrap();
        let run = memo.finish();
        assert!(run.file.starts_with("docs-vectors-") && run.file.ends_with(".bin"), "{}", run.file);
        assert!(cache.path().join(&run.file).is_file());

        let again = EvalMemo::open(&EvalEmbedder::fake(), cache.path());
        again.embedder().embed_batch(&texts(&["one"]), EmbedRole::Passage).await.unwrap();
        let run = again.finish();
        assert!(run.used && run.from_memo == 1 && run.embedded == 0, "{run:?}");
    }
}
