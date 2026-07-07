use anyhow::Result;
use std::sync::Arc;

#[derive(Clone)]
pub struct RedisCache {
    // TODO: Add redis::aio::ConnectionManager
    _placeholder: (),
}

impl RedisCache {
    pub fn new() -> Self {
        Self { _placeholder: () }
    }

    pub async fn get(&self, _key: &str) -> Result<Option<String>> {
        // TODO: Implement Redis GET
        Ok(None)
    }

    pub async fn set(&self, _key: &str, _value: &str) -> Result<()> {
        // TODO: Implement Redis SET
        Ok(())
    }
}

pub type SharedCache = Arc<RedisCache>;
