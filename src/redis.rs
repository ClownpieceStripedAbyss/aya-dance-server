use std::sync::Arc;

use bb8::Pool;
use bb8_redis::RedisConnectionManager;

use crate::Result;

#[derive(Debug)]
pub struct RedisServiceImpl {
  pub pool: RedisPool,
}

pub type RedisService = Arc<RedisServiceImpl>;
pub type RedisPool = Pool<RedisConnectionManager>;

impl RedisServiceImpl {
  pub async fn new(redis_url: String) -> Result<RedisService> {
    let manager = RedisConnectionManager::new(redis_url.as_str())?;
    let pool = Pool::builder().build(manager).await?;
    Ok(Arc::new(Self { pool }))
  }
}
