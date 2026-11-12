//! Generate a BPE vocab file from a SentencePiece .model file, and
//! encode CJK hotwords into BPE subword tokens for models whose
//! tokens.txt lacks individual characters.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use crate::path_display::for_log_path;

/// Errors that can occur during BPE vocab generation.
#[derive(Debug)]
pub enum VocabError {
    Io(io::Error),
    InvalidFormat(String),
    Truncated,
}

impl From<io::Error> for VocabError {
    fn from(e: io::Error) -> Self {
        VocabError::Io(e)
    }
}

impl std::fmt::Display for VocabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VocabError::Io(e) => write!(f, "I/O error: {}", e),
            VocabError::InvalidFormat(s) => write!(f, "invalid model format: {}", s),
            VocabError::Truncated => write!(f, "truncated model data"),
        }
    }
}

/// Generate the vocab file from `bpe_model_path`, writing to `vocab_path`.
///
/// Returns the vocab path on success.
pub fn generate_vocab(bpe_model_path: &Path, vocab_path: &Path) -> Result<(), VocabError> {
    let data = fs::read(bpe_model_path)?;
    let pieces = parse_model_proto(&data)?;

    let mut file = fs::File::create(vocab_path)?;
    // Use the same format as the Python export script: token\tlog_score\n
    for (token, score) in &pieces {
        writeln!(file, "{}\t{}", token, score)?;
    }
    file.flush()?;
    Ok(())
}

/// Check if a vocab file exists for the given bpe model, generating it if needed.
///
/// If `bpe_model_path` does not exist, returns `None`.
/// If a corresponding `.vocab` file already exists, returns its path.
/// Otherwise generates the vocab file and returns its path.
///
/// Returns `None` if generation fails (in which case a warning is logged).
pub fn ensure_vocab(bpe_model_path: &Path) -> Option<std::path::PathBuf> {
    if !bpe_model_path.exists() {
        return None;
    }
    let vocab_path = bpe_model_path.with_extension("vocab");
    if vocab_path.exists() {
        return Some(vocab_path);
    }
    tracing::info!(
        "Generating BPE vocab from {} -> {}",
        for_log_path(bpe_model_path),
        for_log_path(&vocab_path)
    );
    match generate_vocab(bpe_model_path, &vocab_path) {
        Ok(()) => {
            tracing::info!("BPE vocab generated: {}", for_log_path(&vocab_path));
            Some(vocab_path)
        }
        Err(e) => {
            tracing::warn!(
                "Failed to generate BPE vocab from {}: {}",
                for_log_path(bpe_model_path),
                e
            );
            None
        }
    }
}

// ---- ProtoBuf wire-format parser ----

/// A protobuf wire type.
#[derive(Debug, PartialEq)]
enum WireType {
    Varint = 0,
    Bit64 = 1,
    LengthDelimited = 2,
    Bit32 = 5,
}

#[derive(Debug)]
struct FieldTag {
    field_number: u32,
    wire_type: WireType,
}

/// Read a varint from `data` starting at `pos`, return (value, new_pos).
fn read_varint(data: &[u8], pos: &mut usize) -> Result<u64, VocabError> {
    let mut value: u64 = 0;
    let mut shift = 0;
    while *pos < data.len() {
        let byte = data[*pos];
        *pos += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= 64 {
            return Err(VocabError::InvalidFormat("varint too long".into()));
        }
    }
    Err(VocabError::Truncated)
}

fn decode_tag(data: &[u8], pos: &mut usize) -> Result<FieldTag, VocabError> {
    let raw = read_varint(data, pos)?;
    let wire_type = match raw & 0x07 {
        0 => WireType::Varint,
        1 => WireType::Bit64,
        2 => WireType::LengthDelimited,
        5 => WireType::Bit32,
        other => return Err(VocabError::InvalidFormat(format!("unknown wire type: {}", other))),
    };
    Ok(FieldTag {
        field_number: (raw >> 3) as u32,
        wire_type,
    })
}

/// Skip over a field value of the given wire type.
fn skip_field(data: &[u8], pos: &mut usize, wire_type: &WireType) -> Result<(), VocabError> {
    if *pos > data.len() {
        return Err(VocabError::Truncated);
    }
    match wire_type {
        WireType::Varint => {
            let _ = read_varint(data, pos)?;
        }
        WireType::Bit64 => {
            if *pos + 8 > data.len() {
                return Err(VocabError::Truncated);
            }
            *pos += 8;
        }
        WireType::LengthDelimited => {
            let len = read_varint(data, pos)? as usize;
            if *pos + len > data.len() {
                return Err(VocabError::Truncated);
            }
            *pos += len;
        }
        WireType::Bit32 => {
            if *pos + 4 > data.len() {
                return Err(VocabError::Truncated);
            }
            *pos += 4;
        }
    }
    Ok(())
}

