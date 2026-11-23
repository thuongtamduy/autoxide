//! Fake AI models for tests (D-109): no network calls, predefined results, identical on every run.
//! Modules that use AI (`content`, `media`…) test their own logic against these fakes; the real adapter of the
//! `ai` module is tested separately against a fake HTTP server.

use std::sync::Mutex;

use autoxide_kernel::ai::{
    Capabilities, EmbedKind, Embedder, Generation, Prompt, TextModel, Usage, schema,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use serde_json::Value;

/// Answer function of [`FakeTextModel`].
type Answer = Box<dyn Fn(&Prompt) -> Result<Value, Error> + Send + Sync>;

/// Fake text model: answers with `answer` and records every prompt received. A result that violates the schema
/// yields the `ai.output_invalid` error, like the real `ai` module.
pub struct FakeTextModel {
    answer: Answer,
    images: bool,
    enabled: bool,
    /// Every prompt received, in order.
    pub prompts: Mutex<Vec<Prompt>>,
}

impl FakeTextModel {
    pub fn new(answer: impl Fn(&Prompt) -> Result<Value, Error> + Send + Sync + 'static) -> Self {
        FakeTextModel {
            answer: Box::new(answer),
            images: true,
            enabled: true,
            prompts: Mutex::new(Vec::new()),
        }
    }

    /// As if AI were not enabled: every call returns `ai.disabled`.
    pub fn disabled() -> Self {
        let mut model = FakeTextModel::new(|_| {
            Err(Error::unavailable("ai.disabled", "AI features are off on this server."))
        });
        model.enabled = false;
        model
    }

    /// As if the model could not read images.
    pub fn without_images(mut self) -> Self {
        self.images = false;
        self
    }

    /// Number of prompts received.
    pub fn calls(&self) -> usize {
        self.prompts.lock().map(|prompts| prompts.len()).unwrap_or(0)
    }
}

#[async_trait::async_trait]
impl TextModel for FakeTextModel {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities =
            Capabilities::enabled("fake".to_owned(), "fake-model".to_owned()).images(self.images);
        // Bản đã tắt vẫn khai tên giả, như trước.
        capabilities.enabled = self.enabled;
        capabilities
    }

    fn ready(&self) -> Result<(), Error> {
        if self.enabled {
            Ok(())
        } else {
            Err(Error::unavailable("ai.disabled", "AI features are off on this server."))
        }
    }

    async fn generate(&self, _tenant: &TenantContext, prompt: Prompt) -> Result<Generation, Error> {
        if let Ok(mut prompts) = self.prompts.lock() {
            prompts.push(prompt.clone());
        }
        let output = (self.answer)(&prompt)?;
        if !schema::problems(&prompt.output_schema, &output).is_empty() {
            return Err(Error::unavailable(
                "ai.output_invalid",
                "The fake answer has the wrong shape.",
            ));
        }
        Ok(Generation::new(output, Usage::new(1, 1), "fake-model".to_owned()))
    }
}

/// Fake embedding: each word (diacritics stripped, lowercased) adds 1 to a dimension chosen by the word's hash,
/// then the vector is normalized. Two passages sharing many words end up close together — enough to test
/// semantic search without a real model.
pub struct FakeEmbedder {
    dimensions: usize,
    enabled: bool,
    model: String,
    /// Number of `embed` calls (so tests can check that unchanged passages are not recomputed).
    pub calls: Mutex<usize>,
}

impl FakeEmbedder {
    pub fn new(dimensions: usize) -> Self {
        FakeEmbedder {
            dimensions,
            enabled: true,
            model: "fake-embedding".to_owned(),
            calls: Mutex::new(0),
        }
    }

    /// As if embedding were not enabled: `enabled()` is `false` and `embed` returns `ai.embedding_disabled`.
    pub fn disabled() -> Self {
        let mut embedder = FakeEmbedder::new(0);
        embedder.enabled = false;
        embedder
    }

    /// As if switched to a different model (to test re-indexing).
    pub fn with_model(mut self, model: &str) -> Self {
        model.clone_into(&mut self.model);
        self
    }

    pub fn calls(&self) -> usize {
        self.calls.lock().map(|calls| *calls).unwrap_or(0)
    }

    pub fn vector(&self, text: &str) -> Vec<f32> {
        let mut vector = vec![0.0f32; self.dimensions];
        let folded = autoxide_kernel::search::fold(text);
        for word in folded.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()) {
            // FNV-1a: băm đơn giản, như nhau trên mọi máy.
            let hash = word.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            });
            let index = usize::try_from(hash % self.dimensions as u64).unwrap_or(0);
            if let Some(slot) = vector.get_mut(index) {
                *slot += 1.0;
            }
        }
        autoxide_kernel::ai::vector::normalize(&mut vector);
        vector
    }
}

#[async_trait::async_trait]
impl Embedder for FakeEmbedder {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn ready(&self) -> Result<(), Error> {
        if self.enabled {
            Ok(())
        } else {
            Err(Error::unavailable("ai.embedding_disabled", "Embeddings are off."))
        }
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed(
        &self,
        _tenant: &TenantContext,
        _purpose: &'static str,
        texts: &[String],
        _kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, Error> {
        if !self.enabled {
            return Err(Error::unavailable("ai.embedding_disabled", "Embeddings are off."));
        }
        if let Ok(mut calls) = self.calls.lock() {
            *calls += 1;
        }
        Ok(texts.iter().map(|text| self.vector(text)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doan_chung_nhieu_tu_thi_gan_nhau() {
        let embedder = FakeEmbedder::new(64);
        let dalat = embedder.vector("Du lịch Đà Lạt mùa hoa");
        let similar = embedder.vector("da lat mua hoa dep");
        let other = embedder.vector("Rust ownership and lifetimes");
        let dot = autoxide_kernel::ai::vector::dot;
        assert!(dot(&dalat, &similar) > dot(&dalat, &other));
        assert_eq!(dalat, embedder.vector("Du lịch Đà Lạt mùa hoa"));
    }
}
