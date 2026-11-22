//! Tells cargo what the crate's compile-time SQL checks depend on.
//!
//! `sqlx::migrate!` embeds the migrations, and the `query!` macros read either a live database
//! or the `.sqlx/` offline cache depending on `SQLX_OFFLINE`. Without these lines cargo reuses
//! an earlier build when only one of them changes, so an offline build could pass on results
//! that were checked against a database.

fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
    println!("cargo:rerun-if-changed=../../.sqlx");
    println!("cargo:rerun-if-env-changed=SQLX_OFFLINE");
    println!("cargo:rerun-if-env-changed=DATABASE_URL");
}
