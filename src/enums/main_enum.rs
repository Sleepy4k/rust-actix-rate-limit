use actix::dev::*;
use std::pin::Pin;
use std::marker::Send;
use std::time::Duration;
use std::future::Future;

use crate::structs::main_struct::ARError;

pub enum ActorMessage {
  Get(String),
  Set {
    key: String,
    value: usize,
    expiry: Duration,
  },
  Update { key: String, value: usize },
  Expire(String),
  Remove(String),
}

impl Message for ActorMessage {
  type Result = ActorResponse;
}

pub type Output<T> = Pin<Box<dyn Future<Output = Result<T, ARError>> + Send>>;

pub enum ActorResponse {
  Get(Output<Option<usize>>),
  Set(Output<()>),
  Update(Output<usize>),
  Expire(Output<Duration>),
  Remove(Output<usize>),
}

impl<A, M> MessageResponse<A, M> for ActorResponse
where
  A: Actor,
  M: actix::Message<Result = ActorResponse>,
{
  fn handle(self, _: &mut A::Context, tx: Option<OneshotSender<M::Result>>) {
    if let Some(tx) = tx {
      tx.send(self).ok();
    }
  }
}