/// Parse a single sub-message containing a SentencePiece entry.
/// Inside: field 1 = piece (string), field 2 = score (float), field 3+ = other (skip)
fn parse_piece(data: &[u8], pos: &mut usize, end: usize) -> Result<(String, f32), VocabError> {
    let mut piece: Option<String> = None;
    let mut score: Option<f32> = None;

    while *pos < end {
        let tag = decode_tag(data, pos)?;
        match tag.field_number {
            1 => {
                // piece (string)
                let len = read_varint(data, pos)? as usize;
                if *pos + len > end {
                    return Err(VocabError::Truncated);
                }
                let bytes = &data[*pos..*pos + len];
                *pos += len;
                piece = Some(String::from_utf8_lossy(bytes).into_owned());
            }
            2 => {
                // score (float)
                if *pos + 4 > end {
                    return Err(VocabError::Truncated);
                }
                let mut buf = [0u8; 4];
                buf.copy_from_slice(&data[*pos..*pos + 4]);
                *pos += 4;
                score = Some(f32::from_le_bytes(buf));
            }
            _ => {
                // Skip other fields (type, etc.)
                let saved_pos = *pos;
                if let Err(_) = skip_field(data, pos, &tag.wire_type) {
                    *pos = saved_pos;
                    return Ok((
                        piece.unwrap_or_default(),
                        score.unwrap_or(0.0),
                    ));
                }
            }
        }
    }

    Ok((piece.unwrap_or_default(), score.unwrap_or(0.0)))
}

/// Parse the SentencePiece ModelProto binary format from raw bytes.
///
/// Extracts all pieces (token + log score) from the repeated field 1.
fn parse_model_proto(data: &[u8]) -> Result<Vec<(String, f32)>, VocabError> {
    if data.is_empty() {
        return Err(VocabError::InvalidFormat("empty model data".into()));
    }

    let mut pieces = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        let tag = decode_tag(data, &mut pos)?;
        match tag.field_number {
            1 => {
                // repeated pieces — each is a length-delimited sub-message
                let len = read_varint(data, &mut pos)? as usize;
                let mut sub_start = pos;
                let sub_end = pos + len;
                if sub_end > data.len() {
                    return Err(VocabError::Truncated);
                }
                pos = sub_end;
                match parse_piece(data, &mut sub_start, sub_end) {
                    Ok((token, score)) => pieces.push((token, score)),
                    Err(e) => {
                        tracing::warn!("Skipping malformed piece entry: {}", e);
                    }
                }
            }
            _ => {
                // Skip trainer_spec, normalizer_spec, etc.
                skip_field(data, &mut pos, &tag.wire_type)?;
            }
        }
    }

    if pieces.is_empty() {
        return Err(VocabError::InvalidFormat("no pieces found in model".into()));
    }

    Ok(pieces)
}

// ---- BPE tokenizer for hotword pre-encoding ----

/// A minimal SentencePiece-compatible BPE tokenizer.
///
/// Loads a .vocab file (token<TAB>score) and uses Viterbi decoding
/// to segment CJK hotwords into subword tokens that exist in the
/// model's tokens.txt, bypassing sherpa-onnx's broken fallback.
pub struct BpeTokenizer {
    tokens: Vec<String>,
    scores: Vec<f32>,
    trie: Trie,
    #[allow(dead_code)]
    unk_id: usize,
}

#[derive(Default)]
struct TrieNode {
    children: HashMap<char, usize>,
    idx: Option<usize>, // token index if this node is a complete token
}

struct Trie {
    nodes: Vec<TrieNode>,
}

impl Trie {
    fn new() -> Self {
        Trie {
            nodes: vec![TrieNode::default()],
        }
    }

    fn insert(&mut self, word: &str, idx: usize) {
        let mut node = 0;
        for ch in word.chars() {
            let next = self.nodes[node].children.get(&ch).copied();
            node = match next {
                Some(n) => n,
                None => {
                    let id = self.nodes.len();
                    self.nodes.push(TrieNode::default());
                    self.nodes[node].children.insert(ch, id);
                    id
                }
            };
        }
        self.nodes[node].idx = Some(idx);
    }

    /// Find all tokens that are prefixes of `text[pos..]`, returning
    /// (token_index, end_byte_position).
    fn prefix_matches(&self, text: &str, pos: usize) -> Vec<(usize, usize)> {
        let mut results = Vec::new();
        let mut node = 0;
        let tail = &text[pos..];
        for (i, ch) in tail.char_indices() {
            if let Some(&next) = self.nodes[node].children.get(&ch) {
                node = next;
                if let Some(idx) = self.nodes[node].idx {
                    results.push((idx, pos + i + ch.len_utf8()));
                }
            } else {
                break;
            }
        }
        results
    }
}

