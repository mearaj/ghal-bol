use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Internal(String),
}

impl ServerError {
    fn status(&self) -> StatusCode {
        match self {
            ServerError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ServerError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            ServerError::NotFound(_) => StatusCode::NOT_FOUND,
            ServerError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "error": self.to_string() }));
        (self.status(), body).into_response()
    }
}

pub type ApiResult<T> = Result<T, ServerError>;
