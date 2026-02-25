use axum::Json;

pub async fn get_schema() -> Json<serde_json::Value> {
    let json_str = crate::schema::get_schema_json();
    let val: serde_json::Value = serde_json::from_str(&json_str).unwrap_or_default();
    Json(val)
}
