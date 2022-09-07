use std::net::SocketAddr;

use axum::{
    extract,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json,
    Router,
};
use serde::{Deserialize, Serialize};

pub async fn run_json_rpc() {
    let app = Router::new()
        .route("/", get(root))
        .route("/users", post(create_user))
        .route("/users/:id", get(get_user));

    let addr = SocketAddr::from(([127, 0, 0, 1], 13000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr).serve(app.into_make_service()).await.unwrap();
}

async fn root() -> &'static str {
    "Hello, World!"
}

async fn get_user(extract::Path(id): extract::Path<u64>) -> impl IntoResponse {
    let user = User {
        id,
        username: "John".to_string(),
    };

    (StatusCode::OK, Json(user))
}

async fn create_user(
    // this argument tells axum to parse the request body
    // as JSON into a `CreateUser` type
    Json(payload): Json<CreateUser>,
) -> impl IntoResponse {
    // insert your application logic here
    let user = User {
        id: 1337,
        username: payload.username,
    };

    // this will be converted into a JSON response
    // with a status code of `201 Created`
    (StatusCode::CREATED, Json(user))
}

// the input to our `create_user` handler
#[derive(Deserialize)]
struct CreateUser {
    username: String,
}

// the output to our `create_user` handler
#[derive(Serialize)]
struct User {
    id: u64,
    username: String,
}
