use actix_web::HttpResponse;
use actix_web::ResponseError;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Display;
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorReason {
    ErrInvalidArgument,
    ErrInvalidState,
    ErrNotFound,
    ErrRateLimit,
    ErrExternal,
    ErrInternal,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorMessage {
    pub code: i32,
    pub reason: ErrorReason,
    pub message: String,
}
impl Display for ErrorMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(&self).unwrap())?;
        Ok(())
    }
}
impl ResponseError for ErrorMessage {
    fn status_code(&self) -> reqwest::StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }
    fn error_response(&self) -> HttpResponse<actix_web::body::BoxBody> {
        HttpResponse::build(self.status_code()).json(&self)
    }
}
impl<T: std::error::Error> From<T> for ErrorMessage {
    fn from(_err: T) -> Self {
        ErrorMessage {
            code: 6,
            reason: ErrorReason::ErrInternal,
            message: "std::error::Error".to_string(),
        }
    }
}
impl ErrorMessage {
    pub fn r2error(_err: r2d2::Error) -> ErrorMessage {
        ErrorMessage {
            code: 5,
            reason: ErrorReason::ErrExternal,
            message: "rusqlite_error".to_string(),
        }
    }
    pub fn rusqlite_error(_err: rusqlite::Error) -> ErrorMessage {
        ErrorMessage {
            code: 5,
            reason: ErrorReason::ErrExternal,
            message: "rusqlite_error".to_string(),
        }
    }
}