impl BpeTokenizer {
    /// Load from a .vocab file (two columns: token score).
    pub fn load(vocab_path: &Path) -> Result<Self, VocabError> {
        let mut file = fs::File::open(vocab_path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;

        let mut tokens = Vec::new();
        let mut scores = Vec::new();
        let mut trie = Trie::new();
        let mut unk_id = 0usize;

        for (i, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 2 {
                // Try space as separator
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() < 2 {
                    continue;
                }
                tokens.push(parts[0].to_string());
                scores.push(parts[1].parse().unwrap_or(0.0));
            } else {
                tokens.push(parts[0].to_string());
                scores.push(parts[1].parse().unwrap_or(0.0));
            }
            if tokens.last().map(|s| s.as_str()) == Some("<unk>") {
                unk_id = i;
            }
            trie.insert(&tokens[tokens.len() - 1], tokens.len() - 1);
        }

        if tokens.is_empty() {
            return Err(VocabError::InvalidFormat("empty vocab".into()));
        }

        Ok(BpeTokenizer {
            tokens,
            scores,
            trie,
            unk_id,
        })
    }

    /// Encode a text string into BPE subword tokens (space-separated).
    ///
    /// This is equivalent to what SentencePiece does: prefix each
    /// CJK/word segment with `▁`, then run Viterbi DP to find
    /// the best segmentation.
    pub fn encode(&self, text: &str) -> String {
        // SentencePiece normalization: prepend "▁" and join
        // For CJK text, each character is treated as a separate
        // "word" by SentencePiece. We follow the same convention:
        // collect characters and prepend "▁" to the first one.
        let normalized = if text.starts_with('▁') {
            text.to_string()
        } else {
            format!("▁{}", text)
        };

        let chars: Vec<char> = normalized.chars().collect();
        let n = chars.len();

        // Build byte offsets for each character position
        let mut byte_offsets: Vec<usize> = Vec::with_capacity(n + 1);
        byte_offsets.push(0);
        for ch in &chars {
            byte_offsets.push(byte_offsets.last().unwrap() + ch.len_utf8());
        }

        // Build DAG: for each position, list all possible token matches
        // as (end_pos, token_idx, score)
        let mut dag: Vec<Vec<(usize, usize, f32)>> = vec![Vec::new(); n];

        for i in 0..n {
            let matches = self.trie.prefix_matches(&normalized, byte_offsets[i]);
            for (tok_idx, end_byte) in matches {
                // Find the char position corresponding to end_byte
                if let Ok(end_char) = byte_offsets.binary_search(&end_byte) {
                    if end_char <= n {
                        dag[i].push((end_char, tok_idx, self.scores[tok_idx]));
                    }
                }
            }
        }

        // Viterbi DP from right to left
        // route[i] = (best_score, best_end, best_token_idx)
        let neg_inf = f32::NEG_INFINITY;
        let mut route: Vec<(f32, isize, isize)> = vec![(neg_inf, -1, -1); n + 1];
        route[n] = (0.0, 0, 0);

        for i in (0..n).rev() {
            let mut best_score = neg_inf;
            let mut best_end: isize = -1;
            let mut best_idx: isize = -1;
            for &(end, tok_idx, score) in &dag[i] {
                let next_score = route[end].0;
                if next_score == neg_inf {
                    continue;
                }
                let total = score + next_score;
                if total > best_score {
                    best_score = total;
                    best_end = end as isize;
                    best_idx = tok_idx as isize;
                }
            }
            route[i] = (best_score, best_end, best_idx);
        }

        // Cut: extract tokens along best path
        let mut result = Vec::new();
        let mut pos: isize = 0;
        while pos < n as isize {
            let (_score, end, idx) = route[pos as usize];
            if idx < 0 {
                // No match found — emit unk
                result.push("<unk>".to_string());
                pos += 1;
            } else {
                result.push(self.tokens[idx as usize].clone());
                pos = end;
            }
        }

        result.join(" ")
    }
}
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn test_varint_decode() {
        // Single byte: 1 = 0x01
        let mut pos = 0;
        assert_eq!(read_varint(&[0x01], &mut pos).unwrap(), 1);

        // Two bytes: 300 = 0xAC 0x02
        pos = 0;
        assert_eq!(read_varint(&[0xAC, 0x02], &mut pos).unwrap(), 300);
    }

    #[test]
    fn test_decode_tag() {
        // field 1, wire type 2 => (1 << 3) | 2 = 0x0A
        let mut pos = 0;
        let tag = decode_tag(&[0x0A], &mut pos).unwrap();
        assert_eq!(tag.field_number, 1);
        assert_eq!(tag.wire_type, WireType::LengthDelimited);
    }
}
