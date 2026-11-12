use anyhow::Context;
use sherpa_onnx::{
    SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig,
};
use std::sync::Arc;
use tracing::{info, warn};

pub struct SpeakerVerifier {
    extractor: Arc<SpeakerEmbeddingExtractor>,
    reference_embeddings: Vec<Vec<f32>>,
    threshold: f32,
}

impl SpeakerVerifier {
    /// Load the speaker embedding model from disk. Call this once when the ASR
    /// model is loaded; the resulting extractor is shared across sessions.
    pub fn load_extractor(model_path: &str) -> Option<SpeakerEmbeddingExtractor> {
        info!(
            "Loading speaker embedding model: {}",
            crate::path_display::for_log_path(std::path::Path::new(model_path))
        );
        let config = SpeakerEmbeddingExtractorConfig {
            model: Some(model_path.to_string()),
            num_threads: 1,
            debug: false,
            provider: Some("cpu".to_string()),
        };
        SpeakerEmbeddingExtractor::create(&config)
            .context("SpeakerEmbeddingExtractor creation failed")
            .map_err(|e| warn!("{e:#}"))
            .ok()
    }

    /// Build a per-session verifier around a shared [SpeakerEmbeddingExtractor],
    /// storing this session's reference embeddings + threshold. No model is
    /// loaded here.
    pub fn from_shared(
        extractor: Arc<SpeakerEmbeddingExtractor>,
        reference_embeddings: Vec<Vec<f32>>,
        threshold: f32,
    ) -> Self {
        let adjusted_threshold = if threshold <= 0.0 { 0.45 } else { threshold };
        Self {
            extractor,
            reference_embeddings,
            threshold: adjusted_threshold,
        }
    }

    /// Compute the embedding for `samples` and compare it against the
    /// reference embeddings using cosine similarity.  Returns `true` when
    /// the best similarity score exceeds `threshold`.
    pub fn verify(&self, samples: &[f32], sample_rate: i32) -> bool {
        if self.reference_embeddings.is_empty() {
            return true;
        }
        if samples.is_empty() {
            return false;
        }

        // Create a stream, feed the audio, and extract the embedding.
        let stream = match self.extractor.create_stream() {
            Some(s) => s,
            None => {
                warn!("speaker: failed to create OnlineStream");
                return true;
            }
        };
        stream.accept_waveform(sample_rate, samples);
        stream.input_finished();

        let emb = self.extractor.compute(&stream);
        let Some(emb) = emb else {
            warn!("speaker: embedding extraction returned None");
            return true;
        };

        // Cosine similarity against the average reference embedding.
        let avg = average_embedding(&self.reference_embeddings);
        let score = cosine_similarity(&emb, &avg);
        let accepted = score >= self.threshold;

        info!(
            "speaker verification: score={:.4} threshold={:.3} accepted={accepted}",
            score, self.threshold,
        );
        accepted
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += (*x as f64) * (*y as f64);
        na += (*x as f64) * (*x as f64);
        nb += (*y as f64) * (*y as f64);
    }
    let denom = (na.sqrt() * nb.sqrt()) as f64;
    if denom < 1e-12 {
        0.0
    } else {
        (dot / denom) as f32
    }
}

fn average_embedding(embs: &[Vec<f32>]) -> Vec<f32> {
    if embs.is_empty() {
        return Vec::new();
    }
    let dim = embs[0].len();
    let mut avg = vec![0.0f32; dim];
    for emb in embs {
        for (a, v) in avg.iter_mut().zip(emb.iter()) {
            *a += *v;
        }
    }
    let n = embs.len() as f32;
    for v in &mut avg {
        *v /= n;
    }
    avg
}
