use actix::dev::*;
use futures::task::Poll;
use futures::{Future, future::{ok, Ready}};
use actix_web::{
  http::header::{HeaderName, HeaderValue},
  error::{Error as AWError, ErrorInternalServerError},
  dev::{Service, ServiceRequest, ServiceResponse, Transform}
};
use std::{
  rc::Rc,
  ops::Fn,
  pin::Pin,
  cell::RefCell,
  time::Duration
};

use crate::{
  structs::main_struct::ARError,
  enums::main_enum::{
    ActorMessage,
    ActorResponse
  },
};

pub struct RateLimiter<T>
where
  T: Handler<ActorMessage> + Send + Sync + 'static,
  T::Context: ToEnvelope<T, ActorMessage>,
{
  interval: Duration,
  max_requests: usize,
  store: Addr<T>,
  #[allow(clippy::type_complexity)]
  identifier: Rc<Box<dyn Fn(&ServiceRequest) -> Result<String, AWError>>>,
}

impl<T> RateLimiter<T>
where
  T: Handler<ActorMessage> + Send + Sync + 'static,
  <T as Actor>::Context: ToEnvelope<T, ActorMessage>,
{
  pub fn new(store: Addr<T>) -> Self {
    let identifier = |req: &ServiceRequest| {
      let connection_info = req.connection_info();
      let ip = connection_info
        .peer_addr()
        .ok_or(ARError::IdentificationError)?;

      Ok(String::from(ip))
    };
    
    RateLimiter {
      interval: Duration::from_secs(0),
      max_requests: 0,
      store,
      identifier: Rc::new(Box::new(identifier)),
    }
  }

  pub fn with_interval(mut self, interval: Duration) -> Self {
    self.interval = interval;
    self
  }

  pub fn with_max_requests(mut self, max_requests: usize) -> Self {
    self.max_requests = max_requests;
    self
  }

  pub fn with_identifier<F: Fn(&ServiceRequest) -> Result<String, AWError> + 'static>(
    mut self,
    identifier: F,
  ) -> Self {
    self.identifier = Rc::new(Box::new(identifier));
    self
  }
}

impl<T, S, B> Transform<S, ServiceRequest> for RateLimiter<T>
where
  T: Handler<ActorMessage> + Send + Sync + 'static,
  T::Context: ToEnvelope<T, ActorMessage>,
  S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = AWError> + 'static,
  S::Future: 'static,
  B: 'static,
{
  type Response = ServiceResponse<B>;
  type Error = S::Error;
  type InitError = ();
  type Transform = RateLimitMiddleware<S, T>;
  type Future = Ready<Result<Self::Transform, Self::InitError>>;

  fn new_transform(&self, service: S) -> Self::Future {
    ok(RateLimitMiddleware {
      service: Rc::new(RefCell::new(service)),
      store: self.store.clone(),
      max_requests: self.max_requests,
      interval: self.interval.as_secs(),
      identifier: self.identifier.clone(),
    })
  }
}


pub struct RateLimitMiddleware<S, T>
where
  S: 'static,
  T: Handler<ActorMessage> + 'static,
{
  service: Rc<RefCell<S>>,
  store: Addr<T>,
  max_requests: usize,
  interval: u64,
  #[allow(clippy::type_complexity)]
  identifier: Rc<Box<dyn Fn(&ServiceRequest) -> Result<String, AWError> + 'static>>,
}

impl<T, S, B> Service<ServiceRequest> for RateLimitMiddleware<S, T>
where
  T: Handler<ActorMessage> + 'static,
  S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = AWError> + 'static,
  S::Future: 'static,
  B: 'static,
  T::Context: ToEnvelope<T, ActorMessage>,
{
  type Response = ServiceResponse<B>;
  type Error = S::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

  fn poll_ready(
    self: &RateLimitMiddleware<S, T>,
    cx: &mut core::task::Context<'_>,
  ) -> Poll<Result<(), Self::Error>> {
    self.service.borrow_mut().poll_ready(cx)
  }

  fn call(&self, req: ServiceRequest) -> Self::Future {
    let store = self.store.clone();
    let srv = self.service.clone();
    let max_requests = self.max_requests;
    let interval = Duration::from_secs(self.interval);
    let identifier = self.identifier.clone();

    Box::pin(async move {
      let identifier: String = (identifier)(&req).map_err(ErrorInternalServerError)?;
      let remaining: ActorResponse = store
        .send(ActorMessage::Get(String::from(&identifier)))
        .await
        .map_err(ErrorInternalServerError)?;

      match remaining {
        ActorResponse::Get(opt) => {
          let opt = opt.await?;

          if let Some(c) = opt {
            let expiry = store
              .send(ActorMessage::Expire(String::from(&identifier)))
              .await
              .map_err(ErrorInternalServerError)?;

            let reset: Duration = match expiry {
              ActorResponse::Expire(dur) => dur.await?,
              _ => unreachable!(),
            };

            if c == 0 {
              Err(ARError::RateLimitError {
                max_requests,
                c,
                reset,
              }

              .into())
            } else {
              let res: ActorResponse = store
                .send(ActorMessage::Update {
                    key: identifier,
                    value: 1,
                })
                .await
                .map_err(ErrorInternalServerError)?;

              let updated_value: usize = match res {
                ActorResponse::Update(c) => {
                  c.await.map_err(ErrorInternalServerError)?
                }
                _ => unreachable!(),
              };

              let fut = srv.call(req);
              let mut res = fut.await.map_err(ErrorInternalServerError)?;
              let headers = res.headers_mut();

              headers.insert(
                HeaderName::from_static("x-ratelimit-limit"),
                HeaderValue::from_str(max_requests.to_string().as_str()).unwrap(),
              );

              headers.insert(
                HeaderName::from_static("x-ratelimit-remaining"),
                HeaderValue::from_str(updated_value.to_string().as_str()).unwrap(),
              );

              headers.insert(
                HeaderName::from_static("x-ratelimit-reset"),
                HeaderValue::from_str(reset.as_secs().to_string().as_str()).unwrap(),
              );

              Ok(res)
            }
          } else {
            let current_value = max_requests - 1;
            let res = store
              .send(ActorMessage::Set {
                key: String::from(&identifier),
                value: current_value,
                expiry: interval,
              })
              .await
              .map_err(ErrorInternalServerError)?;

            match res {
              ActorResponse::Set(c) => {
                c.await.map_err(ErrorInternalServerError)?
              }
              _ => unreachable!(),
            }

            let fut = srv.call(req);
            let mut res = fut.await.map_err(ErrorInternalServerError)?;
            let headers = res.headers_mut();

            headers.insert(
              HeaderName::from_static("x-ratelimit-limit"),
              HeaderValue::from_str(max_requests.to_string().as_str()).unwrap(),
            );

            headers.insert(
              HeaderName::from_static("x-ratelimit-remaining"),
              HeaderValue::from_str(current_value.to_string().as_str()).unwrap(),
            );

            headers.insert(
              HeaderName::from_static("x-ratelimit-reset"),
              HeaderValue::from_str(interval.as_secs().to_string().as_str()).unwrap(),
            );
            
            Ok(res)
          }
        }
        _ => {
          unreachable!();
        }
      }
    })
  }
}