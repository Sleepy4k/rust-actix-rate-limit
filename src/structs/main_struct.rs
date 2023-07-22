use serde_json::json;
use std::time::Duration;
use failure::{self, Fail};
use actix_web::HttpResponse;
use actix_web::body::BoxBody;
use actix_web::http::StatusCode;
use actix_web::error::ResponseError;

#[derive(Debug, Fail)]
pub enum ARError {
  #[fail(display = "store not connected")]
  NotConnected,
  #[fail(display = "store disconnected")]
  Disconnected,
  #[fail(display = "read/write operatiion failed: {}", _0)]
  ReadWriteError(String),
  #[fail(display = "unknown error: {}", _0)]
  UnknownError(std::io::Error),
  #[fail(display = "client identification failed")]
  IdentificationError,
  #[fail(display = "rate limit failed")]
  RateLimitError {
    max_requests: usize,
    c: usize,
    reset: Duration,
  },
}

impl ResponseError for ARError {
  fn status_code(&self) -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
  }

  fn error_response(&self) -> HttpResponse<BoxBody> {
    match *self {
      Self::RateLimitError {
        max_requests,
        c,
        reset,
      } => HttpResponse::build(StatusCode::TOO_MANY_REQUESTS)
        .content_type("application/json")
        .append_header(("x-ratelimit-limit", max_requests.to_string()))
        .append_header(("x-ratelimit-remaining", c.to_string()))
        .append_header(("x-ratelimit-reset", reset.as_secs().to_string()))
        .json(json!({
          "status": "failed",
          "message": "too many request",
          "data": [],
        })),
      _ => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR)
        .content_type("application/json")
        .json(json!({
          "status": "error",
          "message": "something went wrong",
          "data": [],
        })),
    }
  }
}