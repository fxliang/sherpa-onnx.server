use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

pub fn read_onnx_metadata(files: &[PathBuf]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for file in files {
        if let Ok(meta) = read_single(file) {
            for (k, v) in meta {
                out.entry(k.to_lowercase()).or_insert(v);
            }
        }
    }
    out
}

fn read_single(path: &Path) -> anyhow::Result<HashMap<String, String>> {
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext != "onnx" {
        return Ok(HashMap::new());
    }
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut proto = ModelProtoReader::new(&mut reader);
    proto.read_metadata_props()
}

pub fn to_descriptor_params(metadata: &HashMap<String, String>) -> HashMap<String, String> {
    if metadata.is_empty() {
        return HashMap::new();
    }
    let normalized: HashMap<String, String> = metadata
        .iter()
        .map(|(k, v)| (k.to_lowercase().replace('-', "_"), v.clone()))
        .collect();

    let mut params = HashMap::new();

    let model_type_keys = [
        "model_type",
        "model_type_string",
        "sherpa_onnx_model_type",
        "asr_model_type",
        "architecture",
        "arch",
    ];
    if let Some(val) = first_value(&normalized, &model_type_keys) {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() {
            params.insert("model_type_string".to_string(), trimmed);
        }
    }

    let feature_dim_keys = [
        "feature_dim",
        "feat_dim",
        "num_mel_bins",
        "n_mels",
        "n_mel_bins",
        "mel_bins",
    ];
    if let Some(val) = first_int_value(&normalized, &feature_dim_keys) {
        params.insert("feature_dim".to_string(), val.to_string());
    }

    let sample_rate_keys = ["sample_rate", "sampling_rate", "sampling_frequency", "fs"];
    if let Some(val) = first_int_value(&normalized, &sample_rate_keys) {
        params.insert("sample_rate".to_string(), val.to_string());
    }

    let language_keys = ["language", "lang", "src_lang"];
    if let Some(val) = first_value(&normalized, &language_keys) {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() {
            params.insert("language".to_string(), trimmed);
        }
    }

    // Expose encoder_dims so callers can validate before creating a recognizer
    // (older zipformer2 models may lack this metadata and crash C++ encoder init).
    let encoder_dims_keys = [
        "encoder_dims",
        "encoder_dim",
        "encoder_output_dim",
        "enc_dim",
    ];
    if let Some(val) = first_value(&normalized, &encoder_dims_keys) {
        params.insert("encoder_dims".to_string(), val.clone());
    }

    if let Some(has_punct) = infer_has_built_in_punctuation(metadata) {
        params.insert("has_punctuation".to_string(), has_punct.to_string());
    }

    params
}

fn infer_has_built_in_punctuation(metadata: &HashMap<String, String>) -> Option<bool> {
    if metadata.is_empty() {
        return None;
    }
    let normalized: HashMap<String, String> = metadata
        .iter()
        .map(|(k, v)| (k.to_lowercase().replace('-', "_"), v.clone()))
        .collect();

    let punct_keys = [
        "has_punctuation",
        "has_punct",
        "has_punc",
        "built_in_punctuation",
        "builtin_punctuation",
        "with_punctuation",
        "with_punct",
        "with_punc",
        "punctuation",
        "punct",
        "punc",
        "use_pnc",
        "use_punct",
        "use_punctuation",
    ];
    for key in &punct_keys {
        if let Some(val) = normalized.get(*key) {
            if let Some(b) = parse_booleanish(val) {
                return Some(b);
            }
        }
    }
    None
}

fn parse_booleanish(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" | "enable" | "enabled" | "punc" | "punct"
        | "punctuation" => Some(true),
        "0" | "false" | "no" | "n" | "off" | "disable" | "disabled" | "none" => Some(false),
        _ => None,
    }
}

fn first_value<'a>(metadata: &'a HashMap<String, String>, keys: &[&str]) -> Option<&'a String> {
    for key in keys {
        if let Some(val) = metadata.get(*key) {
            return Some(val);
        }
    }
    None
}

fn first_int_value(metadata: &HashMap<String, String>, keys: &[&str]) -> Option<i64> {
    let raw = first_value(metadata, keys)?;
    let re = regex::Regex::new(r"\d+").ok()?;
    re.find(raw)?.as_str().parse().ok()
}

