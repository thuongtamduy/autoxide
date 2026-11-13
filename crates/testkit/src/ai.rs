//! Mô hình AI giả cho test (D-109): không gọi mạng, kết quả định sẵn, như nhau mỗi lần chạy.
//! Module dùng AI (`content`, `media`…) test logic của mình với các bản giả này; adapter thật của
//! module `ai` được test riêng với máy chủ HTTP giả.

use std::sync::Mutex;

use autoxide_kernel::ai::{
    Capabilities, EmbedKind, Embedder, Generation, Prompt, TextModel, Usage, schema,
};
use autoxide_kernel::error::Error;
use serde_json::Value;

/// Hàm trả lời của [`FakeTextModel`].
type Answer = Box<dyn Fn(&Prompt) -> Result<Value, Error> + Send + Sync>;

/// Mô hình sinh chữ giả: trả lời bằng `answer`, ghi lại mọi prompt đã nhận. Kết quả sai schema
/// thì trả lỗi `ai.output_invalid` như module `ai` thật.
pub struct FakeTextModel {
    answer: Answer,
    images: bool,
    enabled: bool,
    /// Mọi prompt đã nhận, theo thứ tự.
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

    /// Như khi AI chưa bật: mọi lời gọi trả `ai.disabled`.
    pub fn disabled() -> Self {
        let mut model = FakeTextModel::new(|_| {
            Err(Error::unavailable("ai.disabled", "AI features are off on this server."))
        });
        model.enabled = false;
        model
    }

    /// Như mô hình không đọc được ảnh.
    pub fn without_images(mut self) -> Self {
        self.images = false;
        self
    }

    /// Số prompt đã nhận.
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

    async fn generate(&self, prompt: Prompt) -> Result<Generation, Error> {
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

/// Embedding giả: mỗi từ (đã bỏ dấu, viết thường) cộng 1 vào một chiều chọn theo băm của từ,
/// rồi chuẩn hoá. Hai đoạn chung nhiều từ thì gần nhau — đủ để test tìm theo ý nghĩa mà không cần
/// mô hình thật.
pub struct FakeEmbedder {
    dimensions: usize,
    enabled: bool,
    model: String,
    /// Số lần gọi `embed` (để test không tính lại đoạn không đổi).
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

    /// Như khi embedding chưa bật: `enabled()` là `false`, `embed` trả `ai.embedding_disabled`.
    pub fn disabled() -> Self {
        let mut embedder = FakeEmbedder::new(0);
        embedder.enabled = false;
        embedder
    }

    /// Như đổi sang một mô hình khác (để test lập lại index).
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
