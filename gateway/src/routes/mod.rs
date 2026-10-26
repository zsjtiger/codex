use axum::Router;

use crate::state::AppState;

mod artifacts;
mod attachments;
mod auth;
mod connectors;
mod environments;
mod members;
mod projects;
mod scheduled;
mod search;
mod skills;
mod threads;
pub mod usage;
mod workspaces;

/// All REST routes, mounted under `/api/v1`.
pub fn router() -> Router<AppState> {
    let api = Router::new()
        .merge(auth::routes())
        .merge(workspaces::routes())
        .merge(members::routes())
        .merge(environments::routes())
        .merge(threads::routes())
        .merge(attachments::routes())
        .merge(usage::routes())
        .merge(projects::routes())
        .merge(skills::routes())
        .merge(artifacts::routes())
        .merge(search::routes())
        .merge(scheduled::routes())
        .merge(connectors::routes())
        .merge(crate::audit::routes());
    Router::new().nest("/api/v1", api)
}