struct ModelProtoReader<'a> {
    reader: &'a mut BufReader<File>,
}

impl<'a> ModelProtoReader<'a> {
    fn new(reader: &'a mut BufReader<File>) -> Self {
        Self { reader }
    }

    fn read_metadata_props(&mut self) -> anyhow::Result<HashMap<String, String>> {
        let mut out = HashMap::new();
        loop {
            let Some(tag) = self.read_varint()? else {
                break;
            };
            let field_number = (tag >> 3) as u32;
            let wire_type = (tag & 0x7) as u32;
            if field_number == 14 && wire_type == 2 {
                let Some(length) = self.read_varint()? else {
                    break;
                };
                let Some(payload) = self.read_bytes(length as usize)? else {
                    break;
                };
                if let Some((key, value)) = Self::read_string_string_entry(&payload) {
                    if !key.is_empty() {
                        out.insert(key, value);
                    }
                }
            } else {
                self.skip_field(wire_type)?;
            }
        }
        Ok(out)
    }

    fn read_string_string_entry(payload: &[u8]) -> Option<(String, String)> {
        let mut p = 0usize;
        let mut key: Option<String> = None;
        let mut value: Option<String> = None;

        let read_varint = |p: &mut usize| -> Option<u64> {
            let mut result = 0u64;
            let mut shift = 0u32;
            while *p < payload.len() && shift < 64 {
                let b = payload[*p] as u64;
                *p += 1;
                result |= (b & 0x7f) << shift;
                if b & 0x80 == 0 {
                    return Some(result);
                }
                shift += 7;
            }
            None
        };

        while p < payload.len() {
            let mut pp = p;
            let Some(tag) = (|| {
                let t = read_varint(&mut pp)?;
                Some(t)
            })() else {
                break;
            };
            p = pp;
            let field_number = (tag >> 3) as u32;
            let wire_type = (tag & 0x7) as u32;

            if wire_type == 2 && (field_number == 1 || field_number == 2) {
                let mut pp2 = p;
                let Some(length) = read_varint(&mut pp2) else {
                    break;
                };
                p = pp2;
                let len = length as usize;
                if p + len > payload.len() {
                    break;
                }
                let text = String::from_utf8_lossy(&payload[p..p + len]).into_owned();
                p += len;
                if field_number == 1 {
                    key = Some(text);
                } else {
                    value = Some(text);
                }
            } else {
                match wire_type {
                    0 => {
                        let _ = read_varint(&mut p);
                    }
                    1 => {
                        p = (p + 8).min(payload.len());
                    }
                    2 => {
                        let mut pp3 = p;
                        if let Some(length) = read_varint(&mut pp3) {
                            p = (pp3 + length as usize).min(payload.len());
                        } else {
                            p = payload.len();
                        }
                    }
                    5 => {
                        p = (p + 4).min(payload.len());
                    }
                    _ => {
                        p = payload.len();
                    }
                }
            }
        }

        key.map(|k| (k, value.unwrap_or_default()))
    }

    fn read_varint(&mut self) -> anyhow::Result<Option<u64>> {
        let mut result = 0u64;
        let mut shift = 0u32;
        loop {
            let mut buf = [0u8; 1];
            match self.reader.read_exact(&mut buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(e) => return Err(e.into()),
            }
            let b = buf[0] as u64;
            result |= (b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(Some(result));
            }
            shift += 7;
            if shift >= 64 {
                return Ok(None);
            }
        }
    }

    fn read_bytes(&mut self, length: usize) -> anyhow::Result<Option<Vec<u8>>> {
        let mut buf = vec![0u8; length];
        match self.reader.read_exact(&mut buf) {
            Ok(()) => Ok(Some(buf)),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn skip_field(&mut self, wire_type: u32) -> anyhow::Result<()> {
        match wire_type {
            0 => {
                let _ = self.read_varint()?;
            }
            1 => {
                let mut buf = [0u8; 8];
                let _ = self.reader.read_exact(&mut buf);
            }
            2 => {
                if let Some(length) = self.read_varint()? {
                    let mut buf = vec![0u8; length as usize];
                    let _ = self.reader.read_exact(&mut buf);
                }
            }
            5 => {
                let mut buf = [0u8; 4];
                let _ = self.reader.read_exact(&mut buf);
            }
            _ => {}
        }
        Ok(())
    }
}
