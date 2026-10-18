use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
#[derive(Debug)]
pub struct AppError(pub StatusCode, pub &'static str);
pub type Result<T> = std::result::Result<T, AppError>;
impl AppError {
    pub fn bad(msg: &'static str) -> Self {
        Self(StatusCode::BAD_REQUEST, msg)
    }
    pub fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "Sessão inválida ou expirada")
    }
    pub fn forbidden() -> Self {
        Self(StatusCode::FORBIDDEN, "Acesso negado")
    }
    pub fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, "Não encontrado")
    }
    pub fn internal() -> Self {
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Erro interno do servidor",
        )
    }
}
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error":self.1}))).into_response()
    }
}
impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref d) = e
            && d.is_unique_violation()
        {
            return Self(StatusCode::CONFLICT, "Este nome já está em uso");
        }
        tracing::error!(error=%e,"database operation failed");
        Self::internal()
    }
}
