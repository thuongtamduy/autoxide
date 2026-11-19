//! Sổ [`FactSource`] (điểm cắm `SupportFacts`, D-230): module đăng ký sau gắn nguồn câu trả lời từ
//! dữ liệu chính xác lúc `start`; trả lời tự động (bước 53 mục 6) tra ý định ở đây.

use std::sync::{Arc, RwLock};

use autoxide_kernel::error::Error;

use crate::api::{FactSource, SupportFacts};

/// Clone rẻ: mọi bản clone thấy cùng một sổ.
#[derive(Clone, Default)]
pub(crate) struct FactRegistry(Arc<RwLock<Vec<Arc<dyn FactSource>>>>);

impl FactRegistry {
    /// Mọi ý định của mọi nguồn đã gắn.
    pub(crate) fn intents(&self) -> Vec<crate::api::FactIntent> {
        self.0
            .read()
            .map(|sources| sources.iter().flat_map(|source| source.intents()).collect())
            .unwrap_or_default()
    }

    /// Nguồn trả lời ý định `intent`, nếu có.
    pub(crate) fn source_of(&self, intent: &str) -> Option<Arc<dyn FactSource>> {
        let sources = self.0.read().ok()?;
        sources
            .iter()
            .find(|source| source.intents().iter().any(|known| known.intent == intent))
            .cloned()
    }
}

impl SupportFacts for FactRegistry {
    fn attach(&self, source: Arc<dyn FactSource>) -> Result<(), Error> {
        let mut sources =
            self.0.write().map_err(|_| Error::internal("support fact registry is poisoned"))?;
        for intent in source.intents() {
            let Some((module, name)) = intent.intent.split_once('.') else {
                return Err(Error::internal(format!(
                    "fact intent `{}` must be `<module>.<intent>`",
                    intent.intent
                )));
            };
            if module.is_empty() || name.is_empty() {
                return Err(Error::internal(format!(
                    "fact intent `{}` is malformed",
                    intent.intent
                )));
            }
            let taken = sources
                .iter()
                .any(|known| known.intents().iter().any(|other| other.intent == intent.intent));
            if taken {
                return Err(Error::internal(format!(
                    "fact intent `{}` is attached twice",
                    intent.intent
                )));
            }
        }
        sources.push(source);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{FactIntent, Requester};
    use autoxide_kernel::tenancy::TenantContext;

    struct Tracking;

    #[async_trait::async_trait]
    impl FactSource for Tracking {
        fn intents(&self) -> Vec<FactIntent> {
            vec![FactIntent {
                intent: "shipping.tracking",
                description: "Where is my parcel?",
                params: serde_json::json!({"type": "object"}),
            }]
        }

        async fn answer(
            &self,
            _: &TenantContext,
            _: &Requester,
            _: &str,
            _: &serde_json::Value,
            _: &str,
        ) -> Result<Option<String>, Error> {
            Ok(None)
        }
    }

    #[test]
    fn gan_nguon_mot_lan_cho_moi_y_dinh() {
        let registry = FactRegistry::default();
        registry.attach(Arc::new(Tracking)).unwrap();
        assert!(registry.attach(Arc::new(Tracking)).is_err(), "duplicate intent");
        assert!(registry.source_of("shipping.tracking").is_some());
        assert!(registry.source_of("orders.status").is_none());
    }
}
