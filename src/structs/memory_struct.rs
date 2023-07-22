use log::*;
use std::sync::Arc;
use dashmap::DashMap;
use actix::prelude::*;
use futures::future::{self};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{
  structs::main_struct::ARError,
  enums::main_enum::{
    ActorMessage,
    ActorResponse
  }
};

#[derive(Clone)]
pub struct MemoryStore {
  inner: Arc<DashMap<String, (usize, Duration)>>,
}

impl MemoryStore {
  pub fn new() -> Self {
    debug!("Creating new MemoryStore");
    MemoryStore {
      inner: Arc::new(DashMap::<String, (usize, Duration)>::new()),
    }
  }

  pub fn with_capacity(capacity: usize) -> Self {
    debug!("Creating new MemoryStore");
    MemoryStore {
      inner: Arc::new(DashMap::<String, (usize, Duration)>::with_capacity(
        capacity,
      )),
    }
  }
}

pub struct MemoryStoreActor {
  inner: Arc<DashMap<String, (usize, Duration)>>,
}

impl From<MemoryStore> for MemoryStoreActor {
  fn from(store: MemoryStore) -> Self {
    MemoryStoreActor { inner: store.inner }
  }
}

impl MemoryStoreActor {
  pub fn start(self) -> Addr<Self> {
    debug!("Started memory store");
    Supervisor::start(|_| self)
  }
}

impl Actor for MemoryStoreActor {
  type Context = Context<Self>;
}

impl Supervised for MemoryStoreActor {
  fn restarting(&mut self, _: &mut Self::Context) {
    debug!("Restarting memory store");
  }
}

impl Handler<ActorMessage> for MemoryStoreActor {
  type Result = ActorResponse;

  fn handle(&mut self, msg: ActorMessage, ctx: &mut Self::Context) -> Self::Result {
    match msg {
      ActorMessage::Set { key, value, expiry } => {
        debug!("Inserting key {} with expiry {}", &key, &expiry.as_secs());
        let future_key = String::from(&key);
        let now = SystemTime::now();
        let now = now.duration_since(UNIX_EPOCH).unwrap();

        self.inner.insert(key, (value, now + expiry));
        ctx.notify_later(ActorMessage::Remove(future_key), expiry);

        ActorResponse::Set(Box::pin(future::ready(Ok(()))))
      }
      ActorMessage::Update { key, value } => match self.inner.get_mut(&key) {
        Some(mut c) => {
          let val_mut: &mut (usize, Duration) = c.value_mut();

          if val_mut.0 > value {
            val_mut.0 -= value;
          } else {
            val_mut.0 = 0;
          }

          let new_val = val_mut.0;

          ActorResponse::Update(Box::pin(future::ready(Ok(new_val))))
        }
        None => {
          return ActorResponse::Update(Box::pin(future::ready(Err(
            ARError::ReadWriteError("memory store: read failed!".to_string()),
          ))))
        }
      },
      ActorMessage::Get(key) => {
        if self.inner.contains_key(&key) {
          let val = match self.inner.get(&key) {
            Some(c) => c,
            None => {
              return ActorResponse::Get(Box::pin(future::ready(Err(
                ARError::ReadWriteError("memory store: read failed!".to_string()),
              ))))
            }
          };
          
          let val = val.value().0;

          ActorResponse::Get(Box::pin(future::ready(Ok(Some(val)))))
        } else {
          ActorResponse::Get(Box::pin(future::ready(Ok(None))))
        }
      }
      ActorMessage::Expire(key) => {
        let c = match self.inner.get(&key) {
          Some(d) => d,
          None => {
            return ActorResponse::Expire(Box::pin(future::ready(Err(
              ARError::ReadWriteError("memory store: read failed!".to_string()),
            ))))
          }
        };

        let dur = c.value().1;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let res = dur.checked_sub(now).unwrap_or_else(|| Duration::new(0, 0));

        ActorResponse::Expire(Box::pin(future::ready(Ok(res))))
      }
      ActorMessage::Remove(key) => {
        debug!("Removing key: {}", &key);
        let val = match self.inner.remove::<String>(&key) {
          Some(c) => c,
          None => {
            return ActorResponse::Remove(Box::pin(future::ready(Err(
              ARError::ReadWriteError("memory store: remove failed!".to_string()),
            ))))
          }
        };

        let val = val.1;

        ActorResponse::Remove(Box::pin(future::ready(Ok(val.0))))
      }
    }
  }
}